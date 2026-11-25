//! TGPURGE Rust — библиотека (тот же код используют bin-`purge` и cdylib для APK).
//!
//! Гейты (PLAN_RUST §3) проходятся компилятором и юнит-тестами здесь, до любого
//! сетевого кода:
//!   * `caps`  — доказательства того, что grammers 0.12.0 умеет нужные операции,
//!               и честный список того, чего НЕ умеет (описание супергруппы,
//!               встроенный FloodWait-ретрай).
//!   * `store` — персистентная сессия (`Session` поверх JSON-файла), потому что
//!               `MemorySession` не сохраняет состояние, а повторный логин дорог
//!               по flood-wait.

#![deny(unsafe_code)]

pub mod auth;
pub mod caps;
pub mod engine;
pub mod rate;
pub mod store;
pub mod util;

pub use store::{FileSession, FORMAT_VERSION, StoreError};

/// Версия формата сессии на диске. Наружу отдаём числом, чтобы тест и CLI
/// сверяли одно и то же.
pub const SESSION_FORMAT: u32 = FORMAT_VERSION;

/// Тип-указатель на «это наш формат», чтобы нельзя было случайно передать сырое число.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatVersion(pub u32);

impl FormatVersion {
    pub const fn current() -> Self {
        Self(FORMAT_VERSION)
    }

    pub fn is_compatible(self) -> bool {
        self.0 == FORMAT_VERSION
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_report_has_one_known_gap() {
        // ГЕЙТ R3: ровно ОДИН непройденный пункт — описание супергруппы.
        // Второй «провал», который я заявлял про FloodWait, оказался МОЕЙ ошибкой:
        // AutoSleep реально подключён в grammers-mtsender (configuration.rs:123,198),
        // мёртв только файл grammers-client/src/client/retry_policy.rs.
        // Если список изменится — тест падает, и план правится ДО написания движка.
        let rows = crate::caps::capabilities();
        let gaps: Vec<_> = rows.iter().filter(|(_, _, ok)| !*ok).map(|(n, _, _)| *n).collect();
        assert_eq!(
            gaps,
            vec!["оформление: описание СУПЕРГРУППЫ"],
            "список непройденных гейтов изменился: {gaps:?}"
        );
        assert!(rows.len() >= 12);
    }

    #[test]
    fn engine_modules_are_wired() {
        // Регресс на «модуль написан, но не подключён»: без этого lib компилируется,
        // а юнит-тесты не запускаются вообще (так я уже потерял init-баг U-D29).
        // Проверяем ОБА пути: чистая логика живёт в `rate`, а `engine` обязан её
        // реэкспортировать — иначе вызывающий слой не может назвать типы в сигнатурах
        // (`purge_batch(&mut Registry) -> Result<_, Refusal>`).
        let mut reg = crate::rate::Registry::new();
        reg.own(1);
        assert_eq!(crate::rate::guard_saved(true, &reg, &[1]), None);
        assert!(crate::rate::guard_saved(true, &reg, &[2]).is_some());
        assert_eq!(crate::rate::plan_batches(&[1, 2, 3], 2).len(), 2);

        let mut reg2 = crate::engine::Registry::new();
        reg2.own(2);
        assert_eq!(crate::engine::guard_saved(true, &reg2, &[2]), None,
                   "engine не реэкспортирует guard_saved");
        assert_eq!(crate::engine::Registry::new().len(), 0);
        assert!(crate::engine::BATCH_MAX > 0 && crate::engine::BATCH_MAX <= 100,
                "пакет обязан влезать в лимит Telegram «up to 100»");
        assert_eq!(crate::engine::GLOBAL_RATE_PER_MIN, 12);
    }

    #[test]
    fn rate_window_is_reachable_from_engine() {
        // Window — часть публичного API ответок: UI/CLI не должен знать, что он в rate.
        let w = crate::engine::Window::new(2, std::time::Duration::from_secs(60));
        assert!(w.allow(std::time::Instant::now()));
    }

    #[test]
    fn session_format_version_matches_store() {
        assert_eq!(SESSION_FORMAT, FormatVersion::current().0);
        assert!(FormatVersion(FORMAT_VERSION).is_compatible());
        assert!(!FormatVersion(FORMAT_VERSION + 1).is_compatible());
    }
}
