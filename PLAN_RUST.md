# PLAN RUST (v5) — движок на Rust внутри APK, TDLib/grammers-ворды
# Обновлено 25.11 2026 17:3x. Читается исполнителем БЕЗ доступа к чату.
# Это ЗАМЕНА §8 плана v3 (там Rust был «вариантом C»). Юзер выбрал Rust,
# поэтому теперь Rust — основной путь, а Python-движок — fallback до готовности.

## 0. Почему юзер прав, и в чём я обязан его поправить

Юзер: «мне не нужно объяснять… писать на расте», «через APK всё».
Честная рамка, зафиксированная замерами (не мнение):

* **Скорость Rust не даст.** RTT Telegram измерен живьём: запись медиана **191 мс**,
  чтение **366 мс** (PLAN v3 §1.1). Любое MTProto-приложение упирается в сеть,
  а не в интерпретатор. Перепись движка на Rust не уменьшит ни одну задержку.
* **Настоящий выигрыш — независимость от песочницы.** Сейчас всё умирает, когда
  Minis сворачивается: движок = PRoot-процесс, Android убивает его вместе с
  приложением (доказано 24.11: pid 26515 умер молча). Rust-движок внутри APK +
  foreground service = чистка идёт, даже когда Minis закрыт. Это и есть причина
  делать APK, и она веская.
* **Цена:** своя авторизация. Ни grammers, ни TDLib не принимают
  Pyrogram `session_string` (разные форматы сессий) ⇒ нужен НОВЫЙ вход
  по протоколу `/var/minis/shared/TG_AUTH_PROTOCOL.md` (SendCode и SignIn в ОДНОМ
  живом процессе, окно 15 мин, код юзер берёт из чата 777000 в Ayugram —
  пуш официала мёртв [И3]). Риск: лишние коды, FloodWait, потеря текущей сессии,
  если начну путать файлы сессий (в протоколе уже зафиксированы 8 таких ошибок).

## 1. Что уже доказано железно (исходники grammers 0.12.0 скачаны и прочитаны)

| факт | как проверено |
|---|---|
| grammers-client 0.12.0 жив, обновлён 08.11.2026, 294 596 загрузок | crates.io API |
| master (layer 234) обновлён **24.11.2026 17:55** — библиотека активно развивается | codeberg API, коммит 02367f97 |
| зависимости ВСЕ pure-Rust: futures-util, grammers-{crypto,mtsender,session,tl-types}, jiff, md5, mime_guess, pin-project-lite, tokio, tracing | `crates.io /dependencies` — ни ring, ни openssl, ни bindgen ⇒ кросс-компиляция на Android реальна |
| `Client::delete_messages(peer, &[i32]) -> Result<usize, InvocationError>` — пакетное удаление, возвращает число | исходник `src/client/messages.rs` |
| `Client::invoke<R: tl::RemoteCall>(...)` — произвольный TL-запрос доступен | исходник `src/client/net.rs:91` |
| авторизация: `is_authorized`, `request_login_code(phone, hash)`, `sign_in(token, code)`, `SignInError::PasswordRequired(token)` → `check_password(token, pass)` | примеры `dialogs.rs` (официальные) |
| `iter_dialogs`, `iter_participants`, `get_permissions`, `iter_messages`, `get_messages_by_id`, `upload_file`, `upload_media`, `download_media`, `resolve_username`, `resolve_peer`, `set_admin_rights`, `pin_message`, `send_album`, `stream_updates` + `Update::NewMessage` (есть `outgoing()`) | `grep pub fn` по `src/client/*.rs` |
| оформление супергруппы в TL ЕСТЬ: `channels.editPhoto`, `channels.editTitle` (+ `messages.editChatAbout/editTitle/editPhoto` для базовых), `inputChatUploadedPhoto#bdcdaec0`, `inputFileBig#fa4f0bb5` | `tl/api.tl` layer 233 по конструктор-ID |
| `messages.editChatPhoto#35ddd674`, `messages.getFullChat#aeb00b34`, `messages.getStickers#d5a5d3a1`, `photos.uploadProfilePhoto#388a3b5` | `tl/api.tl` по ID |
| **локальная сборка невозможна**: `grammers-tl-types` 0.12.0 имеет `edition = "2024"` ⇒ нужен Cargo ≥1.85, а в песочнице rustc/cargo **1.83** (Alpine) | `cargo fetch` → «feature `edition2024` is required» |
| сборка идёт в CI: публичный репо `tg-purge-ciprobe` даёт зелёные build-apk (15 шагов), приватный падает за 3 с (минуты Actions для private) | GitHub API, run 45592173859 success |
| prebuilt `libtdjson.so` arm64 = 31.9 МБ, экспортирует `td_json_client_{create,execute,send,receive,destroy}` (проверено strings по точным строкам) | скачано `up9cloud/android-libtdjson` v1.8.65 jniLibs.tar.gz 42.8 МБ |

