//! ГЕЙТ R1/R3: компиляционные доказательства того, что grammers 0.12.0 умеет нужное.
//!
//! Здесь НЕТ сетевых вызовов. Смысл: если строка компилируется — тип/метод
//! существует в опубликованной версии и на него можно опираться. Не компилируется —
//! это ответ гейта, полученный за 2 минуты CI, а не после дня написания движка.
//!
//! Все формы вызовов взяты ИЗ ИСХОДНИКОВ grammers 0.12.0 (скачаны в vendor/,
//! в git не попадают). Ниже — точные строки из них, чтобы исполнитель не гадал:
//!   client/net.rs:49        pub fn new(sender_pool: SenderPoolFatHandle) -> Self
//!   client/auth.rs:242      pub async fn request_login_code(&self, phone: &str, api_hash: &str)
//!                             -> Result<LoginToken, InvocationError>
//!   client/auth.rs:339      pub async fn sign_in(&self, token: &LoginToken, code: &str)
//!                             -> Result<User, SignInError>
//!   client/auth.rs:416      pub async fn check_password(&self, token: PasswordToken,
//!                             password: impl AsRef<[u8]>) -> Result<User, SignInError>
//!   client/auth.rs:23       SignInError { SignUpRequired, PasswordRequired(PasswordToken),
//!                             InvalidCode, InvalidPassword(PasswordToken), Other(InvocationError) }
//!   client/chats.rs:414     pub async fn get_me(&self) -> Result<User, InvocationError>
//!   client/dialogs.rs       DialogIter::next(&mut) -> Result<Option<Dialog>, InvocationError>
//!   client/messages.rs      MessageIter::next(&mut) -> Result<Option<Message>, InvocationError>
//!                             delete_messages<C: Into<PeerRef>>(peer: C, &[i32]) -> Result<usize, _>
//!   peer/dialog.rs:90       Dialog::peer(&self) -> &Peer
//!   peer/user.rs:103        User::id(&self) -> PeerId   (★ НЕ i64!)
//!   peer/user.rs            username()/first_name() -> Option<&str>, is_self() -> bool
//!   session/peer.rs         PeerId::{user_unchecked(i64), bare_id_unchecked(self) -> i64}
//!   session/types.rs        UpdatesState {pts,qts,date,seq: i32, channels: Vec<ChannelState>}
//!                             ChannelState {id: i64, pts: i32}
//!                             UpdateState {All, Primary{pts,date,seq}, Secondary{qts}, Channel{id,pts}}
//!   mtsender/sender_pool.rs SenderPool {runner, handle: SenderPoolFatHandle, updates}
//!                             SenderPoolFatHandle {thin, session: Arc<ErasedSession>, api_id: i32}
//!
//! Два провала гейта, найденные ДО написания движка (это и есть цель гейтов):
//!   1. `channels.editDescription` в LAYER 233 ОТСУТСТВУЕТ ⇒ возврат описания
//!      супергруппы на опубликованном grammers невозможен.
//!   2. `src/client/retry_policy.rs` существует, но `mod retry_policy;` в crate
//!      НИГДЕ не объявлен (grep по всему `src/`: совпадения только внутри самого
//!      файла) ⇒ политик повторных попыток в 0.12.0 нет, FLOOD_WAIT обязан
//!      обрабатывать сам движок.

#![allow(dead_code)]

use std::sync::Arc;

use grammers_client::message::Message;
use grammers_client::peer::{Dialog, Peer, User};
use grammers_client::session::types::{PeerId, PeerInfo};
use grammers_client::update::Update;
use grammers_client::{Client, SignInError};
use grammers_mtsender::{InvocationError, RpcError, SenderPool, SenderPoolFatHandle};
use grammers_session::storages::MemorySession;
use grammers_tl_types as tl;

// ---------------------------------------------------------------- авторизация
/// Путь входа пользователя (не бота) достижим целиком, включая 2FA
/// (у юзера пароль в env TG_2FA_PASSWORD, поэтому ветка PasswordRequired обязательна).
async fn auth_surface(client: &Client) {
    let _authorized: bool = client.is_authorized().await.unwrap_or(false);

    if let Ok(token) = client.request_login_code("+70000000000", "hash").await {
        match client.sign_in(&token, "12345").await {
            Ok(_user) => {}
            Err(SignInError::PasswordRequired(ptoken)) => {
                let _ = client.check_password(ptoken, "secret").await;
            }
            // все пять веток SignInError перечислены ЯВНО: если layer сменится и
            // появится шестая, компилятор обязан это поймать, а не проглотить через `_`.
            Err(SignInError::SignUpRequired) => {}
            Err(SignInError::InvalidCode) => {}
            Err(SignInError::InvalidPassword(_ptoken)) => {}
            Err(SignInError::Other(_e)) => {}
        }
    }
}

