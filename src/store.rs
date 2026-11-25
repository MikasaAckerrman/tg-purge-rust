//! Ф-R2 — хранилище сессии с НАСТОЯЩЕЙ персистентностью.
//!
//! Почему своё, а не `MemorySession`: это `#[derive(Default)] pub struct
//! MemorySession(Mutex<SessionData>)` с приватным полем и без публичного экспорта,
//! а собственная документация grammers говорит прямо: «Does not actually offer
//! direct ways to persist the state anywhere… Logging in has a very high cost in
//! terms of flood wait errors, so the state really should be persisted by other
//! means». Повторный вход — дорогая по flood-wait операция, терять сессию нельзя.
//!
//! Почему не `SqliteSession`: фича `sqlite-storage` в grammers-session входит в
//! default ⇒ тянет libsql (C-компиляция на Android). Вместо этого: `SessionData`
//! публичен и состоит из pub-полей, а внутренние типы имеют serde-derive под
//! фичей `serde`, который мы включаем в Cargo.toml унификацией фич графа
//! (у grammers-client собственного serde-passthrough нет: его features —
//! default=["fs"], html, markdown, parse_invite_link, proxy).
//!
//! Формы типов — ИЗ ИСХОДНИКОВ vendor/grammers-session-0.12.0/src (не из догадок):
//!   types.rs:22   DcOption { id: i32, ipv4, ipv6, auth_key: Option<[u8;256]> }
//!                 (serde_as, hex)
//!   types.rs:50   UpdatesState { pts, qts, date, seq: i32, channels: Vec<ChannelState> }
//!   types.rs:66   ChannelState { id: i64, pts: i32 }
//!   types.rs:75   UpdateState = All(UpdatesState) | Primary{pts,date,seq}
//!                          | Secondary{qts} | Channel{id,pts}     ← ровно 4 ветки
//!   peer.rs       PeerId::{user_unchecked(i64), bare_id_unchecked(self) -> i64}
//!                 PeerInfo::id(&self) -> PeerId
//!   session.rs    trait Session { type Error: Send; … 9 методов }
//!
//! Ключи HashMap: `PeerId` — newtype над i64 с приватным полем; как ключ JSON-карты
//! он сломал бы serde («key must be a string»). Поэтому на диск уходит Vec<PeerInfo>,
//! ключ восстанавливается через `PeerInfo::id()`. Для DcOption — его pub-поле `id`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use grammers_session::types::{ChannelState, DcOption, PeerId, PeerInfo, UpdatesState, UpdateState};
use grammers_session::{BoxFuture, Session, SessionData};
use serde::{Deserialize, Serialize};

/// Ошибка хранилища. Требуемый трейтом `Session::Error` обязан быть `Send`,
/// а `SenderPool::new` дополнительно требует `std::error::Error + Send + Sync + 'static`
/// (проверено: sender_pool.rs:250-256) — поэтому только текст, без вложенных типов.
#[derive(Debug)]
pub enum StoreError {
    /// Мьютекс отравлен (паника внутри критической секции).
    Poisoned,
    /// Диск/сериализация/версия формата.
    Io(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Poisoned => write!(f, "хранилище сессии: мьютекс отравлен"),
            StoreError::Io(msg) => write!(f, "хранилище сессии: {msg}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Формат на диске. Версия обязательна: схема grammers меняется вместе с layer,
/// и молча прочитать чужую версию хуже, чем отказать.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct Persisted {
    version: u32,
    home_dc: i32,
    dc_options: Vec<DcOption>,
    peers: Vec<PeerInfo>,
    updates_state: UpdatesState,
}

pub const FORMAT_VERSION: u32 = 1;

pub struct FileSession {
    data: Mutex<SessionData>,
    path: PathBuf,
    dirty: AtomicBool,
}

impl FileSession {
    /// Открыть: файл есть и версия совпадает — загрузить; иначе дефолт.
    /// Дефолт — ровно `SessionData::default()`, как требует документация трейта
    /// («a newly-created storage should return the same values that
    /// SessionData::default would produce»): 5 статических ДЦ, пустые peers,
    /// нулевой updates_state.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let data = if path.exists() {
            let raw = std::fs::read(&path).map_err(|e| StoreError::Io(format!("read: {e}")))?;
            let p: Persisted = serde_json::from_slice(&raw)
                .map_err(|e| StoreError::Io(format!("parse {}: {e}", path.display())))?;
            if p.version != FORMAT_VERSION {
                return Err(StoreError::Io(format!(
                    "формат сессии v{} ≠ ожидаемому v{} — не угадываем, нужна явная миграция",
                    p.version, FORMAT_VERSION
                )));
            }
            let mut d = SessionData::default();
            d.home_dc = p.home_dc;
            d.dc_options = p.dc_options.into_iter().map(|o| (o.id, o)).collect();
            d.peer_infos = p.peers.into_iter().map(|i| (i.id(), i)).collect();
            d.updates_state = p.updates_state;
            d
        } else {
            SessionData::default()
        };
        Ok(Self {
            data: Mutex::new(data),
            path,
            dirty: AtomicBool::new(false),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, SessionData>, StoreError> {
        self.data.lock().map_err(|_| StoreError::Poisoned)
    }

    fn snapshot(&self) -> Result<Persisted, StoreError> {
        let d = self.lock()?;
        Ok(Persisted {
            version: FORMAT_VERSION,
            home_dc: d.home_dc,
            dc_options: d.dc_options.values().cloned().collect(),
            peers: d.peer_infos.values().cloned().collect(),
            updates_state: d.updates_state.clone(),
        })
    }

    /// Записать атомарно: tmp + rename. Оборванная запись без rename убила бы сессию
    /// и заставила логиниться заново — то есть поймала бы ровно тот flood-wait,
    /// ради которого всё это и пишется.
    pub fn flush(&self) -> Result<(), StoreError> {
        let p = self.snapshot()?;
        let json = serde_json::to_vec_pretty(&p)
            .map_err(|e| StoreError::Io(format!("serialize: {e}")))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &json).map_err(|e| StoreError::Io(format!("write tmp: {e}")))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| StoreError::Io(format!("rename: {e}")))?;
        self.dirty.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Есть несохранённые изменения: движок зовёт `flush()` по таймеру и на остановке.
    pub fn dirty(&self) -> bool {
        self.dirty.load(Ordering::SeqCst)
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::SeqCst);
    }
}

impl Session for FileSession {
    type Error = StoreError;

