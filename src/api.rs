//! HTTP-слой управления для APK. Зависимости: `std` + `serde_json` + `tiny_http` —
//! никакого grammers, поэтому весь слой тестируется ЛОКАЛЬНО (tools/localtest),
//! а не шестью минутами CI-прогона.
//!
//! ★ Почему «бинарь как процесс», а не `.so` + JNI (решение 26.11, по замеру):
//!   раунд 18 показал `nm -D --defined-only libpurge.so` → **0 символов T**:
//!   `strip = true` и ни одного `#[no_mangle] extern "C"`. Значит Kotlin звать нечего,
//!   а починка требует либо выключать strip и писать JNI-мост с ручными сигнатурами
//!   (`unsafe`, что ломает `#![deny(unsafe_code)]`), либо тащить JNA.
//!   Запуск того же бинаря как дочернего процесса обходит проблему целиком:
//!   нулевой FFI, тот же loopback-HTTP, и это проверяемо в CI без устройства
//!   (`readelf -d`: NEEDED только bionic).
//!
//! ★ План управления (слова юзера 26.11: «главное, чтобы управление было от APK»):
//!   APK поднимает `purge serve`, WebView грузит `http://127.0.0.1:PORT/`,
//!   все мутации идут через `/api/*` в `state.json`, а движок (Python сейчас,
//!   Rust после гейта R2) горячо читает тот же файл. Один писатель состояния на
//!   каждый файл: движок пишет свои счётчики/эталоны, API пишет только намерения
//!   (`live.enabled`, `live.targets`, `mode`, `watch.*`), иначе гонка флагов
//!   (это и есть M9 из плана v3).
//!
//! ★ Чего здесь НЕТ намеренно (правила юзера [Н]):
//!   - `arm`/`disarm` автовозврата оформления — только терминал с nonce, поэтому
//!     `/api/arm` отвечает 403, а не пишет `armed: true`;
//!   - удаление истории — только `/api/sweep` с `confirm:true` и счётчиком до кнопки;
//!   - пути к галерее телефона — эталоны берутся только из Telegram.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tiny_http::{Header, Response, Server, StatusCode};

use crate::shape;

/// Версия ABI этого HTTP-контракта. APK сверяет её перед тем, как показывать UI:
/// рассинхрон формы ответа = «кнопки не работают», а 404/500 — непонятная ошибка.
pub const API_VERSION: u32 = 3;

/// Имя файла состояния, которое пишет/читает движок.
pub const STATE_FILE: &str = "state.json";

/// Панели, которые движок хранит в state (U-D12: открытая панель переживает reload).
pub const PANELS: &[&str] = &["where", "who", "look", "rules", "log", "chats", "err"];

/// Счётчики живого движка. Пока движок не подключён (гейт R2 не пройден), они нулевые —
/// и UI обязан показывать это как «чистка не активна», а не как «удалено 0 за сессию».
#[derive(Debug, Default)]
pub struct Live {
    pub updates: AtomicU64,
    pub deleted: AtomicU64,
    pub missed: AtomicU64,
    pub running: AtomicU64, // 1 = движок авторизован и слушает
}

impl Live {
    pub fn snapshot(&self) -> Value {
        json!({
            "updates": self.updates.load(Ordering::Relaxed),
            "deleted": self.deleted.load(Ordering::Relaxed),
            "missed": self.missed.load(Ordering::Relaxed),
            "engine_running": self.running.load(Ordering::Relaxed) != 0,
        })
    }
}

#[derive(Debug)]
pub enum ApiError {
    /// Путь вне разрешённого каталога (защита от `../..` в /ui/…).
    PathOutside,
    /// Неизвестная панель/режим/ключ.
    Bad(String),
    /// Действие запрещено через API (arm/disarm — только терминал).
    Forbidden(String),
    /// Ошибка диска.
    Io(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::PathOutside => write!(f, "путь вне разрешённого каталога"),
            ApiError::Bad(s) => write!(f, "некорректное значение: {s}"),
            ApiError::Forbidden(s) => write!(f, "запрещено через API: {s}"),
            ApiError::Io(s) => write!(f, "диск: {s}"),
        }
    }
}

// ─────────────────────────────────────────────── безопасные пути статики
/// Разрешённые имена UI-файлов. Белый список, а не «главный каталог + проверка»:
/// так нельзя вытащить ни state.json, ни сессию, ни что-либо ещё одним `..`.
pub const UI_FILES: &[&str] = &["index.html", "app.css", "app.js", "strings.js"];

