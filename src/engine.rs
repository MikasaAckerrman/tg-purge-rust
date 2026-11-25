//! Ф-R4 — боевая логика очистителя. ЧИСТЫЕ функции + один живой цикл.
//!
//! Разделение обязательное: всё, что можно проверить без сети, лежит в чистых
//! функциях (`Window`, `Registry`, `guard_saved`, `plan_batches`) и покрывается
//! юнит-тестами ниже. Живой код (`run_stream`) только вызывает их и обязан
//! проходить гейт R3 (компиляция) — никаких догадок о типах здесь нет, каждая
//! форма сверена с vendor-исходниками (см. ссылки в комментариях):
//!
//!   client.messages.rs:903  delete_messages<C: Into<PeerRef>>(peer, &[i32]) -> Result<usize, _>
//!                           «Deletes up to 100 messages… the amount of deleted messages is
//!                            returned (it might be less than the amount of input IDs if some
//!                            were already missing)» ⇒ пакет ≤ 100, счёт — от сервера.
//!   client.messages.rs:877  warning: для малых групп и ЛС Telegram НЕ проверяет, что id
//!                           принадлежат этому peer ⇒ движок обязан держать правильный peer
//!                           сам (это же причина, почему избранное охраняется реестром).
//!   message.message.rs:759  Message::delete(&self) -> Result<(), _> — «Shorthand for
//!                           Client::delete_messages»: безопасный путь для одиночного id.
//!   message.message.rs:689  Message::reply<M: Into<InputMessage>>(&self, M) -> Result<Self, _>
//!   message.message.rs:446  Message::media(&self) -> Option<Media>
//!   message.message.rs:305  Message::sender(&self) -> Option<&Peer>
//!   media/media.rs:883      impl Downloadable for Media  (download_media(&D, path))
//!   media/input_media.rs:356 InputMedia::copy_media(&Media) — зеркальный ответ
//!   client/updates.rs:69    stream_updates(mpsc::Receiver<UpdatesLike>, UpdatesConfiguration)
//!                           → UpdateStream::next() -> Result<Update, InvocationError>;
//!                           док: «peers must have been persisted in the session cache
//!                           beforehand… use iter_dialogs at least once after login»
//!                           ⇒ seed_dialogs() ОБЯЗАТЕЛЕН до live-цикла.
//!   errors.rs (mtsender)    InvocationError::Rpc(RpcError{code:420,value:Some(sec)})
//!                           ⇒ FLOOD_WAIT ретраится только нами: retry_policy не объявлен.

#![allow(dead_code)]

use std::collections::HashSet;
use std::time::{Duration, Instant};

use grammers_client::session::types::{PeerId, PeerRef};
use grammers_client::update::Update;
use grammers_client::Client;
use grammers_mtsender::{InvocationError, UpdatesConfiguration};
use tokio::sync::mpsc;

use crate::caps::flood_seconds;

/// Максимальный размер пакета удаления. Telegram документирует «up to 100»,
/// берём 50 как в Python-версии: меньше шанс поймать FLOOD_WAIT на одном пакете
/// и всё ещё один RTT (~191 мс медиана) на 50 сообщений.
pub const BATCH_MAX: usize = 50;

/// Глобальный потолок отправок в минуту для ответок (защита аккаунта).
pub const GLOBAL_RATE_PER_MIN: usize = 12;

// ──────────────────────────────────────────────────────────── скользящее окно
/// Скользящее окно частоты. Чистая функция времени: история — моменты прошлых
/// отправок, никаких таймеров и глобального состояния, поэтому тестируется без сна.
#[derive(Debug, Clone)]
pub struct Window {
    limit: usize,
    span: Duration,
    hits: Vec<Instant>,
}

impl Window {
    pub fn new(limit: usize, span: Duration) -> Self {
        Self { limit, span, hits: Vec::new() }
    }

