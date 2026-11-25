//! ГЕЙТ R1/R3: компиляционные доказательства того, что grammers 0.12.0 умеет нужное.
//!
//! Здесь НЕТ сетевых вызовов. Смысл: если строка компилируется — тип/метод
//! существует в опубликованной версии и на него можно опираться. Не компилируется —
//! это ответ гейта, полученный за 2 минуты CI, а не после дня написания движка.
//!
//! Раунд 6 дал 23 ошибки компилятора — и это первый раз, когда я получил НАСТОЯЩИЕ
//! ответы вместо своих догадок. Все формы ниже теперь сверены по исходникам
//! vendor/ (grammers-client/-session/-tl-types 0.12.0), строка в строку:
//!
//!   session/peer.rs:458-705  From<tl::enums::InputPeer> / From<tl::types::InputPeerSelf>
//!                            / From<tl::types::Channel> / … → PeerRef.
//!                            ★ From<PeerId> для PeerRef НЕТ: только
//!                              `PeerId::to_ambient_ref()` (peer.rs:302).
//!   session/peer.rs:50       PeerRef { id: PeerId, auth: PeerAuth }  (auth не Option)
//!   client/chats.rs:686      get_permissions<C: Into<PeerRef>, U: Into<PeerRef>>(chat, user)
//!                            ★ ДВА аргумента, не один.
//!   client/chats.rs:450      iter_participants<C: Into<PeerRef>>(chat) -> ParticipantIter
//!   client/chats.rs:625      resolve_peer<C: Into<PeerRef>>(peer) -> Result<Peer, _>
//!   client/chats.rs:414      get_me(&self) -> Result<User, InvocationError>   (async)
//!   client/messages.rs       delete_messages<C: Into<PeerRef>>(peer: C, &[i32]) -> Result<usize, _>
//!   peer/dialog.rs:82        Dialog::peer_ref(&self) -> PeerRef ; :90 peer() -> &Peer
//!   peer/user.rs:103         User::id(&self) -> PeerId (★ НЕ i64), :161 username() -> Option<&str>
//!   update/mod.rs:28         pub use message::Message → Update::NewMessage(grammers_client::update::Message)
//!                            ★ это ДРУГОЙ тип, чем grammers_client::message::Message,
//!                              который отдаёт iter_messages.
//!   net.rs:91                invoke<R: RemoteCall>(&self, request: &R)  ★ по ссылке
//!
//! TL-конструкторы (grammers-tl-types-0.12.0/tl/api.tl, LAYER 233):
//!   channels.editTitle#566decd0     channel:InputChannel title:string        = Updates
//!   channels.editPhoto#f12e57c9     channel:InputChannel photo:InputChatPhoto = Updates
//!   messages.editChatAbout#def60797 peer:InputPeer about:string             = Bool
//!   messages.editChatTitle#73783ffd chat_id:long title:string               = Updates
//!   messages.getStickers#d5a5d3a1   emoticon:string hash:long               = messages.Stickers
//!   messages.getStickerSet#c8a0ec74 stickerset:InputStickerSet hash:int     = messages.StickerSet
//!   inputChannel#f35aec28           channel_id:long access_hash:long        = InputChannel
//!   inputChannelEmpty#ee8c1e86                                               = InputChannel
//!   inputChatPhotoEmpty#1ca48f57                                             = InputChatPhoto
//!   inputChatUploadedPhoto#bdcdaec0 flags:# file:flags.0?InputFile video:flags.1?InputFile
//!                               video_start_ts:flags.2?double video_emoji_markup:flags.3?VideoSize
//!   inputFile#f52ff27f          id:long parts:int name:string md5_checksum:string = InputFile
//!   inputFileBig#fa4f0bb5       id:long parts:int name:string                     = InputFile
//!   inputPeerSelf#7da07ec9                                                          = InputPeer
//!   inputStickerSetShortName#861cc8a0 short_name:string                           = InputStickerSet
//!   ★ `inputFileUploaded` НЕ существует (раунд 6: E0422) — есть inputFile/inputFileBig.
//!   ★ `channels.editDescription` в слое 233 НЕ существует — провал гейта №1.
//!
//! Провалы гейта, найденные ДО боевого кода (это и есть цель спайка):
//!   1. описание супергруппы редактировать нечем → git-master (layer 234) / TDLib / Python;
//!   2. `retry_policy.rs` есть в дереве, но `mod retry_policy;` нигде не объявлен
//!      (grep по всему src/: совпадения только внутри самого файла) ⇒ FLOOD_WAIT
//!      обрабатывает только наш движок.