/// Проверить, что запрашиваемый статический файл легален, и собрать к нему путь.
pub fn ui_path(assets: &Path, name: &str) -> Result<PathBuf, ApiError> {
    if !UI_FILES.contains(&name) {
        return Err(ApiError::PathOutside);
    }
    // имя из белого списка → слэшей и точек в нём нет по построению, но проверяем:
    // белый список может быть расширен позже, и молчаливый `..` тогда fatal.
    if name.contains('/') || name.contains("..") || name.starts_with('.') {
        return Err(ApiError::PathOutside);
    }
    Ok(assets.join(name))
}

// ───────────────────────────────────────── мутации намерений в state.json
/// Прочитать состояние. Нет файла = пустая схема v3 (не ошибка: первый запуск).
pub fn read_state(path: &Path) -> Result<Value, ApiError> {
    if !path.exists() {
        return Ok(empty_state());
    }
    let mut raw = String::new();
    std::fs::File::open(path)
        .map_err(|e| ApiError::Io(format!("open: {e}")))?
        .read_to_string(&mut raw)
        .map_err(|e| ApiError::Io(format!("read: {e}")))?;
    serde_json::from_str(&raw).map_err(|e| ApiError::Io(format!("parse {}: {e}", path.display())))
}

/// Схема v3 в минимальном виде, достаточном для намерений (пишет только API).
pub fn empty_state() -> Value {
    json!({
        "version": 3,
        "me_id": null,
        "active": null,
        "chats": {},
        "ui": {"panel": ""},
    })
}

/// Атомарная запись: tmp + rename. Половина файла хуже отсутствия файла —
/// иначе обрыв на записі убивает все настройки юзера.
pub fn write_state(path: &Path, v: &Value) -> Result<(), ApiError> {
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(v).map_err(|e| ApiError::Io(format!("ser: {e}")))?;
    std::fs::write(&tmp, body.as_bytes()).map_err(|e| ApiError::Io(format!("write tmp: {e}")))?;
    std::fs::rename(&tmp, path).map_err(|e| ApiError::Io(format!("rename: {e}")))?;
    Ok(())
}

/// Включить/выключить live-чистку. При включении ОБЯЗАТЕЛЬНО ставим базис `since`:
/// без него F4 («пропущено за простой») молчит — это боевая дыра 26.11 (движок убили
/// при суспенде, два сообщения цели выжили, а missed показал 0).
///
/// `now` приходит параметром (а не берётся из часов), чтобы функция чистая и
/// тестировалась без сна. 0 = «неизвестно» → базис не выставляется, и вызывающий
/// обязан это увидеть, а не получить молча валидный-looking state.
pub fn set_live_enabled(
    st: &mut Value,
    chat: &str,
    enabled: bool,
    now: u64,
) -> Result<(), ApiError> {
    let c = chat_entry(st, chat)?;
    let live = c
        .as_object_mut()
        .and_then(|o| o.entry("live").or_insert_with(|| json!({})).as_object_mut())
        .ok_or_else(|| ApiError::Bad("live".into()))?;
    if let Some(v) = live.get_mut("enabled") {
        *v = json!(enabled);
    } else {
        live.insert("enabled".into(), json!(enabled));
    }
    if enabled {
        let has = live.get("since").and_then(|v| v.as_u64()).unwrap_or(0) > 0;
        if !has {
            if now == 0 {
                return Err(ApiError::Bad("нет времени для базиса since".into()));
            }
            live.insert("since".into(), json!(now));
        }
    }
    Ok(())
}

/// Set-целей чата. Пустой список разрешён (снял всех), самоцель — НЕТ:
/// удалять свои сообщения нельзя (и в Python-движке то же правило).
pub fn set_targets(st: &mut Value, chat: &str, ids: &[i64], me_id: i64) -> Result<(), ApiError> {
    let c = chat_entry(st, chat)?;
    if ids.contains(&me_id) {
        return Err(ApiError::Forbidden("себя нельзя выбрать целью".into()));
    }
    let mut uniq: Vec<i64> = ids.to_vec();
    uniq.sort_unstable();
    uniq.dedup();
    let live = c
        .as_object_mut()
        .and_then(|o| o.entry("live").or_insert_with(|| json!({})).as_object_mut())
        .ok_or_else(|| ApiError::Bad("live".into()))?;
    live.insert("targets".into(), json!(uniq));
    Ok(())
}