// ---------------------------------------------------------------- чтение
/// Диалоги, участники, права, история. `next()` отдаёт `Result<Option<T>, _>` —
/// НЕ вложенный Result (первая версия этого модуля писала `if let Ok(msg) = item`,
/// что компилировалось бы только как совпадение с внешним Ok, а внутри лежал Option:
/// тихая потеря всех сообщений. Здесь разбирается Option явно.
async fn read_surface(client: &Client) {
    let mut dialogs = client.iter_dialogs();
    let _total: usize = dialogs.total().await.unwrap_or(0);
    while let Some(dialog) = dialogs.next().await.unwrap_or(None) {
        let peer: &Peer = dialog.peer();
        let _pid: PeerId = peer.id();
        let _raw: i64 = peer.id().bare_id_unchecked();
        let _name: Option<&str> = peer.name();
    }

    let mut hist = client.iter_messages(peer_id_peer());
    let _n: usize = hist.limit(200).total().await.unwrap_or(0);
    while let Some(msg) = hist.next().await.unwrap_or(None) {
        let _id: i32 = msg.id();
        let _out: bool = msg.outgoing();
        let _text: &str = msg.text();
        let _peer = msg.peer_id();
    }

    let p = peer_id_peer();
    let _parts = client.iter_participants(&p).total().await;
    let _perms = client.get_permissions(&p).await;
    let _resolved = client.resolve_username("we_erti").await;
    let _me: Result<User, InvocationError> = client.get_me().await;
}

/// Небольшой helper, чтобы типы пиров были названы явно (PeerId → PeerRef через Into).
fn peer_id_peer() -> PeerId {
    PeerId::user_unchecked(5804685716)
}

// ---------------------------------------------------------------- удаление
/// ★ Фундамент честного отчёта: `delete_messages` возвращает `usize` — число
/// реально удалённых сообщений ОТ СЕРВЕРА, а не «сколько id отправили».
/// В Python-версии ровно это лечило U-D31 («ты сказал, что удалились, но они
/// не удалились»): Telegram вернул список короче пакета, счёт по ответу занижал.
///
/// ★ Осторожность из док-страницы `Message::id()`: для мегагрупп/каналов
/// `delete_messages` НЕ может проверить peer, откуда удаляется сообщение —
/// значит движок обязан сам держать правильный peer, а не полагаться на сервер.
async fn delete_surface<C: Into<grammers_client::session::types::PeerRef>>(
    client: &Client,
    peer: C,
) -> usize {
    let ids: &[i32] = &[1, 2, 3];
    match client.delete_messages(peer, ids).await {
        Ok(n) => n,
        Err(_) => 0,
    }
}

// ---------------------------------------------------------------- FloodWait
/// Провал гейта №2: `retry_policy` в 0.12.0 не подключён ⇒ паузу ставим сами.
/// Форма ошибки — из док-теста mtsender/errors.rs:
/// `FLOOD_WAIT_31` → `RpcError { code: 420, name: "FLOOD_WAIT", value: Some(31) }`.
pub fn flood_seconds(err: &InvocationError) -> Option<u64> {
    if let InvocationError::Rpc(RpcError { code: 420, value: Some(sec), .. }) = err {
        return Some(*sec as u64);
    }
    None
}

// ---------------------------------------------------------------- оформление
/// Проверено по tl/api.tl (LAYER 233):
///   channels.editTitle#566decd0  chat_id:long title:string
///   channels.editPhoto#f12e57c9  chat_id:long photo:InputChatPhoto
///   messages.editChatTitle#73783ffd / editChatPhoto#35ddd674 / editChatAbout#def60797
///   inputChatPhotoEmpty#1ca48f57 / inputChatUploadedPhoto#bdcdaec0 (flags: file/video/…)
/// Высокоуровневых обёрток нет ни в client/chats.rs, ни в messages.rs — но `invoke`
/// (net.rs:91) даёт их напрямую, типизированно, на этапе компиляции.
async fn decoration_surface(client: &Client, chat_id: i64) {
    let _ = client
        .invoke(tl::functions::channels::EditTitle {
            chat_id,
            title: String::new(),
        })
        .await;

    let _ = client
        .invoke(tl::functions::channels::EditPhoto {
            chat_id,
            photo: tl::enums::InputChatPhoto::Empty,
        })
        .await;

    // uploaded-эталон: inputChatUploadedPhoto#bdcdaec0 flags:# file:flags.0?InputFile …
    let photo = tl::enums::InputChatPhoto::Uploaded(tl::types::InputChatUploadedPhoto {
        file: Some(tl::enums::InputFile::Uploaded(tl::types::InputFileUploaded {
            id: 0,
            parts: 0,
            name: String::new(),
            md5_checksum: None,
        })),
        ..Default::default()
    });
    let _ = client
        .invoke(tl::functions::channels::EditPhoto { chat_id, photo })
        .await;

    let _ = client
        .invoke(tl::functions::messages::EditChatAbout {
            chat_id,
            about: String::new(),
        })
        .await;
}

