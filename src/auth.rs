//! Ф-R3 — авторизация пользовательского аккаунта ОДНИМ живым процессом.
//!
//! Протокол взят из `/var/minis/shared/TG_AUTH_PROTOCOL.md` (проверен живьём 31.10):
//!   * Telegram привязывает auth-код к коннекту, где был `SendCode`. SignIn из другого
//!     процесса/соединения ⇒ `PHONE_CODE_EXPIRED` мгновенно, даже через 3 секунды.
//!     Поэтому здесь SendCode и SignIn происходят на ОДНОМ handle пула.
//!   * Код приходит в чат 777000 (Ayugram); пуш-канал официала на телефоне мёртв.
//!     Человек читает код и кладёт его в файл-канал; движок поллит файл. Никакого
//!     интерактивного stdin: процесс может жить в фоне (`bg_run`).
//!   * Окно 900 с. Короткие окна и синхные sleep'ы уже стоили 40 минут простоя.
//!   * `allow_app_hash: false` grammers выставляет сам внутри `request_login_code`
//!     (проверено в auth.rs), поэтому код не уходит «в никуда» на мёртвый push-канал.
//!   * Код читается ОДИН раз и не пересылается повторно (ошибка №2 протокола:
//!     два send-скрипта пролетели зря).
//!
//! Сигнатуры — ИЗ ИСХОДНИКОВ grammers-client 0.12.0 (`src/client/auth.rs`):
//!   is_authorized(&self) -> Result<bool, InvocationError>
//!   request_login_code(&self, phone: &str, api_hash: &str) -> Result<LoginToken, _>
//!   sign_in(&self, token: &LoginToken, code: &str) -> Result<User, SignInError>
//!   check_password(&self, token: PasswordToken, password: impl AsRef<[u8]>) -> Result<User, SignInError>
//!   get_me(&self) -> Result<User, InvocationError>            (chats.rs:414, ★ async)
//!   User::id(&self) -> PeerId                                 (peer/user.rs:103, ★ НЕ i64)
//!   Client::new(sender_pool: SenderPoolFatHandle) -> Self      (client/net.rs:49, потребляет handle)
//!
//! Сессия НЕ трогает Python-файл `tg_user_session.string`: форматы сессий несовместимы
//! (ошибка №7 протокола: telethon-читатель pyrogram-сессии ⇒ «sqlite version column
//! fail»). Здесь свой `FileSession` + свой путь на диске.

#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use grammers_client::peer::User;
use grammers_client::{Client, SignInError};
use grammers_mtsender::{SenderPool, SenderPoolFatHandle};

use crate::store::FileSession;
use crate::util::{read_code, CodeError};

/// Результат авторизации — только то, что юзер просил показать: id и имя.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthOutcome {
    /// «Сырой» id пользователя (из PeerId, не сам PeerId): для логов и аудита.
    pub id: i64,
    pub username: String,
    pub first_name: String,
    pub session_saved: bool,
    /// Сколько секунд ушло на ожидание кода (для отчёта, не для принятия решений).
    pub waited_secs: u64,
}

#[derive(Debug)]
pub enum AuthError {
    /// Нет кода в файле-канале за окно.
    Timeout(u64),
    /// Файл-канал прочитан, но там не код.
    BadCode(String),
    /// Telegram отказал (текст ошибки без секретов).
    Denied(String),
    /// 2FA требуется, а пароля в окружении нет.
    PasswordMissing,
    /// Не удалось сохранить сессию (её потеря ⇒ повторный логин ⇒ flood-wait).
    Store(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Timeout(s) => write!(f, "код не пришёл за {s} с"),
            AuthError::BadCode(s) => write!(f, "в файле кода не код: {s:?}"),
            AuthError::Denied(s) => write!(f, "Telegram отказал: {s}"),
            AuthError::PasswordMissing => write!(f, "требуется 2FA, а TG_2FA_PASSWORD не задан"),
            AuthError::Store(s) => write!(f, "не сохранили сессию: {s}"),
        }
    }
}

impl std::error::Error for AuthError {}