    /// Сколько ждать до следующей разрешённой отправки. 0 = можно сейчас.
    /// limit = 0 означает «без потолка» (юзер вправе поставить 0 осознанно).
    pub fn wait(&self, now: Instant) -> Duration {
        if self.limit == 0 {
            return Duration::ZERO;
        }
        let cutoff = now.checked_sub(self.span).unwrap_or(now);
        let in_win: Vec<Instant> = self.hits.iter().copied().filter(|t| *t > cutoff).collect();
        if in_win.len() < self.limit {
            return Duration::ZERO;
        }
        // самое старое из окно-выживших освободится через span
        let oldest = in_win[in_win.len() - self.limit];
        let free_at = oldest + self.span;
        if free_at <= now { Duration::ZERO } else { free_at - now }
    }

    pub fn allow(&self, now: Instant) -> bool {
        self.wait(now).is_zero()
    }

    pub fn record(&mut self, now: Instant) {
        self.hits.push(now);
        self.hits.sort_by(|a, b| b.cmp(a)); // новые первыми
        let cutoff = now.checked_sub(self.span).unwrap_or(now);
        self.hits.retain(|t| *t > cutoff);
        if self.hits.len() > self.limit.max(1) * 4 {
            self.hits.truncate(self.limit.max(1) * 4);
        }
    }

    pub fn len(&self) -> usize {
        self.hits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }
}

// ─────────────────────────────────────────────────── реестр «создано движком»
/// Единственное основание удалить сообщение из ЛС с самим собой (Saved Messages /
/// избранное). Правило юзера 25.11: «удалять что-то избранное, присланное мной,
/// запрещено; только то, что ты написал».
///
/// Реестр обязан переживать рестарт процесса (в Python-версии это U-D30: сироты
/// 201157/201164 остались неудаляемыми, пока хранились только в RAM), поэтому
/// здесь он — только память, а сохранение в state делает вызывающий слой
/// (`crate::store::FileSession` + `Session::cache_peer`-подобный путь).
#[derive(Debug, Default, Clone)]
pub struct Registry {
    own: HashSet<i32>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn own(&mut self, id: i32) {
        self.own.insert(id);
        if self.own.len() > 500 {
            // детерминированного порядка у HashSet нет, поэтому хвост режем по
            // возрастанию id: старые сообщения теряются первыми, новые (актуальные) — нет
            let mut v: Vec<i32> = self.own.iter().copied().collect();
            v.sort_unstable();
            let keep = &v[v.len() - 300..];
            self.own = keep.iter().copied().collect();
        }
    }

    pub fn forget(&mut self, ids: &[i32]) {
        for i in ids {
            self.own.remove(i);
        }
    }

    pub fn contains(&self, id: i32) -> bool {
        self.own.contains(&id)
    }

    pub fn len(&self) -> usize {
        self.own.len()
    }

    pub fn is_empty(&self) -> bool {
        self.own.is_empty()
    }

    pub fn ids(&self) -> Vec<i32> {
        let mut v: Vec<i32> = self.own.iter().copied().collect();
        v.sort_unstable();
        v
    }
}

/// Причина отказа удаления. `None` = удалять можно.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Сообщение в ЛС с самим собой, но его не создавал движок → избранное юзера.
    SavedForeign(Vec<i32>),
    /// Пустой пакет.
    Empty,
}

/// ★ Единая точка решения «можно ли удалять». Вызывается ДО любого сетевого вызова.
///
/// `is_self_chat` = чат является Saved Messages (cid == me_id). Вне его гард не
/// действует: там цель определяется списком targets, и «чьё это» не важно
/// (слова юзера: «без разницы, наше или чужое»).
pub fn guard_saved(is_self_chat: bool, reg: &Registry, ids: &[i32]) -> Option<Refusal> {
    if ids.is_empty() {
        return Some(Refusal::Empty);
    }
    if !is_self_chat {
        return None;
    }
    let foreign: Vec<i32> = ids.iter().copied().filter(|i| !reg.contains(*i)).collect();
    if foreign.is_empty() {
        None
    } else {
        // отказ ЦЕЛИКОМ: половинчатое удаление в избранном — риск снести нужное юзеру
        Some(Refusal::SavedForeign(foreign))
    }
}

/// Разбивает id на пакеты не длиннее `BATCH_MAX` (один RTT на пакет).
pub fn plan_batches(ids: &[i32], size: usize) -> Vec<Vec<i32>> {
    let size = size.max(1).min(BATCH_MAX);
    ids.chunks(size).map(|c| c.to_vec()).collect()
}