/// Провал гейта №1: описание СУПЕРГРУППЫ. `channels.editDescription` в слое 233
/// нет (grep по `^channels\.edit(Description|Title|Photo)` → только editTitle/editPhoto;
/// поиск по ID конструктора `f8b0a74e` → 0 совпадений).
/// Варианты: (a) git-master grammers (layer 234, коммит 24.11.2026),
/// (b) TDLib через JSON, (c) оставить описание на Python-движке.
#[allow(dead_code)]
fn description_gap() -> &'static str {
    // tl::functions::channels::EditDescription { channel: .., about: None };
    "channels::EditDescription — ОТСУТСТВУЕТ в grammers 0.12.0 (layer 233)"
}

// ---------------------------------------------------------------- стикеры
/// messages.getStickers#d5a5d3a1 emoticon:string hash:long
/// messages.getStickerSet#c8a0ec74 stickerset:InputStickerSet hash:int
/// inputStickerSetShortName#861cc8a0 short_name:string = InputStickerSet
async fn sticker_surface(client: &Client) {
    let _ = client
        .invoke(tl::functions::messages::GetStickers {
            emoticon: String::new(),
            hash: 0,
        })
        .await;

    let _ = client
        .invoke(tl::functions::messages::GetStickerSet {
            stickerset: tl::enums::InputStickerSet::ShortName(tl::types::InputStickerSetShortName {
                short_name: String::new(),
            }),
            hash: 0,
        })
        .await;
}

// ---------------------------------------------------------------- файлы
async fn file_surface(client: &Client, local: &str) {
    let _uploaded = client.upload_file(local).await;
}

// ---------------------------------------------------------------- апдейты
/// Антицикличный инвариант: `Message::outgoing()` есть ⇒ свои события
/// отсекаем всегда (иначе возврат аватарки сам вызывает NEW_PHOTO → цикл замены).
fn update_surface(msg: Message) {
    match Update::NewMessage(msg) {
        Update::NewMessage(message) => {
            if message.outgoing() {
                return;
            }
            let _id: i32 = message.id();
            let _peer = message.peer_id();
            let _text: &str = message.text();
        }
        _ => {}
    }
}

/// Сессия без C-зависимостей. `MemorySession` — `#[derive(Default)]`
/// (storages/memory.rs), а `sqlite-storage` в grammers-session входит в
/// default-features ⇒ в Cargo.toml он выключен (`default-features = false`),
/// иначе rusqlite потребует C-компилятор на Android.
///
/// ★ Док grammers предупреждает: MemorySession сам НЕ сохраняет состояние,
/// а «логин очень дорог по flood wait» ⇒ сохранение сессии — наша фаза Ф-R2
/// (`crate::store::FileSession`).
///
/// `Client::new` ПОТРЕБЛЯЕТ fat-handle (поле session/api_id читаются из него),
/// поэтому держим копию отдельно: `handle.clone()` до передачи.
pub fn pool_typechecks(api_id: i32) {
    let session = Arc::new(MemorySession::default());
    let SenderPool { runner, handle, updates } = SenderPool::new(Arc::clone(&session), api_id);
    let fat: SenderPoolFatHandle = handle;
    let _client = Client::new(fat.clone());
    let _thin = fat.thin;
    let _ = (runner, updates);
}

/// Человекочитаемый список для `--caps` (печатает CI).
/// false = гейт НЕ пройден, и это находка ДО написания движка.
pub fn capabilities() -> Vec<(&'static str, &'static str, bool)> {
    vec![
        ("вход user + 2FA", "auth.rs: is_authorized/request_login_code/sign_in/check_password", true),
        ("чтение: диалоги/участники/права/история", "iter_dialogs / iter_participants / get_permissions / iter_messages", true),
        ("пакетное удаление со счётом сервера", "delete_messages(&[i32]) -> usize", true),
        ("антицикличный признак", "Message::outgoing() / Update::NewMessage", true),
        ("оформление: название группы/канала", "invoke(channels::EditTitle#566decd0)", true),
        ("оформление: фото", "invoke(channels::EditPhoto#f12e57c9 + inputChatUploadedPhoto)", true),
        ("оформление: описание базовой группы", "invoke(messages::EditChatAbout#def60797)", true),
        ("оформление: описание СУПЕРГРУППЫ", "channels::EditDescription", false),
        ("FloodWait: пауза силами движка", "retry_policy.rs есть, но `mod retry_policy` не объявлен", false),
        ("стикеры", "invoke(messages::GetStickers / GetStickerSet + inputStickerSetShortName)", true),
        ("файлы", "upload_file", true),
        ("сессия без C-зависимостей", "MemorySession + default-features=false (нет rusqlite)", true),
    ]
}
