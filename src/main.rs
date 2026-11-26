//! TGPURGE Rust — бинарь: гейт R1 (сборка), таблица caps и путь входа (Ф-R3).
//!
//! Логика живёт в lib-крете (`purge`), чтобы один и тот же код использовали
//! и этот bin, и cdylib внутри APK. Здесь только CLI-обвязка и ОДИН интерактивный
//! канал ввода кода.
//!
//! ★ Почему вход устроен именно так (протокол `/var/minis/shared/TG_AUTH_PROTOCOL.md`):
//!   * Telegram привязывает auth-код к коннекту, где был SendCode ⇒ SendCode и SignIn
//!     обязаны быть в ОДНОМ процессе (ошибка №1 протокола: второй процесс ⇒ мгновенный
//!     PHONE_CODE_EXPIRED даже через 3 секунды);
//!   * код приходит в чат 777000 (Ayugram), пуш официала на телефоне мёртв;
//!   * окно жизни кода 15 минут;
//!   * лишний SendCode дорог (flood-wait), поэтому перед ним — ручное подтверждение
//!     словом SEND, а из фона (без интерактивного stdin) вход запрещён вовсе.
//!
//! Канал кода: если терминал интерактивен — человек печатает код здесь, и поток
//! пишет его в файл-канал; движок поллит тот же файл. Второй источник правды не
//! создаётся: разбор кода один (`purge::util::parse_code`, 17 локальных тестов).

// ★ BufRead из импорта убран (раунд 17: warning `unused import: BufRead`):
//   у `std::io::Stdin` метод read_line — ВСТРОЕННЫЙ (inherent), а не через трейт
//   BufRead, поэтому импорт не нужен. Write нужен для flush() промпта.
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use purge::auth::AuthRunner;
use purge::caps;
use purge::store::FileSession;
use purge::util::{
    self, CODE_FILE, CODE_WINDOW_SECS, DEFAULT_PHONE, SEND_WORD, SESSION_FILE,
};

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    let code = match arg.as_str() {
        "--caps" => {
            print_caps();
            0
        }
        "--session-format" => {
            println!("{}", purge::SESSION_FORMAT);
            0
        }
        "--auth" => match run_auth() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("ВХОД НЕ СОСТОЯЛСЯ: {e}");
                1
            }
        },
        "--auth-status" => match auth_status() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("СТАТУС НЕ ПОЛУЧЕН: {e}");
                2
            }
        },
        "--serve" => serve_cmd(),
        _ => {
            // Без аргументов — только проверка, что типы сходятся (сети нет).
            caps::pool_typechecks(0);
            println!("purge: компиляция типов прошла");
            println!("  --caps          возможности grammers, доказанные компилятором");
            println!("  --auth-status   авторизована ли сессия (НЕ слать код)");
            println!("  --auth          полный вход: подтверждение «{SEND_WORD}» → код → сессия");
            0
        }
    };
    std::process::exit(code);
}

// ---------------------------------------------------------------- helpers
fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("не поднял runtime: {e}"))
}

fn env_str(name: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("переменная {name} не задана"))
}

fn env_i32(name: &str) -> Result<i32, String> {
    let raw = env_str(name)?;
    raw.trim()
        .parse::<i32>()
        .map_err(|_| format!("{name} не число: {raw:?}"))
}

/// None = человек не печатает (stdin закрыт/не интерактивен) ⇒ вход запрещён.
fn read_answer(prompt: &str) -> Option<String> {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    match std::io::stdin().read_line(&mut s) {
        Ok(0) => None,
        Ok(_) => Some(s),
        Err(_) => None,
    }
}

// ---------------------------------------------------------------- --serve
/// Поднять HTTP-слой для APK.
///
/// ★ Это и есть «управление от APK» без FFI: Kotlin извлекает бинарь `purge` в
///   files_dir, chmod 700 и запускает `purge serve --addr 127.0.0.1:8790`,
///   WebView грузит тот же адрес. Слой не трогает Telegram вообще (только std +
///   serde_json + tiny_http), поэтому работает до и после гейта R2.
fn serve_cmd() -> i32 {
    let addr = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "127.0.0.1:8790".to_string());
    let exe = std::env::current_exe().ok();
    let dir = exe.as_ref().and_then(|p| p.parent()).map(PathBuf::from);
    let (assets, state) = match dir {
        Some(d) => (d.join("ui"), d.join("state.json")),
        None => {
            eprintln!("SERVE: не определил каталог бинаря");
            return 2;
        }
    };
    eprintln!("serve: addr={addr} assets={} state={}", assets.display(), state.display());
    let cfg = purge::api::Config { addr: addr.clone(), assets, state };
    let live = std::sync::Arc::new(purge::api::Live::default());
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    match purge::api::serve(cfg, live, stop) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("SERVE УПАЛ: {e}");
            1
        }
    }
}