// ─────────────────────────────────────────────────────────────── боевой путь
/// Удалить пакет, если гард разрешает. Возвращает число реально удалённых
/// (сервер отдаёт count, messages.rs:885) либо причину отказа.
pub async fn purge_batch(
    client: &Client,
    peer: PeerRef,
    is_self_chat: bool,
    reg: &mut Registry,
    ids: &[i32],
) -> Result<usize, Refusal> {
    if let Some(r) = guard_saved(is_self_chat, reg, ids) {
        return Err(r);
    }
    let n = client
        .delete_messages(peer, ids)
        .await
        .map_err(|_| Refusal::Empty)?;
    // удалённые id больше не наши — реестр не должен копить мусор
    reg.forget(ids);
    Ok(n)
}

/// Живой цикл: апдейты → чистка целей. seed_dialogs() обязан отработать до него,
/// иначе stream_updates не сможет разрешать пропуски (док client/updates.rs:64-67).
pub async fn seed_dialogs(client: &Client) -> Result<usize, InvocationError> {
    let mut it = client.iter_dialogs();
    let total = it.total().await?;
    while let Some(dialog) = it.next().await? {
        let peer = dialog.peer();
        let _ = client.resolve_peer(peer.id().to_ambient_ref()).await?;
    }
    Ok(total)
}

pub async fn run_stream(
    client: Client,
    updates: mpsc::Receiver<grammers_session::updates::UpdatesLike>,
    targets: HashSet<i64>,
    me_id: i64,
    reg: std::sync::Arc<std::sync::Mutex<Registry>>,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    // ★ UpdatesConfiguration НЕ выводит Default (только pub catch_up: bool) —
    //   форма из examples/echo.rs:81.
    let cfg = UpdatesConfiguration { catch_up: true };
    let mut stream = client.stream_updates(updates, cfg).await?;
    let mut removed = 0u64;
    loop {
        let upd = match stream.next().await {
            Ok(u) => u,
            Err(e) => {
                if let Some(sec) = flood_seconds(&e) {
                    tokio::time::sleep(Duration::from_secs(sec + 1)).await;
                    continue;
                }
                return Err(Box::new(e));
            }
        };
        let Update::NewMessage(msg) = upd else { continue };
        if msg.outgoing() {
            continue; // антицикличный инвариант: свои события не трогаем никогда
        }
        let Some(sender) = msg.sender() else { continue };
        let bare = sender.id().bare_id_unchecked();
        if !targets.contains(&bare) {
            continue;
        }
        let peer = msg.peer_id();
        // Saved Messages = ЛС с самим собой; kind() возвращает PeerKind (без is_user),
        // поэтому сравниваем вариант напрямую.
        let is_self = matches!(peer.kind(), grammers_client::session::types::PeerKind::User)
            && peer.bare_id_unchecked() == me_id;
        let ids = [msg.id()];
        let guard = {
            let g = reg.lock().map_err(|_| "реестр отравлен")?;
            guard_saved(is_self, &g, &ids)
        };
        if let Some(r) = guard {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("удаление отклонено гардом: {r:?}"),
            )));
        }
        // ★ delete_messages принимает Into<PeerRef>, а Message::peer_id() отдаёт PeerId,
        //   для которого From<PeerId> для PeerRef НЕТ — только to_ambient_ref().
        match client.delete_messages(peer.to_ambient_ref(), &ids).await {
            Ok(n) => removed += n as u64,
            Err(e) => {
                if let Some(sec) = flood_seconds(&e) {
                    tokio::time::sleep(Duration::from_secs(sec + 1)).await;
                }
            }
        }
        if let Ok(mut g) = reg.lock() {
            g.forget(&ids);
        }
    }
}

