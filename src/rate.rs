//! ЧИСТАЯ логика лимитов и гардов — без Telegram, без сети, без IO.
//!
//! ★ Почему отдельный модуль (раунд 12, 25.11): grammers-tl-types 0.12.0 требует
//! `edition = "2024"` ⇒ Cargo ≥ 1.85, а в песочнице rustc/cargo 1.83 (Alpine).
//! Единственный способ проверить чистую логику БЕЗ 6-минутного CI-прогона — вынести
//! её в файл, который зависит только от `std`, и гонять локально:
//!     rustc --test --edition 2021 src/rate.rs -o /tmp/rate && /tmp/rate
//! Так `Window`/`Registry`/`guard_saved`/`plan_batches` эволюционируют за секунды,
//! а CI тратится только на код, который реально трогает Telegram (`engine.rs`).
//!
//! Инварианты, перенесённые из Python-версии (не обсуждаются, правила юзера [Н]):
//!   * избранное (Saved Messages) неприкосновенно: удалять можно только то, что
//!     создал движок, и только ЦЕЛИКОМ (половинчатое удаление в ЛС с собой — риск
//!     снести нужное юзеру);
//!   * `limit = 0` и `cooldown = 0` — осознанные значения («без потолка»),
//!     старое `x or DEFAULT` превращало 0 в 12 (U-D20);
//!   * пакет удаления ≤ 100 (док Telegram messages.rs:873-888).

use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Максимальный размер пакета удаления. Telegram документирует «up to 100»,
/// берём 50 как в Python-версии: меньше шанс поймать FLOOD_WAIT на одном пакете
/// и всё ещё один RTT (~191 мс медиана) на 50 сообщений.
pub const BATCH_MAX: usize = 50;

/// Глобальный потолок отправок в минуту для ответок (защита аккаунта).
pub const GLOBAL_RATE_PER_MIN: usize = 12;

/// Потолок записей в реестре «создано движком» (RAM демона).
pub const REGISTRY_CAP: usize = 500;

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

    /// Попадания, ещё лежащие в окне. `hits` отсортирован НОВЫМИ ВПЕРЁД.
    ///
    /// ★ Подлёт `checked_sub` (span больше, чем живут монотонные часы) обязан давать
    ///   «ВСЕ попадания», а не «ни одного»: `unwrap_or(now)` делал окно пустым и
    ///   тем самым разваливал лимит (fail-open в ограничителе частоты). Именно на
    ///   таком поведении тесты становились зависимыми от аптайма машины.
    fn in_window(&self, now: Instant) -> Vec<Instant> {
        match now.checked_sub(self.span) {
            Some(cutoff) => self.hits.iter().copied().filter(|t| *t > cutoff).collect(),
            None => self.hits.clone(),
        }
    }

    /// Сколько ждать до следующей разрешённой отправки. 0 = можно сейчас.
    /// limit = 0 означает «без потолка» (юзер вправе поставить 0 осознанно).
    pub fn wait(&self, now: Instant) -> Duration {
        if self.limit == 0 {
            return Duration::ZERO;
        }
        let in_win = self.in_window(now);
        if in_win.len() < self.limit {
            return Duration::ZERO;
        }
        // Чтобы пропустить следующую, должно уйти ЗА ПРЕДЕЛ окна ровно
        // (len - (limit - 1)) самых старых попадания. В массиве «новыми вперёд»
        // k-е с конца стоит в индексе len - k, то есть len - (len - limit + 1) = limit - 1.
        // ★ Раунд 12: индекс `len - limit` брал САМОЕ НОВОЕ попадание вместо самого
        //   старого — тест ждал 40 с, получал 50 с (завышение на целый шаг окна).
        let oldest = in_win[self.limit - 1];
        let free_at = oldest + self.span;
        if free_at <= now { Duration::ZERO } else { free_at - now }
    }

    pub fn allow(&self, now: Instant) -> bool {
        self.wait(now).is_zero()
    }

    pub fn record(&mut self, now: Instant) {
        if self.limit == 0 {
            // без потолка история не нужна: копить её = бесконечный рост RAM
            return;
        }
        self.hits.push(now);
        self.hits.sort_by(|a, b| b.cmp(a)); // новые первыми
        self.hits = self.in_window(now);
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
/// здесь он — только память, а сохранение в state делает вызывающий слой.
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
        if self.own.len() > REGISTRY_CAP {
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
        assert!(w.is_empty(), "без потолка история не должна копиться (RAM)");
    }

    #[test]
    fn window_fail_closed_when_span_overflows_clock() {
        // Span заведомо больше жизни монотонных часов → checked_sub даёт None.
        // Окно обязано считать ВСЕ попадания: `unwrap_or(now)` делал его пустым и
        // разваливало лимит (fail-open), а тесты становились зависимыми от аптайма.
        let t0 = Instant::now();
        let mut w = Window::new(1, Duration::from_secs(60 * 60 * 24 * 3600));
        w.record(t0);
        assert!(!w.allow(at(t0, 1)), "fail-open: окно посчитало пустым при подлёте span");
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn window_waits_for_oldest_not_newest() {
        // Регресс раунда 12: индекс `len - limit` брал новейшее попадание,
        // поэтому wait завышался на целый шаг между отправками (50 с вместо 40).
        let t0 = Instant::now();
        let mut w = Window::new(3, Duration::from_secs(60));
        w.record(at(t0, 0));
        w.record(at(t0, 10));
        w.record(at(t0, 20));
        // развязка: уйдёт САМОЕ СТАРОЕ (t0), значит освободится на t0+60
        assert_eq!(w.wait(at(t0, 21)), Duration::from_secs(39));
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

    #[test]
    fn window_record_is_sorted() {
        // Порядок «новыми вперёд» — опора индекса limit-1 в wait().
        let t0 = Instant::now();
        let mut w = Window::new(3, Duration::from_secs(60));
        w.record(at(t0, 30));
        w.record(at(t0, 0));
        w.record(at(t0, 15));
        assert_eq!(w.len(), 3);
        assert_eq!(w.wait(at(t0, 31)), Duration::from_secs(29), "старейшее t0");
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
        assert!(r.len() <= REGISTRY_CAP, "реестр разросся: {}", r.len());
        assert!(r.contains(599), "свежие id обязаны оставаться");
    }

    #[test]
    fn registry_ids_sorted_and_dedup() {
        let mut r = Registry::new();
        r.own(5);
        r.own(5);
        r.own(1);
        assert_eq!(r.ids(), vec![1, 5]);
        assert_eq!(r.len(), 2);
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

    #[test]
    fn batches_preserve_order_and_content() {
        let ids: Vec<i32> = (1..=7).collect();
        let flat: Vec<i32> = plan_batches(&ids, 3).into_iter().flatten().collect();
        assert_eq!(flat, ids, "пакеты обязаны сохранять порядок и состав");
    }
}