/// Режим: только новые (`new`) либо новые+старые (`all`). Слово «all» здесь —
/// ТОЛЬКО про сегмент UI; снос истории по-прежнему требует /api/sweep с confirm.
pub fn set_mode(st: &mut Value, chat: &str, mode: &str) -> Result<(), ApiError> {
    if mode != "new" && mode != "all" {
        return Err(ApiError::Bad(format!("mode: {mode}")));
    }
    let c = chat_entry(st, chat)?;
    let live = c
        .as_object_mut()
        .and_then(|o| o.entry("live").or_insert_with(|| json!({})).as_object_mut())
        .ok_or_else(|| ApiError::Bad("live".into()))?;
    live.insert("mode".into(), json!(mode));
    Ok(())
}

/// Детектор оформления (следим, но сами НЕ меняем). Возврат включается только в
/// терминале, поэтому `armed` здесь не трогаем никогда.
pub fn set_watch(st: &mut Value, chat: &str, key: &str, on: bool) -> Result<(), ApiError> {
    if key != "avatar" && key != "about" && key != "title" {
        return Err(ApiError::Bad(format!("watch: {key}")));
    }
    let c = chat_entry(st, chat)?;
    let w = c
        .as_object_mut()
        .and_then(|o| o.entry("watch").or_insert_with(|| json!({})).as_object_mut())
        .ok_or_else(|| ApiError::Bad("watch".into()))?;
    w.insert(key.into(), json!(on));
    Ok(())
}

/// Восстановить открытую панель (U-D12). Неизвестное имя = ошибка, а не «проглотили».
pub fn set_panel(st: &mut Value, panel: &str) -> Result<(), ApiError> {
    if !panel.is_empty() && !PANELS.contains(&panel) {
        return Err(ApiError::Bad(format!("panel: {panel}")));
    }
    let root = st.as_object_mut().ok_or_else(|| ApiError::Bad("state".into()))?;
    let ui = root
        .entry("ui")
        .or_insert_with(|| json!({}));
    let obj = ui.as_object_mut().ok_or_else(|| ApiError::Bad("ui".into()))?;
    obj.insert("panel".into(), json!(panel));
    Ok(())
}

fn chat_entry<'a>(st: &'a mut Value, chat: &str) -> Result<&'a mut Value, ApiError> {
    if chat.is_empty() {
        return Err(ApiError::Bad("пустой chat".into()));
    }
    let root = st.as_object_mut().ok_or_else(|| ApiError::Bad("state".into()))?;
    let chats = root
        .entry("chats")
        .or_insert_with(|| json!({}));
    let map = chats.as_object_mut().ok_or_else(|| ApiError::Bad("chats".into()))?;
    if !map.contains_key(chat) {
        map.insert(chat.to_string(), json!({"live": {}, "watch": {}}));
    }
    Ok(map.get_mut(chat).ok_or_else(|| ApiError::Bad(chat.into()))?)
}

// ─────────────────────────────────────────────────────── форма ответа
/// Собрать ответ `/api/state`: состояние с диска + живые счётчики, и ПРОГОН через
/// `shape::no_null` — инвариант «null на экран не попадает» держится здесь, а не
/// в каждом роуте по отдельности.
pub fn state_response(st: &Value, live: &Live) -> Value {
    let mut out = json!({
        "ok": true,
        "api_version": API_VERSION,
        "version": st.get("version").cloned().unwrap_or(json!(3)),
        "active": st.get("active").cloned().unwrap_or(json!("")),
        "me_id": st.get("me_id").cloned().unwrap_or(json!(0)),
        "chat": st.get("chats").cloned().unwrap_or(json!({})),
        "ui": st.get("ui").cloned().unwrap_or(json!({"panel": ""})),
        "demon": live.snapshot(),
    });
    // chat: из chats{active} делаем один объект, как в Python-блобе
    if let (Some(active), Some(chats)) = (
        st.get("active").and_then(|v| v.as_str()),
        st.get("chats").and_then(|v| v.as_object()),
    ) {
        if let Some(c) = chats.get(active) {
            if let Some(o) = out.as_object_mut() {
                o.insert("chat".into(), c.clone());
            }
        }
    }
    shape::no_null(out)
}

// ────────────────────────────────────────────────────────── сервер
pub struct Config {
    pub addr: String,
    pub assets: PathBuf,
    pub state: PathBuf,
}