★ Важная поправка к собственной методике: **substring/strings-проверка НЕ доказывает
отсутствие** метода TDLib (`sendChatMessage`/`getChatFullInfo`/`uploadFile` дали 0
совпадений строк, хотя в TDLib они есть — имена собираются из таблиц). Строки
доказывают только НАЛИЧИЕ. Поэтому выбор движка ниже опирается на grammers-исходники
(их читал текстом), а не на strings.

## 2. Выбор стека: grammers, а не TDLib (решение, не «потом посмотрим»)

| критерий | grammers 0.12 (Rust) | TDLib 1.8.65 через JSON |
|---|---|---|
| размер в APK | ~1–3 МБ .so | **31.9 МБ** .so (arm64) |
| нативные зависимости | нет (pure Rust) | C++ бинарь, только prebuilt |
| типизация | `tl::functions::*` на этапе компиляции | JSON-строки, ошибки только в рантайме |
| оформление чата | `channels.editPhoto/editTitle` + `invoke` | `setChatPhoto/setChatTitle/setChatDescription` (строки есть в .so) |
| сессия | `MemorySession` (+ свой файл) или `SqliteSession` (rusqlite ⇒ C-компиляция) | своя БД внутри TDLib |
| обновление API | master layer 234 от 24.11 | релиз 1.8.65 от 30.06 |
| риск | меньше покрытия high-level, часть через `invoke` | тяжёлый .so, JSON-верbose, нет типов |

**Решение: grammers.** Причина — типизация (комплятор = первый гейт), вес в APK и
живучесть апстрима (коммит вчера). TDLib остаётся ЗАПАСНЫМ вариантом, если grammers
упрётся в отсутствие нужной операции (это проверяемо за 1 CI-прогон, см. R3).

## 3. Гейты (ни один не пропускается; каждый = отдельный CI-прогон с числом)

**Хроника гейта R1 (раунды 6–11, все падения — мои догадки, не библиотека):**
| раунд | HEAD | итог | что упало |
|---|---|---|---|
| 1 | 3c9e2fc | — | `cargo \| tail` маскировал провал (pipe-код = код хвоста) |
| 2 | 2da5d29 | 0 jobs, 3 с | двоеточие в некавированном `name:` → YAML не разобрался |
| 3 | d871087 | 0 jobs | `Cargo.toml` без `[package]` |
| 4 | d24804d | step exit 101 | `cargo tree \| head -20` + pipefail → cargo паникует на Broken pipe; `setup-android@v3` упал; `cargo test --release` несовместим с `panic="abort"` |
| 6 | 1c52cf9 | lib: **23 ошибки** | догадки о типах grammers |
| 7 | 9bab85f | lib: **11 ошибок** | имена вариантов TL, `limit()` потребляет, пинна версии cargo-ndk |
| 8 | 8ef217c | lib: **5 ошибок** | unit-варианты (enums.rs:73-77) |
| 9 | 090de07 | lib: **1 ошибка** | `InputChatPhoto::InputChatUploadedPhoto` (префикс не режется) |
| 10 | 0ba8fea | **lib COMPILED** ✅, bin: 4 ошибки | `println!("-".repeat(..))` (не литерал), borrow `rows` после move |
| 11 | (в работе) | engine.rs + правки main.rs | — |

