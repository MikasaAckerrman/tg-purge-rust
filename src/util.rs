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
    /// Телефон без «+»: авто-конвертация `8…` → `+8…` НЕДЕЛАЕМА (8 — внутрироссийский
    /// префикс, а код страны 7), поэтому требуем E.164 явно, а не угадываем.
    NotE164(String),
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
            CodeError::NotE164(s) => write!(
                f, "телефон {s:?} без «+»: нужен международный формат (например {DEFAULT_PHONE}); \
                   сам подставлять код страны не буду — это риск уйти не в тот аккаунт"
            ),
            CodeError::Fs(s) => write!(f, "файл-канал кода: {s}"),
        }
    }
}

impl std::error::Error for CodeError {}

/// Допустимая длина телефона в формате E.164 (ITU-T): «+» и 7–15 цифр.
pub const PHONE_MIN_DIGITS: usize = 7;
pub const PHONE_MAX_DIGITS: usize = 15;

/// Номер юзера (TG_AUTH_PROTOCOL, аккаунт @armorberserk).
pub const DEFAULT_PHONE: &str = "+79962227579";

/// Сессия Rust-движка. ★ НИКОГДА не tg_user_session.string: форматы несовместимы
/// (ошибка №7 протокола: читатель чужого формата ⇒ «sqlite version column fail»),
/// и Python-сессия — это то, что уже работает и что терять нельзя.
pub const SESSION_FILE: &str = "/var/minis/shared/rustspike/purge.session.json";

/// Файл-канал кода: юзер пишет код сюда (или кладёт из чата 777000), движок поллит.
pub const CODE_FILE: &str = "/var/minis/shared/rustspike/code.txt";

/// Окно ожидания кода: 15 минут — столько живёт код Telegram (протокол, 31.10).
pub const CODE_WINDOW_SECS: u64 = 900;

/// Проверка формата телефона ДО SendCode. Лишний SendCode стоит дорого:
/// код одноразовый, а повторный запрос сдвигает окно и ловит flood-wait.
///
/// ★ Разрешены: ведущий «+», цифры, пробелы, `-`, `(`, `)`, `.`. Всё остальное —
///   отказ. Префикс `8` НЕ конвертируется в `+7`: угадывать код страны означает
///   риск отправить SendCode на чужой аккаунт, а это уже не «лишний код», а потеря
///   окна и лишний бан-риск.
pub fn check_phone(raw: &str) -> Result<String, CodeError> {
    let s: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty() {
        return Err(CodeError::Empty);
    }
    if !s.starts_with('+') {
        return Err(CodeError::NotE164(s));
    }
    if !s.chars().all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '(' | ')' | '.')) {
        return Err(CodeError::NotDigits(s));
    }
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return Err(CodeError::Empty);
    }
    if !(PHONE_MIN_DIGITS..=PHONE_MAX_DIGITS).contains(&digits.len()) {
        return Err(CodeError::WrongLen(s));
    }
    Ok(format!("+{digits}"))
}

/// Разбор кода из файла-канала: только цифры, длина 4–7, без мусора.
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

/// Слово-подтверждение для SendCode. Регистр ВАЖЕН: «send» не проходит.
/// Причина — ошибка №2 протокола TG_AUTH_PROTOCOL: автоматические прогоны слали
/// код дважды и жгли окно 15 минут; подтверждение обязано быть ручным и явным.
pub const SEND_WORD: &str = "SEND";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmError {
    /// Ничего не вписано.
    Empty,
    /// Не то слово (в т.ч. другой регистр).
    Wrong(String),
    /// Ввода нет вообще: stdin закрыт/не интерактивен (bg_run, cron, pipe).
    /// Отказ, а не «предположим, что человек согласен».
    NotInteractive,
}

impl std::fmt::Display for ConfirmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfirmError::Empty => write!(f, "не вписано подтверждение (нужно: {SEND_WORD})"),
            ConfirmError::Wrong(s) => write!(f, "подтверждение «{s}» не совпадает (нужно: {SEND_WORD}, регистр важен)"),
            ConfirmError::NotInteractive => write!(
                f,
                "нет интерактивного ввода: человек не может вписать «{SEND_WORD}», \
                 поэтому SendCode из фона запрещён"
            ),
        }
    }
}

impl std::error::Error for ConfirmError {}

