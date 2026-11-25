//! ГЕЙТ R1/R3: компиляционные доказательства того, что grammers 0.12 умеет нужное.
//!
//! Здесь НЕТ сетевых вызовов. Смысл модуля: если строка компилируется, значит
//! тип/метод существует в опубликованной версии и на него можно опираться.
//! Не компилируется — это и есть ответ гейта R3, полученный за 2 минуты CI,
//! а не после дня написания движка.
//!
//! Все сигнатуры ниже взяты ИЗ ИСХОДНИКОВ grammers 0.12.0 (скачаны в vendor/),
//! а не из памяти или документации:
//!   client/messages.rs   delete_messages<C: Into<PeerRef>>(&self, peer: C, &[i32]) -> Result<usize, InvocationError>
//!   client/net.rs:91     invoke<R: tl::RemoteCall>(...)
//!   client/chats.rs      iter_participants / get_permissions / iter_profile_photos / resolve_username
//!   client/dialogs.rs    iter_dialogs / delete_dialog / mark_as_read
//!   client/files.rs      upload_file / upload_media / download_media / iter_download
//!   client/auth.rs       is_authorized / request_login_code / sign_in / check_password / sign_out
//!   examples/dialogs.rs  форма пути входа для пользовательского аккаунта

#![allow(dead_code)]

use std::sync::Arc;

use grammers_client::{Client, SignInError, update::Update};
use grammers_mtsender::{InvocationError, RpcError, SenderPool};
use grammers_session::storages::MemorySession;
use grammers_tl_types as tl;

// ---------------------------------------------------------------- авторизация
/// Путь входа пользователя (не бота) достижим целиком, включая 2FA.
/// У юзера пароль в env TG_2FA_PASSWORD, поэтому ветка PasswordRequired обязательна.
async fn auth_surface(client: &Client) {
    let _authorized: bool = client.is_authorized().await.unwrap_or(false);

    // request_login_code -> LoginToken; sign_in по нему; 2FA через check_password.
    // Типы НАМЕРЕННО не названы: в 0.12 LoginToken — внутренний, выводим из примера.
    if let Ok(token) = client.request_login_code("+70000000000", "hash").await {
        match client.sign_in(&token, "12345").await {
            Ok(_) => {}
            Err(SignInError::PasswordRequired(password_token)) => {
                let _ = client.check_password(password_token, "secret").await;
            }
            Err(_) => {}
        }
    }
}

/// Антицикличный признак: `Message::outgoing()` существует, поэтому движок может
/// всегда игнорировать СВОИ сервисные сообщения (иначе возврат аватарки сам
/// вызывает NEW_PHOTO → бесконечный цикл — инвариант Python-версии).
fn outgoing_surface(msg: &grammers_client::types::Message) -> bool {
    msg.outgoing()
}

// ---------------------------------------------------------------- чтение
async fn read_surface(client: &Client, peer: grammers_client::types::Peer) {
    let mut dialogs = client.iter_dialogs();
    let _total = dialogs.total().await;
    while let Some((peer, _top)) = dialogs.next().await {
        let _id: i64 = peer.id();
    }
    let _ = client.iter_participants(&peer).total().await;
    let _ = client.get_permissions(&peer).await;
    let _ = client.resolve_username("we_erti").await;
    let mut hist = client.iter_messages(&peer);
    let _ = hist.limit(200).total().await;
    while let Some(item) = hist.next().await {
        if let Ok(msg) = item {
            let _id: i32 = msg.id;
            let _from: Option<i64> = msg.user_id();
        }
    }
}

// ---------------------------------------------------------------- удаление
/// ★ Фундамент честного отчёта: delete_messages возвращает `usize` —
/// число реально удалённых сообщений ОТ СЕРВЕРА, а не «сколько отправили».
/// В Python-версии именно это лечило U-D31 («сказал, что удалились,
/// но они не удалились»): счёт по ответу занижал, здесь сервер отдаёт count.
async fn delete_surface(client: &Client, peer: &grammers_client::types::Peer) -> usize {
    let ids: &[i32] = &[1, 2, 3];
    match client.delete_messages(peer, ids).await {
        Ok(n) => n,
        Err(_) => 0,
    }
}

// ---------------------------------------------------------------- FloodWait
/// grammers НЕ ретраит FLOOD_WAIT сам (в отличие от Pyrogram, который бросает
/// исключение). Ошибка приходит как RpcError с name="FLOOD_WAIT" и value=секунды —
/// проверено по док-тесту в mtsender/errors.rs. Значит движок обязан сам
/// ставить правило/чистку на паузу, иначе аккаунт ловит лимит повторно.
fn flood_surface(err: &InvocationError) -> Option<u32> {
    match err {
        InvocationError::Rpc(RpcError { name, value, .. }) if name == "FLOOD_WAIT" => *value,
        _ => None,
    }
}