★ **cargo-ndk v4.1.2 реально ставится и NDK r27 ставится** (шаги 5–7 android зелёные в раунде 10),
значит путь «Rust-движок внутри APK» технически существует — это уже не предположение.

* **R1 — сборка.** `cargo build --release` для `x86_64-unknown-linux-gnu` (тесты)
  и `cargo ndk -t arm64-v8a build --release` (APK). Ворота: оба зелёные, .so на
  Android ≠ 0 байт, `nm` показывает `JNI_OnLoad` (если cdylib) или бинарь собирается.
  Не прошло ⇒ никакого переписывания движка, сначала чиним тулчейн/зависимости.
* **R2 — авторизация БЕЗ потери текущей сессии.** Новый процесс, новое окно 15 мин,
  сессия grammers пишется в `rustspike/purge.session` (НЕ в `tg_user_session.string`).
  Ворота: `is_authorized() == true`, `get_me()` даёт id 5804685716 / @armorberserk,
  И после этого Python-сессия всё ещё отвечает (`tg status` из движка v3).
  Риск потери текущей сессии снимается тем, что файлы сессий разные и я не трогаю
  `/var/minis/shared/tg_user_session.string`.
* **R3 — 5 операций, каждая с числом.** В полигоне Saved Messages (id 5804685716):
  1. пакетное `delete_messages(&[i32])` на N id → `usize` == N;
  2. `iter_messages` по истории (пагинация) → счёт совпадает с Python-движком;
  3. `upload_file` + `invoke(channels::editPhoto)` — на ГРУППЕ только с явного «да» юзера;
     до этого — проверка компиляции и типов, без сетевого вызова;
  4. `invoke(messages::getStickers)` по `set_name` → число стикеров == 55 (эталон Python);
  5. FloodWait: искусственный триггер (быстрая серия) → обработан, а не уронил процесс.
  Ворота: все 5 дают числа, совпадающие с Python-эталоном.
* **R4 — бенчмарк.** 159 сообщений @we_erti (столько же, сколько считал Python)
  в полигоне Saved Messages (клоны) или в группе с явного согласия. Сравнить
  wall-clock против Python-движка (baseline: 45 сообщений за 17 с поштучно;
  пакетами по 50 — ≤12 с на 159). Ворота: **≥2×** выигрыша, иначе Rust не переносит
  движок, а остаётся только оболочкой APK (и это честный результат, не провал).

## 4. Архитектура готового продукта (когда гейты пройдены)

```
APK tg.purge (Kotlin, ~200 строк, только оболочка)
├── jniLibs/arm64-v8a/libpurge.so      ← Rust (grammers + HTTP-сервер)
├── foreground service (START_STICKY)  ← держит MTProto-соединение живым
│     уведомление «чистка включена · удалено N» (требование Android 14)
├── Rust-поток: 127.0.0.1:8790 ── тот же JSON-контракт, что у Python-движка
└── WebView грузит http://127.0.0.1:8790/  ← UI v4 (index.html/app.css/app.js/strings.js)
```

* UI НЕ переписывается: уже приняты замеры «6 контролов в покое, 4 цвета, 0 машинных
  данных» и клики (PLAN_UI §11). Rust обязан отдать **тот же** `/api/*` контракт —
  тогда HTML/JS переезжают в assets без правок. Это и есть настоящая экономия.
* Python-движок остаётся работающим fallback до R4: юзер может чистить сейчас,
  не дожидаясь Rust.
* Сессии раздельные: Python `tg_user_session.string`, Rust `purge.session`.
  Ни один из движков не читает файл другого (иначе повторится ошибка №7 из
  TG_AUTH_PROTOCOL: telethon-читатель-pyrogram-сессии → «sqlite version column fail»).

## 5. Фазы (порядок исполнителя; каждая = коммит + отчёт с числами)

* **Ф-R0** — скелет crate: `Cargo.toml` (grammers-client 0.12, tokio, serde_json,
  tiny_http), `src/lib.rs` (модули: `caps`, `auth`, `store`, `api`, `engine`),
  CI `.github/workflows/rust.yml` с двумя таргетами. Выход: R1 зелёный.