// ---------------------------------------------------------------- --caps
fn print_caps() {
    let rows = caps::capabilities();
    let total = rows.len();
    let missing = rows.iter().filter(|(_, _, ok)| !*ok).count();
    let sep = "-".repeat(112);
    println!("{:<44} {:<58} {}", "ВОЗМОЖНОСТЬ", "ЧЕМ ДОКАЗАНО", "ЕСТЬ");
    println!("{sep}");
    for (name, proof, ok) in rows {
        println!(
            "{:<44} {:<58} {}",
            name,
            proof,
            if ok { "да" } else { "НЕТ — гейт не пройден" }
        );
    }
    println!("{sep}");
    println!(
        "итого: {} из {} доступно; непройденных гейтов: {}",
        total - missing,
        total,
        missing
    );
    let _ = std::io::stdout().flush();
}

// ---------------------------------------------------------------- --auth-status
/// Проверка авторизации БЕЗ SendCode: дешёвая операция, коды не тратятся.
fn auth_status() -> Result<(), String> {
    let api_id = env_i32("TG_API_ID")?;
    let session = Arc::new(
        FileSession::open(SESSION_FILE).map_err(|e| format!("сессия: {e}"))?,
    );
    let rt = runtime()?;
    rt.block_on(async move {
        let runner = AuthRunner::connect(Arc::clone(&session), api_id)
            .await
            .map_err(|e| format!("connect: {e}"))?;
        let authorized = runner
            .client
            .is_authorized()
            .await
            .map_err(|e| format!("is_authorized: {e}"))?;
        if !authorized {
            println!("СЕССИЯ НЕ АВТОРИЗОВАНА — нужен: tg-rust --auth");
            runner.shutdown().await;
            return Ok(());
        }
        let me = runner
            .client
            .get_me()
            .await
            .map_err(|e| format!("get_me: {e}"))?;
        println!(
            "АВТОРИЗОВАН: id={} @{} ({})",
            me.id().bare_id_unchecked(),
            me.username().unwrap_or("нет ника"),
            // ★ first_name() -> Option<&str> (peer/user.rs:128), а не &str:
            //   println! с Option не компилируется (нет Display).
            me.first_name().unwrap_or("без имени"),
        );
        runner.shutdown().await;
        Ok(())
    })
}

// ---------------------------------------------------------------- --auth
fn run_auth() -> Result<(), String> {
    let api_id = env_i32("TG_API_ID")?;
    let api_hash = env_str("TG_API_HASH")?;
    let phone_raw = std::env::var("TG_PHONE").unwrap_or_else(|_| DEFAULT_PHONE.to_string());
    let phone = util::check_phone(&phone_raw).map_err(|e| e.to_string())?;
    let password = std::env::var("TG_2FA_PASSWORD").ok().filter(|v| !v.trim().is_empty());

    // ★ Gate ДО SendCode: ручное слово, и только из интерактивного терминала.
    let answer = read_answer(&format!(
        "Впиши {SEND_WORD} (без кавычек), чтобы Telegram прислал код на {phone}: "
    ));
    util::confirm_send(answer.as_deref()).map_err(|e| e.to_string())?;

    // Обнуляем файл-канал: иначе следующий прогон мгновенно прочитает ПРОШЛЫЙ код
    // и получит PHONE_CODE_EXPIRED (ошибка №5 протокола).
    util::prepare_code_channel(std::path::Path::new(CODE_FILE)).map_err(|e| e.to_string())?;

    println!(
        "Код придёт в чат 777000 (Ayugram). Как только увидишь его — \
         впиши сюда или положи в {CODE_FILE}."
    );
    // Поток-перехватчик: человек печатает код в терминале, мы кладем его в файл-канал,
    // а движок поллит тот же файл (разбор один — util::parse_code, 17 локальных тестов).
    std::thread::spawn(|| {
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) => {}
            Ok(_) => {
                let _ = std::fs::write(CODE_FILE, line.trim());
            }
            Err(_) => {}
        }
    });

    let session = Arc::new(
        FileSession::open(SESSION_FILE).map_err(|e| format!("сессия: {e}"))?,
    );
    let rt = runtime()?;
    rt.block_on(async move {
        let runner = AuthRunner::connect(Arc::clone(&session), api_id)
            .await
            .map_err(|e| format!("connect: {e}"))?;
        let (outcome, runner) = runner
            .authorize(
                &phone,
                &api_hash,
                password.as_deref(),
                std::path::Path::new(CODE_FILE),
                Duration::from_secs(CODE_WINDOW_SECS),
            )
            .await
            .map_err(|e| e.to_string())?;
        println!(
            "ВШЁЛ: id={} @{} ({}) · сессия сохранена={} · код ждали {} с",
            outcome.id,
            outcome.username,
            outcome.first_name,
            outcome.session_saved,
            outcome.waited_secs,
        );
        if !outcome.session_saved {
            return Err("сессия НЕ сохранена: при следующем запуске придётся входить заново"
                .to_string());
        }
        runner.shutdown().await;
        Ok(())
    })
}