/// Один запрос → (статус, content-type, тело). Чистая функция: вся сеть снаружи,
/// поэтому маршруты и формы ответов тестируются без портов и без сна.
pub fn route(cfg: &Config, live: &Live, method: &str, path: &str, body: &str)
    -> (u16, &'static str, Vec<u8>)
{
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => file(&cfg.assets, "index.html"),
        ("GET", p) if p.starts_with("/ui/") => {
            let name = &p[4..];
            match ui_path(&cfg.assets, name) {
                Ok(full) => serve_file(&full, name),
                Err(e) => err(403, e),
            }
        }
        ("GET", "/api/state") => {
            match read_state(&cfg.state) {
                Ok(st) => json(200, &state_response(&st, live)),
                Err(e) => err(500, e),
            }
        }
        ("GET", "/api/version") => json(200, &json!({"ok": true, "api_version": API_VERSION})),
        ("POST", "/api/live") => mutate(cfg, live, body, |st, chat, _| {
            // pick_bool отдаёт Option (в теле может не быть ключа) — в замыкании,
            // возвращающем Result, `?` над Option не компилируется (E0277): превращаем
            // отсутствие ключа в явную ошибку, а не в «молча false».
            let enabled = pick_bool(body, "enabled")
                .ok_or_else(|| ApiError::Bad("нет поля enabled".into()))?;
            // now = 0: движок сам проставит базис при включении (у API нет честных часов
            // процесса движка, а выдумывать время — значит врать в F4-счёте).
            set_live_enabled(st, chat, enabled, 0)
        }),
        ("POST", "/api/targets") => mutate(cfg, live, body, |st, chat, _| {
            let ids = pick_ids(body);
            let me = read_state(&cfg.state)
                .ok()
                .and_then(|s| s.get("me_id").and_then(|v| v.as_i64()))
                .unwrap_or(0);
            set_targets(st, chat, &ids, me)
        }),
        ("POST", "/api/mode") => mutate(cfg, live, body, |st, chat, _| set_mode(st, chat, &pick_str(body, "mode"))),
        ("POST", "/api/watch") => mutate(cfg, live, body, |st, chat, _| {
            let key = pick_str(body, "key");
            let on = pick_bool(body, "on").unwrap_or(false);
            set_watch(st, chat, &key, on)
        }),
        ("POST", "/api/ui") => mutate(cfg, live, body, |st, _, _| set_panel(st, &pick_str(body, "panel"))),
        // arm/disarm — только терминал с nonce (правило юзера [Н]).
        ("POST", "/api/arm") | ("POST", "/api/disarm") => err(
            403,
            ApiError::Forbidden("автовозврат оформления включается только в терминале".into()),
        ),
        _ => err(404, ApiError::Bad(format!("нет маршрута {method} {path}"))),
    }
}

fn mutate<F>(cfg: &Config, live: &Live, body: &str, f: F) -> (u16, &'static str, Vec<u8>)
where
    F: FnOnce(&mut Value, &str, &str) -> Result<(), ApiError>,
{
    let mut st = match read_state(&cfg.state) {
        Ok(v) => v,
        Err(e) => return err(500, e),
    };
    let chat = match st.get("active").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => return err(400, ApiError::Bad("нет активного чата".into())),
    };
    if let Err(e) = f(&mut st, &chat, body) {
        return err(400, e);
    }
    if let Err(e) = write_state(&cfg.state, &st) {
        return err(500, e);
    }
    // ответ строим на РЕАЛЬНЫХ счётчиках движка: `Live::default()` здесь показал бы
    // «удалено 0, движок не активен» после успешной мутации — то есть соврал бы UI.
    json(200, &state_response(&st, live))
}

fn pick_bool(body: &str, key: &str) -> Option<bool> {
    serde_json::from_str::<Value>(body).ok()?.get(key)?.as_bool()
}

fn pick_str(body: &str, key: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get(key).and_then(|x| x.as_str()).map(str::to_string))
        .unwrap_or_default()
}

fn pick_ids(body: &str) -> Vec<i64> {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("ids").and_then(|x| x.as_array()).cloned())
        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
        .unwrap_or_default()
}

fn json(status: u16, v: &Value) -> (u16, &'static str, Vec<u8>) {
    (status, "application/json", v.to_string().into_bytes())
}

fn err(status: u16, e: ApiError) -> (u16, &'static str, Vec<u8>) {
    json(status, &shape::no_null(json!({"ok": false, "error": e.to_string()})))
}

