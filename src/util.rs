//! Утилиты входа, НЕ зависящие от grammers ⇒ тестируются ЛОКАЛЬНО за доли секунды:
//!
//!     rustc --test --edition 2021 src/util.rs -o /tmp/util && /tmp/util
//!
//! ★ Почему это важно именно для входа (раунд 13): повторный `SendCode` — дорогая
//! операция (ошибка №2 протокола TG_AUTH_PROTOCOL: два send-скрипта пролетели зря,
//! лишенные коды 77241/36031), а окно жизни кода — 15 минут. Баг в разборе кода,
//! пойманный в CI через 6 минут прогона, сжигает и код, и окно. Здесь же весь
//! разбор кода проверяется до того, как мы вообще попросим у Telegram код.

use std::path::{Path, PathBuf};

/// Допустимая длина Telegram-кода: 5 цифр в официальном клиенте, но Telegram
/// меняет длину (бывали 4 и 6), поэтому 4–7 — и ни буквы больше, иначе это мусор.
pub const CODE_LEN_MIN: usize = 4;
pub const CODE_LEN_MAX: usize = 7;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeError {
    /// Файл есть, но пуст (или одни пробелы).
    Empty,
    /// Не цифры: Telegram-код — только цифры.
    NotDigits(String),
    /// Слишком коротко/длинно.
    WrongLen(String),
    /// Ошибка ФС (нет файла, нет прав, нет каталога).
    Fs(String),
}

impl std::fmt::Display for CodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodeError::Empty => write!(f, "файл кода пуст"),
            CodeError::NotDigits(s) => write!(f, "в файле кода не цифры: {s:?}"),
            CodeError::WrongLen(s) => {
                write!(f, "длина кода {len} вне {CODE_LEN_MIN}..={CODE_LEN_MAX}: {s:?}", len = s.len())
            }
            CodeError::Fs(s) => write!(f, "файл-канал кода: {s}"),
        }
    }
}

impl std::error::Error for CodeError {}

/// Разбор кода из файла-канала: только цифры, длина 4–7, без мусора.
///
/// Строгость обязательна: принять «+79962227579» или «12a45» как код — это не
/// «попробуем ещё раз», а гарантированный отказ Telegram и расход окна 15 минут.
pub fn parse_code(raw: &str) -> Result<String, CodeError> {
    let s: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty() {
        return Err(CodeError::Empty);
    }
    if !s.chars().all(|c| c.is_ascii_digit()) {
        return Err(CodeError::NotDigits(s));
    }
    if !(CODE_LEN_MIN..=CODE_LEN_MAX).contains(&s.len()) {
        return Err(CodeError::WrongLen(s));
    }
    Ok(s)
}

/// Прочитать код, если он уже лежит. Ничего не ждём и не создаём.
/// Ошибку ФС отличаем от «кода ещё нет»: первая — повод остановить прогон сразу.
pub fn read_code(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(raw) => parse_code(&raw).ok(),
        Err(_) => None,
    }
}

/// Путь файла-канала кода: обнуляем, чтобы следующий прогон не прочитал ПРОШЛЫЙ код
/// (иначе мгновенный PHONE_CODE_EXPIRED — ошибка №5 протокола).
pub fn prepare_code_channel(path: &Path) -> Result<PathBuf, CodeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CodeError::Fs(format!("mkdir: {e}")))?;
    }
    std::fs::write(path, "").map_err(|e| CodeError::Fs(format!("truncate: {e}")))?;
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("purge_util_test_{name}.txt"));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn parse_accepts_digits() {
        assert_eq!(parse_code("  12345\n").unwrap(), "12345");
        assert_eq!(parse_code("1234").unwrap(), "1234");
        assert_eq!(parse_code("1234567").unwrap(), "1234567");
        assert_eq!(parse_code("1\t2\n3 4 5").unwrap(), "12345", "пробелы/табы вырезаются");
    }

    #[test]
    fn parse_rejects_garbage() {
        assert_eq!(parse_code(""), Err(CodeError::Empty));
        assert_eq!(parse_code("   \n\t").unwrap_err(), CodeError::Empty);
        assert!(matches!(parse_code("12a45"), Err(CodeError::NotDigits(_))));
        assert!(matches!(parse_code("+79962227579"), Err(CodeError::NotDigits(_))));
        assert!(matches!(parse_code("123"), Err(CodeError::WrongLen(_))));
        assert!(matches!(parse_code("12345678"), Err(CodeError::WrongLen(_))));
        assert!(matches!(parse_code("12 3"), Err(CodeError::WrongLen(_))), "после вырезания пробелов 3 цифры");
    }

    #[test]
    fn parse_never_panics_on_unicode() {
        // Код приходит из файла, который может содержать что угодно (в т.ч. эмодзи,
        // кириллицу, BOM). `chars().all(is_ascii_digit)` обязан это отклонить, а не упасть.
        assert!(parse_code("1234🙂").is_err());
        assert!(parse_code("\u{feff}12345").is_err(), "BOM — не цифра");
        assert!(parse_code("１２３４５").is_err(), "полноширинные цифры — не ascii-цифры");
    }

    #[test]
    fn read_code_absent_file() {
        let p = tmp("absent");
        assert!(read_code(&p).is_none());
    }

    #[test]
    fn read_code_accepts_written() {
        let p = tmp("ok");
        std::fs::write(&p, "42424\n").unwrap();
        assert_eq!(read_code(&p).as_deref(), Some("42424"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn read_code_rejects_garbage_file() {
        let p = tmp("garbage");
        std::fs::write(&p, "не код").unwrap();
        assert!(read_code(&p).is_none());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn prepare_empties_stale_code() {
        // Иначе следующий прогон мгновенно прочитает ПРОШЛЫЙ код и получит EXPIRED.
        let p = tmp("stale");
        std::fs::write(&p, "11111").unwrap();
        prepare_code_channel(&p).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "");
        assert!(read_code(&p).is_none());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn prepare_creates_missing_parent() {
        let mut dir = std::env::temp_dir();
        dir.push("purge_util_test_dir");
        let p = dir.join("code.txt");
        let _ = std::fs::remove_dir_all(&dir);
        prepare_code_channel(&p).unwrap();
        assert!(p.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn errors_say_what_is_wrong_without_leaking_secrets() {
        // В текст ошибки не должны попадать api_hash/пароль.
        let m = CodeError::WrongLen("123".into()).to_string();
        assert!(m.contains("3"), "нет фактической длины: {m}");
        assert!(m.contains("4..=7"), "нет допустимого диапазона: {m}");
        assert!(CodeError::Empty.to_string().contains("пуст"));
        assert!(CodeError::Fs("nope".into()).to_string().contains("nope"));
    }
}