* **Ф-R1** — `caps.rs`: компиляционные доказательства (каждый нужный тип/метод
  упомянут в коде, который компилируется, но не исполняется): `delete_messages`,
  `iter_messages`, `iter_dialogs`, `iter_participants`, `get_permissions`,
  `invoke::<channels::functions::EditPhoto/EditTitle>`, `upload_file`,
  `get_stickers`, `Update::NewMessage`, `InvocationError::FloodWait`.
  Выход: список «есть/нет» по 5 операциям R3 без сети.
* **Ф-R2** — `store.rs`: свой формат сессии (MemorySession + JSON/бинарный снапшот
  на диске в `files_dir`), ротация, явная миграция. Выход: переживает рестарт процесса.
* **Ф-R3** — `auth.rs`: SendCode/SignIn/2FA в ОДНОМ процессе, окно 900 с,
  файл-канал кода (`/var/minis/shared/rustspike/code.txt`) — ровно протокол 31.10.
  Выход: R2 зелёный, Python-сессия цела.
* **Ф-R4** — `engine.rs`: live-чистка (updates), пакетное удаление, бюджеты,
  FloodWait-паузы, зеркало-ответки, детекторы оформления, поллинг описания.
  Чистые функции (norm_chat, no_null, should_react, plan_batches, бюджеты) —
  перенос `core/model.py` 1:1, с теми же юнит-тестами (97 тестов → Rust `#[test]`).
* **Ф-R5** — `api.rs`: тот же контракт `/api/*`, что у Python (PLAN v3 §4),
  инвариант «ни одного null», то же состояние покоя UI.
* **Ф-R6** — Kotlin-оболочка + foreground service + WebView, сборка в CI
  (уже зелёный пайплайн `tg-purge-ciprobe` расширяется под Rust-таргет).
* **Ф-R7** — R4-бенчмарк с числами и окончательный вердикт: переносить движок
  целиком или оставить Python + Rust-оболочку.

## 6. Инварианты, переносимые из Python-версии (не обсуждаются)

1. Избранное (Saved Messages) неприкосновенно: удалять только id из реестра
   `self_owned`, отказ целиком, единая точка удаления (U-D28/U-D30).
2. Автовозврат оформления — только ручной `arm` с nonce, введённым человеком.
3. Галерея телефона запрещена: эталоны только из Telegram.
4. `set_profile_photo`/`delete_profile_photos` отсутствуют в проекте.
5. Удаление истории только с `confirm:true` + счётчиком до кнопки.
6. Себя нельзя выбрать целью. Чтение ≠ участие (NOT_PARTICIPANT → 403).
7. Статус = последствие («удалено N»), а не механизм; машинных данных в UI нет.
8. Тест, который сам инициализирует проверяемое состояние, не тестирует
   инициализацию (U-D29). Регресс обязан краснеть на намеренной поломке.

## 7. Чего я НЕ обещаю

* Срока «завтра»: R2 требует участия юзера (код из 777000) — без него авторизация
  невозможна физически, и я не буду слать коды «чтобы попробовать» (ошибка №2
  в протоколе: лишние коды 77241/36031 пролетели зря).
* Ускорения RTT. Если R4 даст <2×, я скажу «Rust не переносим», и это не провал.
* Совместимости с старыми Android < 24 и 32-битными ABI — не заявлял, не делаю.
* Что APK установится сам: root/adb/shizuku нет, установка только руками юзера.

---

## 8. Хроника CI-раундов (25.11) — все падения были МОИМИ ошибками автора пайплайна