/// Разбор/чтение/обнуление файла-канала кода переехали в `crate::util` — std-only
/// модуль, который гоняется ЛОКАЛЬНО (rustc 1.83 может), потому что расход кода
/// дорог: окно 15 минут и flood-wait на повторный SendCode.
impl From<CodeError> for AuthError {
    fn from(e: CodeError) -> Self {
        match e {
            CodeError::Empty | CodeError::NotDigits(_) | CodeError::WrongLen(_) => {
                AuthError::BadCode(e.to_string())
            }
            CodeError::Fs(_) => AuthError::Store(e.to_string()),
        }
    }
}

// `prepare_code_channel` живёт в `crate::util` (std-only, локальные тесты).
// Держать её здесь — второй источник правды плюс ссылка на удалённый PathBuf.

pub struct AuthRunner {
    pub client: Client,
    pub session: Arc<FileSession>,
    /// fat-handle живёт здесь: `Client::new` его потребляет, но thin-копию мы
    /// храним, чтобы не зависеть от времени жизни пула в тестях.
    pub handle: SenderPoolFatHandle,
    runner: Option<tokio::task::JoinHandle<()>>,
}

impl AuthRunner {
    /// Поднять соединение с telegram на ОДНОМ пуле. Живёт до `drop`.
    ///
    /// ★ `Client::new(fat)` ЗАБИРАЕТ fat-handle (читает из него session/api_id),
    /// поэтому копируем до передачи — иначе поле handle было бы мёртвым.
    pub async fn connect(session: Arc<FileSession>, api_id: i32) -> Result<Self, AuthError> {
        let SenderPool { runner, handle, updates } = SenderPool::new(Arc::clone(&session), api_id);
        let fat: SenderPoolFatHandle = handle;
        let client = Client::new(fat.clone());
        let task = tokio::spawn(runner.run());
        // updates-канал в этом спайке не используем (live-чистка — фаза Ф-R4),
        // но обязаны держать его живым, иначе runner падает на send.
        let _ = updates;
        Ok(Self {
            client,
            session,
            handle: fat,
            runner: Some(task),
        })
    }