    fn home_dc_id(&self) -> Result<i32, Self::Error> {
        Ok(self.lock()?.home_dc)
    }

    fn set_home_dc_id(&self, dc_id: i32) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            self.lock()?.home_dc = dc_id;
            self.mark_dirty();
            Ok(())
        })
    }

    fn dc_option(&self, dc_id: i32) -> Result<Option<DcOption>, Self::Error> {
        Ok(self.lock()?.dc_options.get(&dc_id).cloned())
    }

    fn set_dc_option(&self, dc_option: &DcOption) -> BoxFuture<'_, Result<(), Self::Error>> {
        // ★ Клони ДО async move: иначе будущий блок заимствует аргумент функции,
        // а трейт требует 'self (раунд 6: «method was supposed to return data
        // with lifetime '2 but it is returning data with lifetime '1»).
        let dc = dc_option.clone();
        Box::pin(async move {
            self.lock()?.dc_options.insert(dc.id, dc);
            self.mark_dirty();
            Ok(())
        })
    }

    fn peer(&self, peer: PeerId) -> BoxFuture<'_, Result<Option<PeerInfo>, Self::Error>> {
        Box::pin(async move { Ok(self.lock()?.peer_infos.get(&peer).cloned()) })
    }

    fn cache_peer(&self, peer: PeerInfo) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            let id = peer.id();
            self.lock()?.peer_infos.insert(id, peer);
            self.mark_dirty();
            Ok(())
        })
    }

    fn updates_state(&self) -> BoxFuture<'_, Result<UpdatesState, Self::Error>> {
        Box::pin(async move { Ok(self.lock()?.updates_state.clone()) })
    }

    /// Ровно 4 ветки `UpdateState` из types.rs 0.12.0 (All/Primary/Secondary/Channel),
    /// перечислены ЯВНО. `_ =>` здесь НЕ добавляем: если layer сменится и появится
    /// пятая ветка, компилятор обязан это поймать, а не проглотить.
    fn set_update_state(&self, update: UpdateState) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            let mut d = self.lock()?;
            match update {
                UpdateState::All(state) => d.updates_state = state,
                UpdateState::Primary { pts, date, seq } => {
                    d.updates_state.pts = pts;
                    d.updates_state.date = date;
                    d.updates_state.seq = seq;
                }
                UpdateState::Secondary { qts } => d.updates_state.qts = qts,
                UpdateState::Channel { id, pts } => {
                    match d.updates_state.channels.iter_mut().find(|c| c.id == id) {
                        Some(ch) => ch.pts = pts,
                        None => d.updates_state.channels.push(ChannelState { id, pts }),
                    }
                }
            }
            self.mark_dirty();
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("purge_store_test_{name}.json"));
        let _ = std::fs::remove_file(&p);
        p
    }

    /// Бокс-футеры трейта `Session` выполняем current-thread executor'ом:
    /// они не делают реальных await, но poll вручную с noop-waker — хрупкая
    /// самоделка, а tokio уже в зависимостях.
    fn block<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("executor для теста")
            .block_on(f)
    }

    #[test]
    fn open_without_file_is_default() {
        let s = FileSession::open(tmp("a")).unwrap();
        let d = s.lock().unwrap();
        // их собственный док-тест: data.dc_options.len() == 5, ДЦ 1..=5
        assert_eq!(d.dc_options.len(), 5, "нет статических ДЦ");
        assert!((1..=5).all(|id| d.dc_options.contains_key(&id)));
        assert!(d.peer_infos.is_empty());
        assert_eq!(d.updates_state.pts, 0);
        assert!(d.dc_options.values().all(|o| o.auth_key.is_none()));
    }

    #[test]
    fn flush_then_open_restores_everything() {
        let path = tmp("b");
        {
            let s = FileSession::open(&path).unwrap();
            block(s.set_home_dc_id(2)).unwrap();
            block(s.cache_peer(PeerInfo::User {
                id: 5804685716,
                // ★ auth: Option<PeerAuth> (peer.rs:81), поэтому Some(...)
                auth: Some(grammers_session::types::PeerAuth::from_hash(12345678901234)),
                bot: Some(false),
                is_self: Some(true),
            }))
            .unwrap();
            block(s.set_update_state(UpdateState::Primary { pts: 11, date: 22, seq: 33 })).unwrap();
            block(s.set_update_state(UpdateState::Channel { id: 3845264517, pts: 44 })).unwrap();
            assert!(s.dirty(), "после мутаций должен быть флаг несохранённости");
            s.flush().unwrap();
            assert!(!s.dirty());
        }
        let s2 = FileSession::open(&path).unwrap();
        assert_eq!(s2.home_dc_id().unwrap(), 2, "home_dc потерялся");
        let d = s2.lock().unwrap();
        let info = d
            .peer_infos
            .get(&PeerId::user_unchecked(5804685716))
            .expect("peer не восстановился");
        match info {
            PeerInfo::User { is_self, bot, .. } => {
                assert_eq!(*is_self, Some(true));
                assert_eq!(*bot, Some(false));
            }
            other => panic!("не тот PeerInfo: {other:?}"),
        }
        assert_eq!(d.updates_state.pts, 11);
        assert_eq!(d.updates_state.date, 22);
        assert_eq!(d.updates_state.seq, 33);
        assert_eq!(d.updates_state.channels.len(), 1);
        assert_eq!(d.updates_state.channels[0].id, 3845264517);
        assert_eq!(d.updates_state.channels[0].pts, 44);
    }

    #[test]
    fn secondary_and_all_branches_work() {
        let path = tmp("c");
        let s = FileSession::open(&path).unwrap();
        block(s.set_update_state(UpdateState::Secondary { qts: 7 })).unwrap();
        assert_eq!(s.lock().unwrap().updates_state.qts, 7);
        let full = UpdatesState {
            pts: 1,
            qts: 2,
            date: 3,
            seq: 4,
            channels: vec![ChannelState { id: 5, pts: 6 }],
        };
        block(s.set_update_state(UpdateState::All(full.clone()))).unwrap();
        assert_eq!(s.lock().unwrap().updates_state, full);
    }

    #[test]
    fn wrong_format_version_is_refused() {
        let path = tmp("d");
        std::fs::write(
            &path,
            r#"{"version":999,"home_dc":1,"dc_options":[],"peers":[],"updates_state":{"pts":0,"qts":0,"date":0,"seq":0,"channels":[]}}"#,
        ).unwrap();
        // ★ НЕ `.unwrap_err()`: у `Result::unwrap_err` bound `T: Debug`, а T здесь —
        //   FileSession (внутри Mutex<SessionData>, Debug у SessionData не выведен).
        //   Раунд 11 упал именно на этом: error[E0277] `FileSession` doesn't implement Debug.
        let e = match FileSession::open(&path) {
            Ok(_) => panic!("файл чужой версии принят — молча читать чужую схему нельзя"),
            Err(e) => e,
        };
        assert!(e.to_string().contains("формат сессии"), "не та ошибка: {e}");
    }

    #[test]
    fn corrupt_file_is_refused_not_defaulted() {
        // Молча заменить испорченную сессию дефолтом = потерять вход и поймать
        // flood-wait на повторном логине. Поэтому ошибка, а не «ну и ладно».
        let path = tmp("e");
        std::fs::write(&path, b"{ ne json ").unwrap();
        assert!(FileSession::open(&path).is_err());
    }

    #[test]
    fn no_partial_file_after_flush() {
        let path = tmp("f");
        let s = FileSession::open(&path).unwrap();
        s.flush().unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists(), "tmp-файл остался на диске");
    }

    #[test]
    fn auth_key_survives_roundtrip_as_hex() {
        // auth_key = [u8;256] сериализуется serde_with::hex; проверим, что
        // ключ ДЦ не теряется, иначе сессия «авторизована», но не соединяется.
        let path = tmp("g");
        let s = FileSession::open(&path).unwrap();
        let mut dc = s.lock().unwrap().dc_options.get(&1).cloned().unwrap();
        dc.auth_key = Some([9u8; 256]);
        block(s.set_dc_option(&dc)).unwrap();
        s.flush().unwrap();
        let s2 = FileSession::open(&path).unwrap();
        let got = s2.dc_option(1).unwrap().expect("ДЦ 1 пропал");
        assert_eq!(got.auth_key, Some([9u8; 256]), "auth_key не восстановился");
    }

    #[test]
    fn peer_id_roundtrip_via_bare_id() {
        // User::id() отдаёт PeerId, а не i64 — движок обязан уметь получать
        // «сырой» id для логов/аудита, не теряя тип.
        let pid = PeerId::user_unchecked(5804685716);
        assert_eq!(pid.bare_id_unchecked(), 5804685716);
        let ch = PeerId::channel_unchecked(3845264517);
        assert_eq!(ch.bare_id_unchecked(), 3845264517);
    }
}