| раунд | HEAD | что упало | корень | лечится |
|---|---|---|---|---|
| 1 | 3c9e2fc | шаги «тесты»/«caps» = OK при упавшей сборке | `cargo … \| tail` возвращает код хвоста | `set -o pipefail` в каждом шаге |
| 2 | 2da5d29 | run с **0 jobs** за 3 с | `name: … (без стража: только печать)` — двоеточие в некавированном скаляре | кавычки во всех `name`; `yaml.safe_load` ДО пуша |
| 3 | d871087 | run с 0 jobs | `Cargo.toml` без заголовка `[package]` | `tomllib`-валидация ДО пуша |
| 4 | d24804d | step «версия тулчейна» **exit 101**; android — `setup-android@v3` | `cargo tree \| head -20` + pipefail ⇒ cargo паникует на Broken pipe (Rust игнорирует SIGPIPE); экшен NDK падает | вывод в файл, `sed/head` читают файл; NDK ставим `sdkmanager` напрямую |
| 4 | — | `cargo test --release` | несовместим с `panic = "abort"` в `[profile.release]` | тесты в dev-профиле, релиз отдельно |
| 5 | (в работе) | — | валидатор сам давал ложные срабатывания: делил по `\|`, попадая в `\|\|` | `re.split(r"(?<!\|)\|(?!\|)", line)` + красный тест на `/tmp/bad.yml` |

Правила автора CI, вытекшие из этих шести падений (исполнитель обязан соблюдать):
1. Локальная валидация ДО пуша (`validate_before_push.py`): tomllib, yaml.safe_load,
   баланс скобок, запрет `cmd | head` при pipefail, запрет `_ =>` в match по
   `UpdateState`, запрет ссылок на несуществующие модули (`grammers_client::types`,
   `retry_policy::`).
2. Валидатор обязан уметь краснеть: раунд 5 начинает с прогона на заведомо сломанном
   `/tmp/bad.yml` и НЕ пушит, если тот не пойман.
3. Никаких `| head`/`| tail` рядом с pipefail: перенаправляем в файл.
4. Логи шагов пишем в файл и печатаем хвост только после успеха (`|| { tail -80 log; exit 1; }`).

## 9. Что известно о grammers 0.12.0 ЖЕЛЕЗНО (исходники в `vendor/`, не доки)

* ★ **ИСПРАВЛЕНО МОЁ ЛОЖНОЕ УТВЕРЖДЕНИЕ (раунд 10, чтение mtsender, а не догадка):**
  политики повторных попыток РЕАЛЬНО есть и используются — но не в клиенте, а в
  `grammers-mtsender/src/configuration.rs`: `lib.rs:33` экспортирует
  `{RetryPolicy, NoRetries, AutoSleep}`, `configuration.rs:123` —
  `impl Default for ConnectionParams` ставит `AutoSleep::default()`,
  `configuration.rs:198` — `AutoSleep { tries: NonZeroU32::MIN (=1), threshold: 60 s,
  io_errors_as_flood_of: Some(1 s) }`. Мёртв только файл
  `grammers-client/src/client/retry_policy.rs` (`mod retry_policy;` нигде нет).
  Практический вывод: FLOOD_WAIT ≤ 60 с ретраится ОДИН раз библиотекой, всё длиннее —
  приходит нам как ошибка, и паузу ставит движок.
* `delete_messages<C: Into<PeerRef>>(peer, &[i32]) -> Result<usize, InvocationError>` —
  док (messages.rs:873-888): «Deletes up to 100 messages… the amount of deleted messages
  is returned (it might be less than the amount of input message IDs if some were already
  missing)» ⇒ пакет ≤ 100, счёт — ОТ СЕРВЕРА (в Python это лечилось U-D31).
  Warning там же: для малых групп и ЛС Telegram **не проверяет** принадлежность id peer.
* `Message::delete(&self) -> Result<(), _>` (message.rs:759) — «Shorthand for
  Client::delete_messages», безопасный путь для одиночного id.
* `Message::reply<M: Into<InputMessage>>(&self, M) -> Result<Self, _>` (message.rs:689),
  `Message::forward_to<C: Into<PeerRef>>(&self, chat) -> Result<Self, _>` (:724),
  `Message::media(&self) -> Option<Media>` (:446), `Message::sender(&self) -> Option<&Peer>` (:305).
* `InputMedia::copy_media(&Media)` (input_media.rs:356) — зеркальный ответ «тем же стикером»
  без перекачивания байтов; `InputMedia::stickers(Option<Vec<InputDocument>>)` (:620).
  `impl Downloadable for Media` (media.rs:883), `download_media<D: Downloadable, P: AsRef<Path>>`
  (files.rs:212), `upload_file<P: AsRef<Path>> -> Result<Uploaded, io::Error>` (:500),
  `Uploaded { pub raw: tl::enums::InputFile }` (media.rs:40).