#![allow(dead_code)]

use std::sync::Arc;

use grammers_client::peer::{Peer, User};
use grammers_client::session::types::{PeerId, PeerInfo, PeerRef};
use grammers_client::update::Update;
use grammers_client::{Client, SignInError};
use grammers_mtsender::{InvocationError, RpcError, SenderPool, SenderPoolFatHandle};
use grammers_session::storages::MemorySession;
use grammers_tl_types as tl;

// ---------------------------------------------------------------- авторизация
/// Путь входа пользователя (не бота) достижим целиком, включая 2FA
/// (у юзера пароль в env TG_2FA_PASSWORD, поэтому ветка PasswordRequired обязательна).
/// Все пять веток SignInError перечислены ЯВНО: `_ =>` скрыл бы появление шестой.
async fn auth_surface(client: &Client) {
    let _authorized: bool = client.is_authorized().await.unwrap_or(false);

    if let Ok(token) = client.request_login_code("+70000000000", "hash").await {
        match client.sign_in(&token, "12345").await {
            Ok(_user) => {}
            Err(SignInError::PasswordRequired(ptoken)) => {
                let _ = client.check_password(ptoken, "secret").await;
            }
            Err(SignInError::SignUpRequired) => {}
            Err(SignInError::InvalidCode) => {}
            Err(SignInError::InvalidPassword(_ptoken)) => {}
            Err(SignInError::Other(_e)) => {}
        }
    }
}