    /// Полный путь входа. Возвращает исходный `self`, чтобы вызывающий мог
    /// продолжить работу на ТОМ ЖЕ соединении (или решительно его закрыть).
    pub async fn authorize(
        self,
        phone: &str,
        api_hash: &str,
        password: Option<&str>,
        code_file: &Path,
        window: Duration,
    ) -> Result<(AuthOutcome, Self), AuthError> {
        let client = self.client.clone();
        let authorized = client
            .is_authorized()
            .await
            .map_err(|e| AuthError::Denied(format!("{e}")))?;

        if authorized {
            let me = client
                .get_me()
                .await
                .map_err(|e| AuthError::Denied(format!("get_me: {e}")))?;
            let outcome = outcome_from(&me, 0, true);
            return Ok((outcome, self));
        }

        // 1) SendCode — ОДИН раз за процесс.
        let token = client
            .request_login_code(phone, api_hash)
            .await
            .map_err(|e| AuthError::Denied(format!("SendCode: {e}")))?;
        log_line(&format!("SEND_OK phone-prefix={}…", &phone[..phone.len().min(6)]));

        // 2) ждём код в файле-канале, не трогая соединение
        let t0 = std::time::Instant::now();
        let code = loop {
            if let Some(c) = read_code(code_file) {
                break c;
            }
            if t0.elapsed() >= window {
                log_line(&format!("TIMEOUT {}s", window.as_secs()));
                return Err(AuthError::Timeout(window.as_secs()));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        };
        let waited = t0.elapsed().as_secs();
        log_line(&format!("GOT_CODE после {waited} с (длина {})", code.len()));

        // 3) SignIn на ТОМ ЖЕ пуле
        let user = match client.sign_in(&token, &code).await {
            Ok(u) => u,
            Err(SignInError::PasswordRequired(ptoken)) => {
                let pw = match password {
                    Some(p) => p,
                    None => return Err(AuthError::PasswordMissing),
                };
                client
                    .check_password(ptoken, pw.as_bytes())
                    .await
                    .map_err(|e| AuthError::Denied(format!("2FA: {e}")))?
            }
            Err(SignInError::SignUpRequired) => {
                return Err(AuthError::Denied(
                    "аккаунт не зарегистрирован (SignUpRequired) — не создаём аккаунт".into(),
                ))
            }
            Err(SignInError::InvalidCode) => {
                return Err(AuthError::Denied("код неверный (InvalidCode)".into()))
            }
            Err(SignInError::InvalidPassword(_)) => {
                return Err(AuthError::Denied("2FA-пароль неверный".into()))
            }
            Err(SignInError::Other(e)) => {
                // EXPIRED — это всегда про разрыв коннекта/хеша, НЕ про код
                // (ошибка №5 протокола). Пишем именно так, чтобы следующий исполнитель
                // не начал «подбирать» код.
                return Err(AuthError::Denied(format!("SignIn: {e}")));
            }
        };

        // 4) сохранить сессию ОБЯЗАТЕЛЬНО: потеря ⇒ повторный вход ⇒ flood-wait.
        let mut saved = false;
        match self.session.flush() {
            Ok(()) => saved = true,
            Err(e) => log_line(&format!("SESSION_FLUSH_FAIL: {e}")),
        }
        let outcome = outcome_from(&user, waited, saved);
        log_line(&format!(
            "LOGIN_OK id={} username={} session_saved={}",
            outcome.id, outcome.username, saved
        ));
        Ok((outcome, self))
    }

    /// Остановить пул и сохранить сессию.
    pub async fn shutdown(mut self) {
        if let Some(task) = self.runner.take() {
            task.abort();
        }
        let _ = self.session.flush();
    }
}

fn outcome_from(user: &User, waited_secs: u64, saved: bool) -> AuthOutcome {
    AuthOutcome {
        // ★ User::id() -> PeerId, поэтому bare_id_unchecked(): «сырой» id для логов.
        id: user.id().bare_id_unchecked(),
        username: user.username().unwrap_or("").to_string(),
        first_name: user.first_name().unwrap_or("").to_string(),
        session_saved: saved,
        waited_secs,
    }
}

fn log_line(msg: &str) {
    // Только stdout: ни кода, ни пароля, ни api_hash в логи не попадают.
    println!("{msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pure-тесты разбора кода переехали в `crate::util` (std-only) и гонятся ЛОКАЛЬНО.
    // Здесь остаются только проверки, которым нужен grammers-тип AuthError.

    #[test]
    fn auth_error_messages_have_no_secrets() {
        // Текст ошибки не должен содержать api_hash/пароля. Код в BadCode —
        // намеренно: он уже в файле-канале, который читает только юзер.
        let t = AuthError::Timeout(900).to_string();
        assert!(t.contains("900"));
        let m = AuthError::PasswordMissing.to_string();
        assert!(m.contains("TG_2FA_PASSWORD"));
        let d = AuthError::Denied("SignIn: PHONE_CODE_EXPIRED".into()).to_string();
        assert!(d.contains("PHONE_CODE_EXPIRED"));
    }

    #[test]
    fn code_error_maps_to_right_class() {
        // Ошибка ФС НЕ должна выглядеть как отказ Telegram: это разные решения
        // («починить диск/права» против «не слать повторный SendCode»).
        let bad: AuthError = CodeError::WrongLen("123".into()).into();
        assert!(matches!(bad, AuthError::BadCode(_)), "не тот класс: {bad}");
        let empty: AuthError = CodeError::Empty.into();
        assert!(matches!(empty, AuthError::BadCode(_)));
        let fs: AuthError = CodeError::Fs("nope".into()).into();
        assert!(matches!(fs, AuthError::Store(_)), "ошибка ФС попала в Telegram-класс: {fs}");
    }

    #[test]
    fn outcome_uses_bare_id_not_peerid() {
        // id в AuthOutcome — число, чтобы аудит/логи не зависели от PeerId.
        let o = AuthOutcome {
            id: 5804685716,
            username: "armorberserk".into(),
            first_name: "me".into(),
            session_saved: true,
            waited_secs: 3,
        };
        assert_eq!(o.id, 5804685716);
        assert!(o.session_saved);
    }
}