* Апдейты: `client.stream_updates(mpsc::Receiver<UpdatesLike>, UpdatesConfiguration)
  -> Result<UpdateStream, Box<dyn Error + Send + Sync>>` (updates.rs:69),
  `UpdateStream::next(&mut) -> Result<Update, InvocationError>`;
  `UpdatesConfiguration { pub catch_up: bool }` — **без Default**, форма из echo.rs:81;
  `UpdatesLike` реэкспортируется `grammers_session::updates` (updates.rs:13).
  Док: до stream обязаны быть закэшированы peers («use iter_dialogs at least once after login»).
* `SenderPool::new(Arc<S>, i32) -> SenderPool {runner, handle: SenderPoolFatHandle, updates}`
  с `S: Session + Sized, S::Error: std::error::Error + Send + Sync + 'static`;
  `SenderPoolRunner::run(mut self)` (sender_pool.rs:297); `Client::new(SenderPoolFatHandle)`
  ПОТРЕБЛЯЕТ handle (net.rs:49) ⇒ клонировать.
* Оформление в LAYER 233: `channels.editTitle#566decd0 channel:InputChannel`,
  `channels.editPhoto#f12e57c9 channel:InputChannel photo:InputChatPhoto`,
  `messages.editChatAbout#def60797 peer:InputPeer`, `messages.editChatTitle#73783ffd chat_id:long`,
  `inputChannel#f35aec28{channel_id,access_hash}`, `inputChannelEmpty#ee8c1e86`,
  `inputChatUploadedPhoto#bdcdaec0{file,video,video_start_ts,video_emoji_markup}`,
  `inputFileBig#fa4f0bb5`, `inputStickerSetShortName#861cc8a0`.
* ★ **Провал гейта R3 (единственный, подтверждён дважды):** `channels.editDescription`
  в слое 233 ОТСУТСТВУЕТ (grep по имени → 0, по ID `f8b0a74e` → 0). Описание супергруппы:
  git-master (layer 234, коммит 24.11.2026) / TDLib JSON / остаётся на Python-движке.
* ★ **ГЕЙТ R1 ПРОЙДЕН** (раунд 13, run 45673919282): host success + android success,
  артефакт `libpurge.so` = ELF aarch64 «for Android 24, built by NDK r27», 307216 Б,
  бинарь `purge` 498592 Б. Раунд 14: android снова зелёный (39.33 с сборка), host упал
  на шаге 6 (юнит-тесты в dev-профиле) — и отчёт не сохранился, потому что шаги после
  падения пропускаются ⇒ нужен `if: always()` на сборе диагностики.
* ★ `nm -D --defined-only libpurge.so` → **символов T: 0** (раунд 14). Причина:
  `[profile.release] strip = true` и нет ни одного `#[no_mangle] pub extern "C"`.
  Следствие для Ф-R6: либо Kotlin JNI-мост обязан звать только явно экспортированные
  `#[no_mangle] extern "C" fn` (и тогда `strip` для cdylib надо отключить/оставить
  символы), либо APK запускает бинарь `purge` как процесс (путь Termux). Выбор —
  часть Ф-R6, но проверка «что реально экспортируется» обязана быть в CI-отчёте
  всегда, иначе это всплывёт только на устройстве, которое я не могу прошить сам.
* Именование TL (rustifier.rs:92-122 + enums.rs:73-77): префикс типа режется только при
  `starts_with`; иначе вариант = ПОЛНОЕ имя (`inputChatUploadedPhoto` →
  `InputChatPhoto::InputChatUploadedPhoto`, НЕ `Uploaded`); `inputPeerSelf` → `PeerSelf`;
  конструкторы без параметров → **unit-варианты без структур**; структуры без namespace
  лежат прямо в `tl::types::X`.
* Методика: **отсутствие** доказываем только grep'ом по исходникам/`api.tl` и ID
  конструктора. `strings` по `.so` доказывает лишь наличие (на этом я чуть не
  похоронил TDLib wrongly: `sendChatMessage` не нашёлся строками, хотя метод есть).