// ---------------------------------------------------------------- чтение
/// Диалоги, участники, права, история.
/// `next()` отдаёт `Result<Option<T>, _>` — разбирается Option явно; первая версия
/// модуля писала `if let Ok(msg) = item`, что компилировалось бы как совпадение с
/// внешним Ok, а внутри лежал Option: тихая потеря ВСЕХ сообщений.
async fn read_surface(client: &Client) {
    let mut me_ref: Option<PeerRef> = None;
    let mut dialogs = client.iter_dialogs();
    let _total: usize = dialogs.total().await.unwrap_or(0);
    while let Some(dialog) = dialogs.next().await.unwrap_or(None) {
        let peer: &Peer = dialog.peer();
        let _pid: PeerId = peer.id();
        let _raw: i64 = peer.id().bare_id_unchecked();
        let _name: Option<&str> = peer.name();
        let chat: PeerRef = dialog.peer_ref();
        if me_ref.is_none() {
            me_ref = Some(chat.clone());
        }
    }

    // свой peer — через PeerId::self_user().to_ambient_ref(): From<PeerId> для PeerRef нет
    let myself = PeerId::self_user().to_ambient_ref();
    let _me: Result<User, InvocationError> = client.get_me().await;
    let _resolved = client.resolve_username("we_erti").await;

    if let Some(chat) = me_ref {
        let mut hist = client.iter_messages(chat.clone());
        let _n: usize = hist.limit(200).total().await.unwrap_or(0);
        while let Some(msg) = hist.next().await.unwrap_or(None) {
            let _id: i32 = msg.id();
            let _out: bool = msg.outgoing();
            let _text: &str = msg.text();
            let _peer = msg.peer_id();
        }
        let _parts = client.iter_participants(chat.clone()).total().await;
        // ★ get_permissions принимает ДВА аргумента: chat и user (chats.rs:686)
        let _perms = client.get_permissions(chat, myself).await;
    }
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
async fn delete_surface(client: &Client, chat: PeerRef) -> usize {
    let ids: &[i32] = &[1, 2, 3];
    match client.delete_messages(chat, ids).await {
        Ok(n) => n,
        Err(_) => 0,
    }
}

// ---------------------------------------------------------------- FloodWait
/// Провал гейта №2: `retry_policy` в 0.12.0 не подключён ⇒ паузу ставим сами.
/// Форма ошибки — док-тест mtsender/errors.rs: `FLOOD_WAIT_31` →
/// `RpcError { code: 420, name: "FLOOD_WAIT", value: Some(31) }` (value: Option<u32>).
pub fn flood_seconds(err: &InvocationError) -> Option<u64> {
    if let InvocationError::Rpc(RpcError { code: 420, value: Some(sec), .. }) = err {
        return Some(u64::from(*sec));
    }
    None
}

// ---------------------------------------------------------------- оформление
/// Название и фото супергруппы/канала — через `invoke` (высокоуровневых обёрток нет).
/// ★ `invoke` принимает ЗАПРОС ПО ССЫЛКЕ (net.rs:91), поэтому держим локальную переменную.
/// ★ Поле называется `channel: InputChannel`, а не `chat_id` (раунд 6: E0560).
async fn decoration_surface(client: &Client, channel_id: i64, access_hash: i64) {
    let channel = tl::enums::InputChannel::InputChannel(tl::types::input_channel::InputChannel {
        channel_id,
        access_hash,
    });

    let title = tl::functions::channels::EditTitle {
        channel: channel.clone(),
        title: String::new(),
    };
    let _ = client.invoke(&title).await;

    let clear = tl::functions::channels::EditPhoto {
        channel: channel.clone(),
        photo: tl::enums::InputChatPhoto::Empty(tl::types::input_chat_photo::InputChatPhotoEmpty),
    };
    let _ = client.invoke(&clear).await;

    // uploaded-эталон: inputChatUploadedPhoto#bdcdaec0 flags:# file:flags.0?InputFile …
    // ★ struct НЕ выводит Default (раунд 6: E0277) — все четыре поля названы явно.
    let photo = tl::enums::InputChatPhoto::Uploaded(tl::types::input_chat_uploaded_photo::InputChatUploadedPhoto {
        file: Some(tl::enums::InputFile::Big(tl::types::input_file::input_file_big::InputFileBig {
            id: 0,
            parts: 0,
            name: String::new(),
        })),
        video: None,
        video_start_ts: None,
        video_emoji_markup: None,
    });
    let up = tl::functions::channels::EditPhoto { channel, photo };
    let _ = client.invoke(&up).await;

    // описание БАЗОВОЙ группы: messages.editChatAbout#def60797 peer:InputPeer about:string
    let peer = tl::enums::InputPeer::Self_(tl::types::input_peer::input_peer_self::InputPeerSelf);
    let about = tl::functions::messages::EditChatAbout {
        peer,
        about: String::new(),
    };
    let _ = client.invoke(&about).await;

    // название базовой группы: messages.editChatTitle#73783ffd chat_id:long title:string
    let bt = tl::functions::messages::EditChatTitle {
        chat_id: channel_id,
        title: String::new(),
    };
    let _ = client.invoke(&bt).await;
}

/// Провал гейта №1: описание СУПЕРГРУППЫ. `channels.editDescription` в слое 233
/// нет (grep по `^channels\.editDescription#` → 0; поиск по ID `f8b0a74e` → 0).
/// Варианты: (a) git-master grammers (layer 234, коммит 24.11.2026),
/// (b) TDLib через JSON, (c) оставить описание на Python-движке.
#[allow(dead_code)]
fn description_gap() -> &'static str {
    // tl::functions::channels::EditDescription { .. };   ← такого типа нет
    "channels::EditDescription — ОТСУТСТВУЕТ в grammers 0.12.0 (layer 233)"
}