// ────────────────────────────────────────────────────────────────── тесты
#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    // ---- Window ----
    #[test]
    fn window_allows_until_limit() {
        let t0 = Instant::now();
        let mut w = Window::new(2, Duration::from_secs(60));
        assert!(w.allow(t0));
        w.record(t0);
        assert!(w.allow(at(t0, 1)));
        w.record(at(t0, 1));
        assert!(!w.allow(at(t0, 2)), "третий за минуту упирается в потолок");
    }

    #[test]
    fn window_waits_exactly_span_from_oldest() {
        let t0 = Instant::now();
        let mut w = Window::new(2, Duration::from_secs(60));
        w.record(at(t0, 0));
        w.record(at(t0, 10));
        let wait = w.wait(at(t0, 20));
        assert_eq!(wait, Duration::from_secs(40), "освободится на t0+60");
        assert!(w.allow(at(t0, 61)));
    }

    #[test]
    fn window_limit_zero_means_unlimited() {
        // 0 — осознанное «без потолка». Старое `or DEFAULT` превращало 0 в 12.
        let t0 = Instant::now();
        let mut w = Window::new(0, Duration::from_secs(60));
        for i in 0..50 {
            assert!(w.allow(at(t0, i)), "потолка нет: {i}");
            w.record(at(t0, i));
        }
    }

    #[test]
    fn window_forgets_old_hits() {
        let t0 = Instant::now();
        let mut w = Window::new(1, Duration::from_secs(10));
        w.record(t0);
        assert!(!w.allow(at(t0, 5)));
        assert!(w.allow(at(t0, 11)));
        w.record(at(t0, 11));
        assert_eq!(w.len(), 1, "старые должны вылететь из окна");
    }

    // ---- Registry ----
    #[test]
    fn registry_remembers_only_marked() {
        let mut r = Registry::new();
        r.own(10);
        r.own(20);
        assert!(r.contains(10));
        assert!(!r.contains(11));
        r.forget(&[10]);
        assert!(!r.contains(10));
        assert_eq!(r.ids(), vec![20]);
    }

    #[test]
    fn registry_is_bounded() {
        let mut r = Registry::new();
        for i in 0..600 {
            r.own(i);
        }
        assert!(r.len() <= 500, "реестр разросся: {}", r.len());
        assert!(r.contains(599), "свежие id обязаны оставаться");
    }

    // ---- guard_saved ----
    #[test]
    fn guard_refuses_foreign_in_saved_messages() {
        let r = Registry::new();
        match guard_saved(true, &r, &[1, 2]) {
            Some(Refusal::SavedForeign(v)) => assert_eq!(v, vec![1, 2]),
            other => panic!("ожидался отказ, получили {other:?}"),
        }
    }

    #[test]
    fn guard_allows_own_in_saved_messages() {
        let mut r = Registry::new();
        r.own(1);
        r.own(2);
        assert_eq!(guard_saved(true, &r, &[1, 2]), None);
    }

    #[test]
    fn guard_refuses_whole_mixed_batch() {
        let mut r = Registry::new();
        r.own(1);
        match guard_saved(true, &r, &[1, 2]) {
            Some(Refusal::SavedForeign(v)) => assert_eq!(v, vec![2]),
            other => panic!("ожидался отказ целиком, получили {other:?}"),
        }
    }

    #[test]
    fn guard_inactive_outside_saved_messages() {
        let r = Registry::new();
        assert_eq!(guard_saved(false, &r, &[1, 2, 3]), None,
                   "в обычном чате цель определяется списком targets, а не реестром");
    }

    #[test]
    fn guard_empty_batch_refused() {
        let r = Registry::new();
        assert_eq!(guard_saved(false, &r, &[]), Some(Refusal::Empty));
    }

    // ---- plan_batches ----
    #[test]
    fn batches_split_and_capped() {
        let ids: Vec<i32> = (1..=120).collect();
        let b = plan_batches(&ids, 50);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].len(), 50);
        assert_eq!(b[2].len(), 20);
        assert!(b.iter().all(|x| x.len() <= BATCH_MAX));
    }

    #[test]
    fn batch_size_zero_becomes_one() {
        let b = plan_batches(&[1, 2, 3], 0);
        assert_eq!(b.len(), 3, "0 → 1, иначе бесконечный цикл");
    }

    #[test]
    fn batch_size_above_cap_is_clamped() {
        let ids: Vec<i32> = (1..=200).collect();
        let b = plan_batches(&ids, 10_000);
        assert!(b.iter().all(|x| x.len() <= BATCH_MAX),
                "Telegram документирует «up to 100» — выше нельзя");
    }

    #[test]
    fn empty_input_gives_no_batches() {
        assert!(plan_batches(&[], 50).is_empty());
    }
}