* Нет модуля `types`: `Message` в `grammers_client::message`, пиры в `grammers_client::peer`,
  сессии в `grammers_client::session` (`pub use grammers_session as session`).
* Все peer-методы — `<C: Into<PeerRef>>` (`PeerRef` = `grammers_client::session::types::PeerRef`).
* `Message::id() -> i32`, `peer_id() -> PeerId`, `text() -> &str`, `outgoing() -> bool` — методы.
* `delete_messages<C: Into<PeerRef>>(peer, &[i32]) -> Result<usize, InvocationError>` —
  **сервер возвращает число удалённых**, то есть честный счёт возможен без
  `_confirmed_removed` (в Python-версии это был отдельный костыль, U-D31).
* Док `Message::id()`: для мегагрупп/каналов `delete_messages` НЕ проверяет peer ⇒
  движок обязан держать правильный peer сам, иначе удаление уйдёт не туда.
* `retry_policy.rs` есть в дереве, но `mod retry_policy;` нигде нет ⇒ встроенного
  ретрая НЕТ ⇒ FLOOD_WAIT обрабатываем сами: `InvocationError::Rpc(RpcError{code:420,
  name:"FLOOD_WAIT", value:Some(sec)})`.
* Оформление: `channels.editTitle#566decd0`, `channels.editPhoto#f12e57c9`,
  `messages.editChatTitle#73783ffd`, `editChatPhoto#35ddd674`, `editChatAbout#def60797`,
  `inputChatUploadedPhoto#bdcdaec0`, `inputChatPhotoEmpty#1ca48f57` — через `invoke`.
* **Провал гейта:** `channels.editDescription` в layer 233 ОТСУТСТВУЕТ (grep по имени
  и по ID `f8b0a74e` — 0). Описание супергруппы: либо git-master (layer 234, коммит
  24.11.2026), либо TDLib, либо остаётся на Python-движке.
* `SessionData` = 4 pub-поля (`home_dc`, `dc_options: HashMap<i32, DcOption>`,
  `peer_infos: HashMap<PeerId, PeerInfo>`, `updates_state`); внутренние типы имеют
  serde под фичей `grammers-session/serde` (у `grammers-client` serde-passthrough НЕТ,
  но фичи унифицируются по графу ⇒ подключаем `grammers-session = { features = ["serde"] }`
  напрямую). `MemorySession` не сохраняет состояние и не даёт его прочитать ⇒
  своё `FileSession` (`src/store.rs`), ключи peers восстанавливаем через `PeerInfo::id()`.
* `sqlite-storage` ∈ default у grammers-session ⇒ `default-features = false`,
  иначе libsql = C-компиляция на Android.
* Методика: **отсутствие** доказываем только grep'ом по исходникам/`api.tl` и ID
  конструктора. `strings` по `.so` доказывает лишь наличие (на этом я чуть не
  похоронил TDLib wrongly: `sendChatMessage` не нашёлся строками, хотя метод есть).

---

## 10. СТАТУС НА 26.11 08:47 (исполнитель: Minis) — что доказано, а не обещано

### Гейт R1 — ПРОЙДЕН (run 45754720790: host success + android success)
| метрика | число |
|---|---|
| host build.log | 0 warning, 0 error |
| host test.log | 0 warning, 0 error, **53** юнит-теста passed |
| android.log | 0 error (1 warning: `ANDROID_NDK_HOME` ≠ `ANDROID_NDK_ROOT` у cargo-ndk) |
| `libpurge.so` | **307 216 Б**, ELF aarch64 «for Android 24, built by NDK r27», stripped |
| бинарь `purge` | **2 250 528 Б** |
| caps-таблица | 11 из 12; непройденный гейт ровно один: `channels::EditDescription` |

Динамика ошибок по раундам: 23 → 11 → 5 → 1 → 4 → 0. Все 18 раундов сгорели на моих
догадках о типах и ошибках пайплайна, а не на grammers.