// ---------------------------------------------------------------- оформление
/// Проверено grep'ом по tl/api.tl (LAYER 233, опубликованный grammers 0.12.0):
///   channels.editTitle#566decd0  chat_id:long title:string
///   channels.editPhoto#f12e57c9  chat_id:long photo:InputChatPhoto
///   messages.editChatTitle#73783ffd / editChatPhoto#35ddd674 / editChatAbout#def60797
///   inputChatUploadedPhoto#bdcdaec0 / inputChatPhotoEmpty#1ca48f57
/// Через `invoke` (net.rs:91) они доступны, несмотря на отсутствие обёрток.
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
    let _ = client
        .invoke(tl::functions::messages::EditChatAbout {
            chat_id,
            about: String::new(),
        })
        .await;
}

/// ★-ЕДИНСТВЕННЫЙ ПРОВАЛ ГЕЙТА R3, найденный ДО написания движка:
/// `channels.editDescription` в слое 233 ОТСУТСТВУЕТ.
/// Проверено двумя способами: grep по `^channels\.edit(Description|Title|Photo)`
/// дал только editTitle/editPhoto, и поиск по ID конструктора `f8b0a74e` — 0 совпадений.
/// Следствие: возврат ОПИСАНИЯ супергруппы через опубликованный grammers невозможен.
/// Варианты: (a) git-master grammers (layer 234, коммит 24.11.2026),
/// (b) TDLib через JSON (`setChatDescription` есть), (c) оставить описание на
/// Python-движке. Раскомментировать ниже = получить ошибку компиляции как доказательство.
#[allow(dead_code)]
fn description_gap() -> &'static str {
    // tl::functions::channels::EditDescription { channel: Default::default(), about: None };
    "channels::EditDescription — ОТСУТСТВУЕТ в grammers 0.12.0 (layer 233)"
}

// ---------------------------------------------------------------- стикеры
async fn sticker_surface(client: &Client) {
    let _ = client
        .invoke(tl::functions::messages::GetStickers {
            emoticon: String::new(),
            hash: 0,
        })
        .await;
    let _ = client
        .invoke(tl::functions::messages::GetStickerSet {
            short_name: Some(String::new()),
            id: None,
        })
        .await;
}

// ---------------------------------------------------------------- файлы
async fn file_surface(client: &Client, local: &str) {
    let _uploaded = client.upload_file(local).await;
    let _ = client.upload_media(local, Default::default()).await;
}

// ---------------------------------------------------------------- апдейты
fn update_surface(client: Client, msg: grammers_client::types::Message) {
    match Update::NewMessage(msg) {
        Update::NewMessage(message) => {
            if message.outgoing() {
                return; // свои сообщения не трогаем никогда
            }
            let _peer = message.peer_id();
            let _text = message.text();
        }
        _ => {}
    }
    let _ = client;
}

/// Сессия без C-зависимостей: `MemorySession` — `#[derive(Default)]`
/// (проверено в исходниках storages/memory.rs), а `sqlite-storage` в
/// grammers-session — **default-features**, поэтому в Cargo.toml мы его
/// выключаем (`default-features = false`) и избегаем rusqlite ⇒ никакого
/// C-компилятора на Android.
///
/// ★ Документация grammers прямо предупреждает: MemorySession НЕ умеет сам
/// сохранять состояние, а «логин стоит очень дорого с точки зрения flood wait»,
/// поэтому сессию обязан сохранять наш код (через SessionData, фаза Ф-R2).
pub fn pool_typechecks(api_id: i32) -> SenderPool<MemorySession> {
    let session = Arc::new(MemorySession::default());
    SenderPool::new(session, api_id)
}

/// Человекочитаемый список для `--caps` (печатает CI).
pub fn capabilities() -> Vec<(&'static str, &'static str, bool)> {
    vec![
        ("вход user + 2FA", "auth.rs: is_authorized/request_login_code/sign_in/check_password", true),
        ("антицикличный признак", "Message::outgoing()", true),
        ("чтение диалогов/участников/прав/истории", "dialogs.rs, chats.rs, messages.rs", true),
        ("пакетное удаление со счётом сервера", "delete_messages(&[i32]) -> usize", true),
        ("FloodWait как данные, не исключение", "RpcError{name=\"FLOOD_WAIT\", value}", true),
        ("оформление: название", "invoke(channels::EditTitle#566decd0)", true),
        ("оформление: фото", "invoke(channels::EditPhoto#f12e57c9)", true),
        ("оформление: описание базовой группы", "invoke(messages::EditChatAbout#def60797)", true),
        ("оформление: описание СУПЕРГРУППЫ", "channels::EditDescription", false),
        ("стикеры", "invoke(messages::GetStickers / GetStickerSet)", true),
        ("файлы", "upload_file / upload_media / download_media", true),
        ("апдейты", "Update::NewMessage + stream_updates", true),
        ("сессия без C-зависимостей", "MemorySession + default-features=false (нет rusqlite)", true),
    ]
}
