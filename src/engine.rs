//! Ф-R4 — живой код очистителя: всё, что ОБЯЗАНО компилироваться с grammers.
//!
//! ★ Чистая логика (Window/Registry/guard_saved/plan_batches + 20 тестов) вынесена
//!   в `crate::rate`, потому что только она тестируется ЛОКАЛЬНО за 0.03 с:
//!       rustc --test --edition 2021 src/rate.rs -o /tmp/rate && /tmp/rate
//!   Здесь остаётся код, для которого нужен grammers-tl-types 0.12.0 с
//!   `edition = "2024"` (Cargo ≥ 1.85), а в песочнице rustc/cargo 1.83 ⇒
//!   проверка только через CI. Это разделение сокращает CI-раунды в разы.
//!
//! Все формы вызовов ниже сверены с vendor-исходниками (не с доками и не с памятью):
//!
//!   client.messages.rs:903  delete_messages<C: Into<PeerRef>>(peer, &[i32]) -> Result<usize, _>
//!                           «Deletes up to 100 messages… the amount of deleted messages is
//!                            returned (it might be less than the amount of input IDs if some
//!                            were already missing)» ⇒ пакет ≤ 100, счёт — ОТ СЕРВЕРА.
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
//!   client/updates.rs:31    UpdateStream::next(&mut) -> Result<Update, InvocationError>
//!   client/updates.rs:69    stream_updates(mpsc::Receiver<UpdatesLike>, UpdatesConfiguration)
//!                           → Result<UpdateStream, Box<dyn Error + Send + Sync>>;
//!                           док: «peers must have been persisted in the session cache
//!                           beforehand… use iter_dialogs at least once after login»
//!                           ⇒ seed_dialogs() ОБЯЗАТЕЛЕН до live-цикла.
//!   errors.rs (mtsender):186 InvocationError { Session, Rpc, Io, Deserialize, Transport,
//!                           Dropped(&'static str) } — док Dropped: «may mean that the
//!                           SenderPoolRunner is no longer running… contents should not
//!                           be matched on», поэтому сверяем ТОЛЬКО вариант enum.
//!   configuration.rs:198    AutoSleep{tries=1, threshold=60s}: FLOOD_WAIT ≤ 60 с
//!                           ретраится библиотекой один раз, длиннее приходит к нам.

#![allow(dead_code)]

use std::collections::HashSet;
use std::time::Duration;

use grammers_client::session::types::PeerRef;
use grammers_client::update::Update;
use grammers_client::Client;
use grammers_mtsender::{InvocationError, UpdatesConfiguration};
use tokio::sync::mpsc;

use crate::caps::flood_seconds;
// Чистая логика доступна и отсюда ПУБЛИЧНО: `purge_batch` принимает `&mut Registry`
// и возвращает `Result<_, Refusal>`, поэтому вызывающий слой обязан уметь назвать
// эти типы, не зная про внутреннее устройство модуля rate.
pub use crate::rate::{guard_saved, plan_batches, Registry, Refusal, BATCH_MAX};

// Чистая логика доступна и отсюда, чтобы вызывающий слой не менялся при выносе
// в `rate` (и чтобы регресс «модуль написан, но не подключён» ловился обоими путями).
pub use crate::rate::{GLOBAL_RATE_PER_MIN, Window};

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

/// Основной live-цикл. Возвращает число удалённых сообщений, когда пул остановки.
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
                // ★ Dropped = пул больше не работает. Строковое содержимое сопоставлять
                //   нельзя (док errors.rs:224-226), поэтому только вариант enum.
                //   Это ЕДИНСТВЕННЫЙ выход из цикла со счётчиком: без него `removed`
                //   был «assigned but never used» (warning раунда 11) — то есть число
                //   удалённого терялось навсегда.
                if matches!(e, InvocationError::Dropped(_)) {
                    return Ok(removed);
                }
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

/// Пакетное удаление всей пачки id с гардом (путь для sweep-задач, не live).
pub async fn purge_all(
    client: &Client,
    peer: PeerRef,
    is_self_chat: bool,
    reg: &mut Registry,
    ids: &[i32],
) -> Result<usize, Refusal> {
    let mut total = 0usize;
    for batch in plan_batches(ids, BATCH_MAX) {
        total += purge_batch(client, peer.clone(), is_self_chat, reg, &batch).await?;
        // пауза между пакетами: один RTT ~191 мс медиана, дешевле ждать, чем ловить FLOOD_WAIT
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    Ok(total)
}