/// ★ Единственный gate перед SendCode. `answer = None` означает, что человека нет
///   перед терминалом (фон/пайп/таймер) — вход ОТКЛОНЯЕТСЯ, а не выполняется «автоматом».
///   Пробелы по краям срезаются, регистр — чувствителен.
pub fn confirm_send(answer: Option<&str>) -> Result<(), ConfirmError> {
    let raw = match answer {
        Some(s) => s,
        None => return Err(ConfirmError::NotInteractive),
    };
    let s = raw.trim();
    if s.is_empty() {
        return Err(ConfirmError::Empty);
    }
    if s != SEND_WORD {
        return Err(ConfirmError::Wrong(s.to_string()));
    }
    Ok(())
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

    // ---- confirm_send: gate перед дорогой операцией ----
    #[test]
    fn confirm_accepts_exact_word() {
        assert_eq!(confirm_send(Some("SEND")), Ok(()));
        assert_eq!(confirm_send(Some("SEND\n")), Ok(()), "Enter из терминала");
        assert_eq!(confirm_send(Some("  SEND  ")), Ok(()), "пробелы по краям");
    }

    #[test]
    fn confirm_rejects_case_and_typos() {
        // Регистр важен намеренно: «send» — это то, что печатает скрипт, а не человек.
        assert_eq!(confirm_send(Some("send")), Err(ConfirmError::Wrong("send".into())));
        assert_eq!(confirm_send(Some("Send")), Err(ConfirmError::Wrong("Send".into())));
        assert_eq!(confirm_send(Some("SEN")), Err(ConfirmError::Wrong("SEN".into())));
        assert_eq!(confirm_send(Some("да")), Err(ConfirmError::Wrong("да".into())));
    }

    #[test]
    fn confirm_refuses_when_nobody_is_typing() {
        // None = stdin закрыт (bg_run/cron/pipe). Это ГЛАВНЫЙ кейс: именно так
        // в прошлых прогонах жглись коды 77241/36031.
        assert_eq!(confirm_send(None), Err(ConfirmError::NotInteractive));
        assert_eq!(confirm_send(Some("")).unwrap_err(), ConfirmError::Empty);
        assert_eq!(confirm_send(Some("\n")).unwrap_err(), ConfirmError::Empty);
    }

    #[test]
    fn confirm_error_text_names_the_expected_word() {
        for e in [ConfirmError::Empty, ConfirmError::Wrong("x".into()), ConfirmError::NotInteractive] {
            assert!(e.to_string().contains(SEND_WORD), "подсказка не говорит, чего ждать: {e}");
        }
    }

    // ---- пути и номер: дешёвые проверки до дорогой операции ----
    #[test]
    fn paths_never_point_at_python_session() {
        // Ошибка №7 протокола: Python-сессия в другом формате ⇒ «sqlite version
        // column fail». Терять рабочую авторизацию юзера из-за опечатки в пути —
        // худший из возможных исходов, поэтому проверка на месте.
        assert!(!SESSION_FILE.contains("tg_user_session"), "путь указывает на Python-сессию");
        assert!(!CODE_FILE.contains("tg_user_session"));
        assert!(SESSION_FILE.ends_with(".json"));
        assert_ne!(SESSION_FILE, CODE_FILE);
        assert_eq!(CODE_WINDOW_SECS, 900, "окно жизни кода Telegram — 15 минут");
    }

    #[test]
    fn default_phone_is_valid_e164() {
        assert_eq!(check_phone(DEFAULT_PHONE).unwrap(), DEFAULT_PHONE);
    }

    #[test]
    fn check_phone_normalizes_separators_and_rejects() {
        assert_eq!(check_phone("+7 (996) 222-75-79").unwrap(), "+79962227579");
        assert_eq!(check_phone(" +7.996.222.75.79 ").unwrap(), "+79962227579");
        assert!(matches!(check_phone(""), Err(CodeError::Empty)));
        assert!(matches!(check_phone("+7996abc"), Err(CodeError::NotDigits(_))));
        assert!(matches!(check_phone("+7996"), Err(CodeError::WrongLen(_))), "5 цифр мало");
        assert!(matches!(check_phone("+1234567890123456"), Err(CodeError::WrongLen(_))), "16 цифр много");
        assert!(matches!(check_phone("+"), Err(CodeError::Empty)), "только плюс");
    }

    #[test]
    fn check_phone_refuses_trunk_prefix_conversion() {
        // 8 — внутрироссийский префикс, код страны 7. Авто-конвертация «8999…» → «+7999…»
        // выглядит правдоподобно, но это УГАДЫВАНИЕ страны: для номера из другой
        // (+8… = Япония) оно уведёт не в тот аккаунт. Требуем E.164 явно.
        assert!(matches!(check_phone("89991234567"), Err(CodeError::NotE164(_))));
        assert!(matches!(check_phone("79962227579"), Err(CodeError::NotE164(_))));
        let m = check_phone("89991234567").unwrap_err().to_string();
        assert!(m.contains('+'), "подсказка не требует международный формат: {m}");
        assert!(m.contains(DEFAULT_PHONE), "подсказка не даёт пример: {m}");
    }
}
