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
pub mod store;

pub use store::{FileSession, FormatVersion, FORMAT_VERSION, StoreError};

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
    fn caps_report_has_two_known_gaps() {
        // Гейт R3: ровно два известных провала. Если их станет больше — тест падает,
        // и план меняется до написания движка, а не после.
        let rows = crate::caps::capabilities();
        let gaps: Vec<_> = rows.iter().filter(|(_, _, ok)| !*ok).map(|(n, _, _)| *n).collect();
        assert_eq!(
            gaps,
            vec![
                "оформление: описание СУПЕРГРУППЫ",
                "FloodWait: пауза силами движка",
            ],
            "список непройденных гейтов изменился: {gaps:?}"
        );
        assert!(rows.len() >= 12);
    }

    #[test]
    fn session_format_version_matches_store() {
        assert_eq!(SESSION_FORMAT, FormatVersion::current().0);
        assert!(FormatVersion(FORMAT_VERSION).is_compatible());
        assert!(!FormatVersion(FORMAT_VERSION + 1).is_compatible());
    }
}