// ---------------------------------------------------------------- стикеры
/// messages.getStickers#d5a5d3a1 emoticon:string hash:long
/// messages.getStickerSet#c8a0ec74 stickerset:InputStickerSet hash:int
/// inputStickerSetShortName#861cc8a0 short_name:string = InputStickerSet
async fn sticker_surface(client: &Client) {
    let list = tl::functions::messages::GetStickers {
        emoticon: String::new(),
        hash: 0,
    };
    let _ = client.invoke(&list).await;

    let one = tl::functions::messages::GetStickerSet {
        stickerset: tl::enums::InputStickerSet::ShortName(
            tl::types::input_sticker_set::input_sticker_set_short_name::InputStickerSetShortName {
                short_name: String::new(),
            },
        ),
        hash: 0,
    };
    let _ = client.invoke(&one).await;
}

// ---------------------------------------------------------------- файлы
async fn file_surface(client: &Client, local: &str) {
    let _uploaded = client.upload_file(local).await;
}

// ---------------------------------------------------------------- апдейты
/// Антицикличный инвариант: `Message::outgoing()` есть ⇒ свои события
/// отсекаем всегда (иначе возврат аватарки сам вызывает NEW_PHOTO → цикл замены).
///
/// ★ Это `grammers_client::update::Message` (update/mod.rs:28 re-export), а НЕ тот
/// `message::Message`, который отдаёт `iter_messages` (раунд 6: E0308).
fn update_surface(msg: grammers_client::update::Message) {
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
/// `Client::new` ПОТРЕБЛЯЕТ fat-handle (читает из него session/api_id),
/// поэтому копируем до передачи.
pub fn pool_typechecks(api_id: i32) {
    let session = Arc::new(MemorySession::default());
    let SenderPool { runner, handle, updates } = SenderPool::new(Arc::clone(&session), api_id);
    let fat: SenderPoolFatHandle = handle;
    let _client = Client::new(fat.clone());
    let _thin = fat.thin;
    let _ = (runner, updates);
}

/// Проверка, что `PeerInfo` вообще достижим из кода клиента (для Ф-R4:
/// цели и участники хранятся в сессии именно в этом виде).
fn peer_info_surface(info: PeerInfo) -> Option<i64> {
    match info {
        PeerInfo::User { id, .. } => Some(id),
        PeerInfo::Chat { id } => Some(id),
        PeerInfo::Channel { id, .. } => Some(id),
    }
}

/// Человекочитаемый список для `--caps` (печатает CI).
/// false = гейт НЕ пройден, и это находка ДО написания движка.
pub fn capabilities() -> Vec<(&'static str, &'static str, bool)> {
    vec![
        ("вход user + 2FA", "auth.rs: is_authorized/request_login_code/sign_in/check_password", true),
        ("чтение: диалоги/участники/права/история", "iter_dialogs / iter_participants / get_permissions(chat,user) / iter_messages", true),
        ("пакетное удаление со счётом сервера", "delete_messages(&[i32]) -> usize", true),
        ("антицикличный признак", "Message::outgoing() / Update::NewMessage", true),
        ("оформление: название группы/канала", "invoke(channels::EditTitle#566decd0 channel:InputChannel)", true),
        ("оформление: фото", "invoke(channels::EditPhoto#f12e57c9 + inputChatUploadedPhoto#bdcdaec0)", true),
        ("оформление: описание базовой группы", "invoke(messages::EditChatAbout#def60797 peer:InputPeer)", true),
        ("оформление: описание СУПЕРГРУППЫ", "channels::EditDescription", false),
        ("FloodWait: пауза силами движка", "retry_policy.rs есть, но `mod retry_policy` не объявлен", false),
        ("стикеры", "invoke(messages::GetStickers / GetStickerSet + inputStickerSetShortName)", true),
        ("файлы", "upload_file", true),
        ("сессия без C-зависимостей", "MemorySession + default-features=false (нет rusqlite)", true),
    ]
}
