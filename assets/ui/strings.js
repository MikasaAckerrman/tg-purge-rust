/* SLOVAR' (PLAN_UI §4) — единственный источник подписей.
   UI, CLI и журнал берут строки отсюда. Правила: глагол+объект, без иконок,
   без машинных данных (sha/байты/pid/id/uptime), без военного жаргона.
   Функции-строки — только для числительных; склонение делает здесь, а не наугад в UI. */
(function () {
  function plural(n, one, few, many) {
    const m = Math.abs(n) % 100, d = m % 10;
    if (m > 10 && m < 20) return many;
    if (d > 1 && d < 5) return few;
    if (d === 1) return one;
    return many;
  }
  function count(n, one, few, many) { return `${n} ${plural(n, one, few, many)}`; }

  /* Все русские числительные живут здесь: app.js не имеет права на свои строковые
     литералы, иначе тест §4 («единый словарь») становится бумажкой. */
  const peopleWord = (n) => count(n, "человек", "человека", "человек");
  const msgWord = (n) => (n > 0 ? count(n, "сообщение", "сообщения", "сообщений") : "нет сообщений");
  const targetWord = (n) => (n > 0 ? count(n, "цель", "цели", "целей") : "выбери, чьи сообщения удалять");
  const rulesWord = (n) => count(n, "правило", "правила", "правил");
  const chatsWord = (n) => count(n, "чат", "чата", "чатов");
  const oldWord = (n) => count(n, "старое сообщение", "старых сообщения", "старых сообщений");
  const doneWord = (n) => count(n, "сообщение удалено", "сообщения удалено", "сообщений удалено");
  const missedWord = (n) => (n === 0 ? "за простое ничего не пропущено"
                                     : count(n, "сообщение пропущено", "сообщения пропущено", "сообщений пропущено"));
  const sentWord = (n) => (n === 0 ? "ещё не срабатывало"
                                   : count(n, "ответ отправлен", "ответа отправлено", "ответов отправлено"));
  const stickWord = (n) => count(n, "стикер", "стикера", "стикеров");

  const T = {
    /* ── строка 1: ГДЕ ── */
    where_hint: "какой чат чистим",
    where_change: "сменить",
    where_search_ph: "название, @ник, id или tg://chat?id=",
    search_none: "ничего не найдено",
    chat_noname: "без названия",
    here: "этот чат",

    /* ── типы чатов: один и тот же тип не должен выглядеть как «два одинаковых» (U-D6) */
    type_group: "группа",
    type_group_small: "группа",
    type_channel: "канал",
    type_bot: "бот",
    type_private: "личная переписка",

    /* ── строка 2: КОГО ── */
    who_label: "цели",
    who_none: "выбери, чьи сообщения удалять",
    who_add: "выбрать",
    who_me: "это ты",
    who_not_self: "свои сообщения удалять нельзя",
    who_saved: (n) => n === 0 ? "не выбран никто" : targetWord(n) + " выбрано",
    /* D1/D2 (26.11): маркер несохранённого черновика. Без него строка «ЦЕЛИ»
       либо отставала от галочек (казалось «не убирается»), либо молча теряла
       добавленные цели при закрытии панели. */
    who_dirty: "не сохранено",
    who_close_dirty: (c) => `есть несохранённый выбор: тапни «${c}» ещё раз, чтобы отменить`,
    who_title: "кого удаляем",
    who_filter_ph: "фильтр по имени или нику",
    who_done: "готово",
    who_owner: "владелец",
    who_admin: "админ",
    who_member: "участник",
    who_not_member: "не участник",
    who_restricted: "ограничен",
    msgs_none: "нет сообщений",

    /* ── строка 3: ЧТО ── */
    what_new: "только новые",
    what_all: "новые и старые",
    what_all_count: (n) => `новые и старые · ${n}`,
    what_hint_new: "история не трогается",
    what_hint_all: "старые сообщения целей тоже удалятся",

    /* ── строка 4: ПУСК ── */
    power_start: "включить чистку",
    power_stop: "выключить чистку",
    power_no_rights: "чистка невозможна: нет прав",
    power_no_targets: "чистка невозможна: не выбраны цели",
    power_no_chat: "чистка невозможна: чат не выбран",
    // BA-10 (26.11): в личной переписке цель НЕ выбирается из списка — там
    // собеседник является целью по определению (M.effective_targets). Без этой
    // строки панель «ЦЕЛИ» показывала «выбери, чьи сообщения удалять» при уже
    // работающей чистке: тот же класс лжи, что и «нет прав», только на другом
    // элементе. Основание: «Client can delete incoming messages in private chats»
    // (pyrogram/methods/messages/delete_messages.py, Kurigram 2.2.26).
    power_dm_hint: "в личной переписке чистятся сообщения собеседника",

    /* ── статус = последствие, не механизм (§2.8) ── */
    status_idle: "чистка выключена",
    status_live: (at) => `чистка включена с ${at}`,
    status_deleted: (n, at) => `${doneWord(n)} · с ${at}`,
    status_dead: "движок не отвечает",
    // BA-15 (26.11): сборка устарела, а перезагрузиться сама страница не смогла
    // (sessionStorage недоступен либо сервер отдаёт код старше своего штампа).
    // Молчать здесь нельзя: юзер примет это за «кнопка не работает».
    status_stale: (b) => `открыта старая версия страницы: закрой и открой заново (сборка ${b})`,
    // BA-16 (26.11): отказ открытия показывается в панели поиска, а не в whereSub:
    // панель остаётся открытой, юзер видит причину и может попробовать другой ввод.
    open_failed: (e) => `не открылось: ${e}`,
    status_dead_how: "в терминале Minis: tg up",

    /* ── подтверждение сноса (§3: замещает строку пуска, число в лейбле) ── */
    sweep_ask: (n) => `удалить ${oldWord(n)} для всех?`,
    sweep_go: (n) => `удалить ${n}`,
    sweep_cancel: "отмена",
    sweep_counting: "считаю…",
    sweep_doing: "удаляю…",
    sweep_nothing: "старых сообщений целей нет",
    sweep_irreversible: "отката в Telegram нет",
    sweep_scanned: (n) => `просмотрено ${n}`,
    sweep_scope_all: "вся история",
    sweep_done: (n) => n === 0 ? "старых сообщений не оказалось" : oldWord(n) + " удалено",

    /* ── ещё ── */
    more: "ещё",
    hide: "спрятать",
    more_items: ["аватар и описание", "ответ стикером", "журнал", "чаты"],
    close: "закрыть",
    saved: "сохранено",
    not_saved: "не сохранилось",
    err_notjson: "движок ответил непонятно",

    /* ── аватар и описание ── */
    look_title: "аватар и описание",
    look_avatar: "аватар",
    look_about: "описание",
    look_title_name: "название",
    look_now: "как сейчас",
    look_about_empty: "описание пустое",
    look_ref_none: "эталон не снят",
    look_ref_when: (d) => `вернём к тому, что было ${d}`,
    look_watch_on: "следим",
    look_watch_off: "не следим",
    look_restore_on: "возвращаем сами",
    look_restore_off: "сами не меняем",
    look_how_arm: "включить возврат: в терминале tg arm (аватар / описание / название)",
    look_take: "снять эталон",
    look_restore_now: "вернуть сейчас",

    /* ── ответ стикером ── */
    rules_title: "ответ стикером",
    rules_none: "правил нет",
    rules_count: (n) => count(n, "правило", "правила", "правил"),
    rules_new: "новое правило",
    rules_who: "кому отвечаем",
    rules_who_ph: "— выбери человека —",
    rules_what: "чем отвечаем",
    rules_what_ph: "— выбери стикер из пака —",
    rules_trigger: "на что отвечаем",
    rules_trigger_sticker: "на стикер",
    rules_trigger_media: "на любое медиа",
    rules_trigger_text: "на текст",
    rules_mode: "как отправляем",
    rules_mode_reply: "ответом на его сообщение",
    rules_mode_plain: "обычным сообщением",
    rules_mode_mirror: "тем же стикером, что он прислал",
    rules_mode_short_reply: "ответом",
    rules_mode_short_plain: "сообщением",
    rules_mode_short_mirror: "тем же стикером",
    rules_mirror_no_sticker: "зеркало работает только по стикеру — триггер обязан быть «на стикер»",
    rules_cooldown: "пауза между ответами, сек",
    rules_rate: "не больше в минуту",
    rules_rate_short: (n) => n > 0 ? `${n} в минуту` : "без потолка",
    rules_create: "создать правило",
    rules_need_sticker: "выбери стикер в списке выше",
    rules_need_who: "выбери человека выше",
    rules_mirror: "отвечать тем же стикером",
    rules_on: "включено",
    rules_off: "выключено",
    rules_test: "проверить",
    rules_testing: "проверяю…",
    rules_test_ok: "проверено в твоих сохранённых: отправлено ровно то, что выбрано",
    rules_test_fail: "не совпало — отправленное удалено сразу",
    rules_delete: "убрать правило",
    rules_sent: (n) => sentWord(n),
    rules_warn: "массовая отправка = риск ограничений от Telegram",
    packs_none: "паков не видно — пусть пришлют стикер",
    packs_pick_first: "выбери пак выше",
    packs_loading: "загружаю стикеры…",
    packs_ready: (n) => stickWord(n),
    sticker_video: "видео",
    sticker_animated: "анимация",

    /* ── журнал ── */
    log_title: "журнал",
    log_empty: "пока пусто",
    log_missed: (n) => missedWord(n),
    log_run: "проверить, что чистка работает",
    log_running: "проверяю…",
    log_ok: "живой пайплайн работает",
    log_fail: "самопроверка не прошла",
    log_ms: (n) => `${n} мс`,
    log_clear: "очистить журнал",

    /* ── чаты ── */
    chats_title: "чаты",
    chats_none: "список пуст",
    chats_reload: "обновить список чатов",
    chats_reloading: (n) => `читаю диалоги: ${n}`,
    chats_targets: (n) => n === 0 ? "целей нет" : targetWord(n),
    chats_clean_on: "чистит",
    chats_clean_off: "не чистит",
    chats_here: "этот чат",
    chats_known: "здесь уже выбирали цели",
    who_in_chat: "цель выбирается внутри чата: открой чат и отметь человека",
    rules_mirror_no_sticker: "зеркало отвечает только стикером — триггер обязан быть «на стикер»",
    search_people: "люди",
    search_chats: "чаты",

    /* ── неполадки ── */
    err_title: "неполадки",
    err_none: "неполадок нет",
  };

  /* что запрещено печатать в UI (машинный мусор) — ассерт теста §2.4 */
  T.FORBIDDEN = ["sha", "pid", "uptime", "SUPERGROUP", "ADMINISTRATOR", "armed", "detected"];

  window.plural = plural;
  window.count = count;
  window.words = {peopleWord, msgWord, targetWord, rulesWord, chatsWord,
                  oldWord, doneWord, missedWord, sentWord, stickWord};
  window.T = T;
})();