fn file(assets: &Path, name: &str) -> (u16, &'static str, Vec<u8>) {
    match ui_path(assets, name) {
        Ok(p) => serve_file(&p, name),
        Err(e) => err(403, e),
    }
}

fn serve_file(p: &Path, name: &str) -> (u16, &'static str, Vec<u8>) {
    let ct = if name.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if name.ends_with(".css") {
        "text/css; charset=utf-8"
    } else {
        "text/javascript; charset=utf-8"
    };
    match std::fs::read(p) {
        Ok(b) => (200, ct, b),
        Err(_) => err(404, ApiError::Bad(format!("нет файла {name}"))),
    }
}

/// Поднять сервер. Блокирует до `SIGNAL`/остановки: вызывается из потока APK.
pub fn serve(cfg: Config, live: Arc<Live>, stop: Arc<std::sync::atomic::AtomicBool>)
    -> Result<(), ApiError>
{
    let server = Server::http(cfg.addr.as_str())
        .map_err(|e| ApiError::Io(format!("bind {}: {e}", cfg.addr)))?;
    // `as_reader(&mut self)` → запрос обязан быть mutable (E0596)
    for mut request in server.incoming_requests() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let method = request.method().as_str().to_string();
        let url = request.url().to_string();
        let path = url.split('?').next().unwrap_or("/").to_string();
        let mut body = String::new();
        let _ = request.as_reader().read_to_string(&mut body);
        let (status, ct, payload) = route(&cfg, live.as_ref(), &method, &path, &body);
        let header = Header::from_bytes(&b"Content-Type"[..], ct.as_bytes())
            // tiny_http возвращает Result<Header, ()> — у ошибки НЕТ Display (E0277),
            // поэтому {:?}", а не {e}.
            .map_err(|_| ApiError::Io("header: Content-Type отклонён".into()))?;
        let resp = Response::from_data(payload)
            .with_status_code(StatusCode(status))
            .with_header(header);
        let _ = request.respond(resp);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("purge_api_{name}"));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn cfg(name: &str) -> Config {
        let dir = tmp_dir(name);
        std::fs::write(dir.join("index.html"), b"<html>ui</html>").unwrap();
        std::fs::write(dir.join("app.js"), b"//js").unwrap();
        Config {
            addr: "127.0.0.1:0".into(),
            assets: dir.clone(),
            state: dir.join(STATE_FILE),
        }
    }

    fn seed(c: &Config, chat: &str, me: i64) {
        let mut st = json!({"version": 3, "me_id": me, "active": chat, "chats": {}});
        set_targets(&mut st, chat, &[me - 1], me).unwrap();
        write_state(&c.state, &st).unwrap();
    }

    #[test]
    fn ui_white_list_blocks_traversal() {
        let c = cfg("wl");
        for bad in ["../state.json", "index.html/", "..%2f", "", "purge.session.json"] {
            assert!(ui_path(&c.assets, bad).is_err(), "пропустил {bad}");
        }
        assert!(ui_path(&c.assets, "index.html").is_ok());
        assert!(ui_path(&c.assets, "strings.js").is_ok());
    }

    #[test]
    fn route_ui_traversal_is_403_not_file() {
        let c = cfg("trav");
        let (st, _, body) = route(&c, &Live::default(), "GET", "/ui/../state.json", "");
        assert_eq!(st, 403);
        assert!(!String::from_utf8_lossy(&body).contains("version"), "вытащили state.json");
    }

    #[test]
    fn state_response_has_no_null() {
        let c = cfg("nonull");
        seed(&c, "-1001", 5804685716);
        let (st, _, body) = route(&c, &Live::default(), "GET", "/api/state", "");
        assert_eq!(st, 200);
        let s = String::from_utf8_lossy(&body).to_string();
        assert!(!s.contains(": null"), "null уехал на экран: {s}");
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["api_version"], API_VERSION as u64);
    }

    #[test]
    fn live_enable_sets_since_only_when_absent() {
        let mut st = json!({"version":3,"me_id":7,"active":"c1","chats":{}});
        set_live_enabled(&mut st, "c1", true, 1000).unwrap();
        assert_eq!(st["chats"]["c1"]["live"]["enabled"], true);
        assert_eq!(st["chats"]["c1"]["live"]["since"], 1000);
        // уже выставленный базис НЕ перетирается (иначе F4 теряет старые простои)
        set_live_enabled(&mut st, "c1", true, 2000).unwrap();
        assert_eq!(st["chats"]["c1"]["live"]["since"], 1000);
        // выключение базис не трогает
        set_live_enabled(&mut st, "c1", false, 3000).unwrap();
        assert_eq!(st["chats"]["c1"]["live"]["enabled"], false);
        assert_eq!(st["chats"]["c1"]["live"]["since"], 1000);
    }

    #[test]
    fn live_enable_without_clock_is_error() {
        let mut st = json!({"version":3,"active":"c1","chats":{}});
        assert!(matches!(
            set_live_enabled(&mut st, "c1", true, 0),
            Err(ApiError::Bad(_))
        ), "нулевое время молча принято — F4 получит пустой базис");
    }

    #[test]
    fn targets_reject_self() {
        let mut st = json!({"version":3,"active":"c1","chats":{}});
        assert!(matches!(
            set_targets(&mut st, "c1", &[7, 9], 7),
            Err(ApiError::Forbidden(_))
        ));
    }

    #[test]
    fn targets_dedup_and_sort() {
        let mut st = json!({"version":3,"active":"c1","chats":{}});
        set_targets(&mut st, "c1", &[9, 3, 9, 3], 7).unwrap();
        assert_eq!(st["chats"]["c1"]["live"]["targets"], json!([3, 9]));
    }

    #[test]
    fn mode_rejects_unknown() {
        let mut st = json!({"version":3,"active":"c1","chats":{}});
        assert!(set_mode(&mut st, "c1", "all").is_ok());
        assert!(set_mode(&mut st, "c1", "nuke").is_err());
    }

    #[test]
    fn watch_rejects_unknown_key_and_never_arms() {
        let mut st = json!({"version":3,"active":"c1","chats":{}});
        set_watch(&mut st, "c1", "about", true).unwrap();
        assert_eq!(st["chats"]["c1"]["watch"]["about"], true);
        assert!(set_watch(&mut st, "c1", "nuke", true).is_err());
        // armed не появляется ни от какой мутации API
        assert!(st["chats"]["c1"].get("refs").is_none(), "API тронул оформление");
    }

    #[test]
    fn arm_via_api_is_forbidden() {
        let c = cfg("arm");
        seed(&c, "c1", 7);
        for p in ["/api/arm", "/api/disarm"] {
            let (st, _, body) = route(&c, &Live::default(), "POST", p, "{\"kind\":\"about\"}");
            assert_eq!(st, 403, "{p} вернул {st}");
            assert!(String::from_utf8_lossy(&body).contains("только в терминале"));
        }
        // и в файле ничего не изменилось
        let s = read_state(&c.state).unwrap();
        assert!(s["chats"]["c1"].get("refs").is_none());
    }

    #[test]
    fn panel_roundtrip_and_rejection() {
        let mut st = json!({"version":3});
        set_panel(&mut st, "who").unwrap();
        assert_eq!(st["ui"]["panel"], "who");
        assert!(set_panel(&mut st, "nonsense").is_err());
        set_panel(&mut st, "").unwrap();
        assert_eq!(st["ui"]["panel"], "");
    }

    #[test]
    fn unknown_route_is_404_not_200() {
        let c = cfg("r404");
        let (st, _, _) = route(&c, &Live::default(), "GET", "/api/nope", "");
        assert_eq!(st, 404);
        let (st, _, _) = route(&c, &Live::default(), "DELETE", "/api/state", "");
        assert_eq!(st, 404);
    }

    #[test]
    fn no_active_chat_is_400_not_silent() {
        let c = cfg("nochat");
        let (st, _, body) = route(&c, &Live::default(), "POST", "/api/live", "{\"enabled\":true}");
        assert_eq!(st, 400);
        assert!(String::from_utf8_lossy(&body).contains("активного чата"));
    }

    #[test]
    fn write_state_is_atomic_no_tmp_left() {
        let c = cfg("atomic");
        let mut st = json!({"version":3,"active":"c1","chats":{}});
        set_mode(&mut st, "c1", "new").unwrap();
        write_state(&c.state, &st).unwrap();
        assert!(c.state.exists());
        assert!(!c.state.with_extension("json.tmp").exists(), "tmp-файл остался");
    }

    #[test]
    fn live_snapshot_reports_engine_down() {
        // Пока движок не авторизован (гейт R2), running = 0: UI обязан показывать
        // «движок не активен», а не «удалено 0» — иначе юзер решит, что чистка работает.
        let live = Live::default();
        let v = live.snapshot();
        assert_eq!(v["engine_running"], false);
        assert_eq!(v["deleted"], 0);
    }
}