### Локально тестируемые слои (без CI, без grammers) — главная экономия времени
| слой | файл | как тестируется | результат |
|---|---|---|---|
| арифметика лимитов/гардов | `src/rate.rs` (только std) | `rustc --test` | 21 passed, 0.02 с |
| разбор кода входа | `src/util.rs` (только std) | `rustc --test` | 17 passed, 0.01 с |
| **форма JSON** | `src/shape.rs` (serde_json) | `tools/localtest` (cargo, подключает боевой файл через `#[path]`, а не копию) | **18 passed, 0 warning** |
| паритет окон Rust↔Python | `tools/parity.py` + `tools/parity.rs` | локально | 2029 сценариев, 0 расхождений |
| паритет no_null Rust↔Python | `tools/parity_json.py` | локально | **157 структур, 0 расхождений** |

Правило: **если тулчейн не даёт тестировать логику — выноси её в модуль без тяжёлой
зависимости.** cargo 1.83 в песочнице собирает serde/serde_json (замерено), поэтому
JSON-слой тестируется за 25 с, а не за 6 минут CI.

### Что нашёл паритет no_null (и почему он окупился)
1. Python-таблицы ключей отстали от схемы v3: `targets/since/last_seen/sent_total/
   cooldown_sec/cleaning/known/in_targets` отсутствовали ⇒ null превращался в `""`
   вместо `0`/`false` — UI получал строку там, где должен число/флаг.
2. Обратный дрейф поймал мой же тест (`removed_count` был только в Python).
3. Ключи `about`/`title`/`avatar` имеют **разный тип в разных местах схемы**
   (`watch.about` bool, `refs.about` dict, `chat.about` str) ⇒ глобальная таблица
   типов для них принципиально невозможна. Введено `AMBIGUOUS_KEYS` (всегда `""`)
   + механизм точечных исключений по пути (`PATH_DEFAULTS`: пуст, но протестирован —
   пустой и непроверенный механизм это ловушка). Приоритет в `no_null`:
   путь → амбигуозный → числовой → булевый → `""`.

### Боевые факты по Python-движку (fallback, работает сейчас)
- Первое live-удаление в ГРУППЕ состоялось: 26.11 07:51, `deleted=1`, подтверждено
  двумя независимыми источниками (событие движка + mtime `state.json`).
- Дыра F4 закрыта: движок умирал при суспенде приложения, @we_erti прислала
  5417/5418 (25.11 19:46) — они выжили, а `missed` показал 0, потому что базис
  ставился только при удалении. Теперь `live.since` пишется в момент включения,
  базис = `max(since, last_seen)`, после расчёта базис сдвигается (нет двойного счёта).
- 129 Python-теста OK, AST-чекер чист.

### Следующие фазы (порядок плана, без изменений)
1. **Ф-R2/R3 — ворота юзера:** `--auth` требует интерактивного ввода кода из чата 777000
   (окно 15 мин, SendCode и SignIn в ОДНОМ процессе). Без него Rust-движок не
   авторизуется: grammers НЕ принимает Pyrogram `session_string` (форматы несовместимы).
2. **Ф-R4:** живой цикл `engine.rs` поверх `rate.rs` + mirror-ответки
   (`InputMedia::copy_media`) + детекторы оформления (`invoke(channels::Edit*)`).
3. **Ф-R5:** `api.rs` — тот же контракт `/api/*`, что у Python, поверх `shape.rs`,
   чтобы UI v4 переехал в APK assets без правок.
4. **Ф-R6:** Kotlin-оболочка + foreground service + WebView. ★ Открытый вопрос раунда 18:
   `nm -D` даёт **0 символов T** в `libpurge.so` (strip=true и нет `#[no_mangle]
   extern "C"`) ⇒ JNI-звать нечего. Лечится: экспорты + `strip=false` для cdylib,
   либо запуск бинаря `purge` как процесса (путь Termux). Проверять обязанно в CI-отчёте
   всегда, иначе всплывёт только на устройстве, которое я сам прошить не могу.
5. **Ф-R7 / гейт R4:** бенчмарк 159 сообщений против Python (пакеты по 50, ≤12 с).
   Порог ≥2× — иначе Rust не переносит движок, и это честный результат, не провал.
