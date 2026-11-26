"use strict";
/* TGPURGE UI v4 (PLAN_UI §1…§6).
   ГЛАВНЫЙ ИНВАРИАНТ (корень U-D2/U-D3): опрос состояния не имеет права пересобирать DOM.
   Статика монтируется один раз в mount(); опрос точечно правит textContent/classList
   только тех узлов, чьё значение реально изменилось (сверка с самим узлом). Списки
   сверяются по ключу — узлы переиспользуются, поэтому фокус/выбор/скролл не теряются.
   Поля ввода живут в HTML и опросом не трогаются никогда. */

const T = window.T;
const byId = (s) => document.getElementById(s);

/* ───────── состояние экрана (только здесь, не в DOM) ───────── */
const UI = {
  panel: null,           // where|who|look|rules|log|chats|err|null
  scope: "new",          // черновик «что удаляем»: new | all (не persist)
  ask: null,             // {found, scanned, all, job} — подтверждение сноса
  busy: false,
  whoSel: [],            // черновик целей до «готово»
  whoFilter: "",
  searchQ: "",
  searchRes: null,
  pickSha: "",
  packStk: [],
  packOpen: "",
  invBusy: false,
  invNote: "",
  whoSeeded: false,        // черновик целей засеян из сервера?
  whoOrder: [],            // фиксированный порядок строк на время открытой панели (D3-i)
  whoAskClose: false,      // первый тап «закрыть» при несохранённом выборе — ждём второй
  dead: false,
  flash: null,           // {node, text, until, base} — сообщения не затираются опросом
  tapAt: {},              // {ключ: метка} — защита от двойного тапа (D3-ii)
  uiSeq: 0,               // номер отправки /api/ui: порядок на сервере = порядок тапов
  uiChain: Promise.resolve(),
};

let DATA = null;
let pollTimer = 0;
let countTimer = 0;
let searchTimer = 0;

/* ───────── точечная запись с дифом ─────────
   Диф сравнивается с ФАКТИЧЕСКИМ состоянием узла, а не с внешним кэшем.
   Причина (поймано живьём 25.11): side-кэш жил дольше узла — reconcile пересоздаёт
   <button> при смене фильтра, новый узел пуст, а кэш кричит «уже записано», и 4 из 5
   строк списка людей вышли пустыми. Узел — единственный источник истины о экране. */
function setText(node, val) {
  if (!node) return;
  const v = val == null ? "" : String(val);
  if (node.textContent !== v) node.textContent = v;
}
function setAttr(node, name, val) {
  if (!node) return;
  if (val === null || val === false) {
    if (node.hasAttribute(name)) node.removeAttribute(name);
  } else {
    const s = val === true ? "" : String(val);
    if (node.getAttribute(name) !== s) node.setAttribute(name, s);
  }
}
function setClass(node, cls, on) {
  if (!node) return;
  const v = !!on;
  if (node.classList.contains(cls) !== v) node.classList.toggle(cls, v);
}
function setHidden(node, hidden) {
  setAttr(node, "hidden", hidden ? true : null);
}
function setStyle(node, prop, val) {
  if (!node) return;
  if (node.style[prop] !== val) node.style[prop] = val;
}

/* ───────── сверка списка по ключу (узлы не уничтожаются зря) ───────── */
function reconcile(container, items, keyFn, make, patch) {
  // keyFn обязан получать индекс: без него ключи вида "m"+i схлопывались в
  // "mundefined" для ВСЕХ строк, и список показывал одну строку N раз
  // (поймано живьём 25.11: панель «ещё» показывала «чаты» вместо «журнал»).
  const keys = items.map((it, i) => String(keyFn(it, i)));
  const have = new Map();
  for (const n of Array.from(container.children)) {
    if (n.dataset.k === undefined) continue;
    have.set(n.dataset.k, n);
  }
  for (const [k, n] of have) if (!keys.includes(k)) container.removeChild(n);
  items.forEach((it, i) => {
    const k = keys[i];
    let n = have.get(k);
    if (!n) { n = make(it, i); n.dataset.k = k; have.set(k, n); }
    const cur = container.children[i];
    if (cur !== n) container.insertBefore(n, cur || null);
    patch(n, it, i);
  });
}

function mkRow() {
  // без параметров: reconcile зовит make(it, i), а старая подпись mkRow(cls)
  // превращала это в class="item [object Object]" — мусор в DOM (U-D26)
  const b = document.createElement("button");
  b.className = "item";
  return b;
}
function mkSpan(cls) {
  const s = document.createElement("span");
  s.className = cls;
  return s;
}
function rowParts(node, parts) {
  for (const p of parts) if (!node.querySelector("." + p)) node.appendChild(mkSpan(p));
}
function ensureEmpty(box, text) {
  let e = box.querySelector(".empty");
  if (!text) { if (e) box.removeChild(e); return; }
  if (!e) { e = document.createElement("div"); e.className = "empty"; box.appendChild(e); }
  setText(e, text);
}

/* ───────── flash-сообщение: переживает опрос (иначе его затирает) ───────── */
/* flash — только для ошибок и подтверждений. Любая МУТАЦИЯ состояния обязана
   его стирать: иначе старое сообщение («свои сообщения удалять нельзя») маскирует
   свежее положение («2 цели выбрано») — поймано живьём 25.11 при кликах.
   D3-iv (26.11): раньше flash держал поле ровно 4 с по таймеру и в конце делал
   полный renderAll() — то есть нажатие выглядело «тем же окном» ещё 4 секунды после
   того, как данные изменились. Теперь flash держится, только пока БАЗОВОЕ значение
   не изменилось: shown(id, base) сравнивает с тем, что было на момент flash. */
function clearFlash() { UI.flash = null; }

function flash(node, text) {
  if (!node) return;
  UI.flash = {id: node.id, text: String(text || ""), until: Date.now() + 4000,
              base: node.textContent};
  node.textContent = UI.flash.text;
  setTimeout(() => { if (UI.flash && Date.now() >= UI.flash.until) { UI.flash = null; renderAll(); } }, 4200);
}
/* Единственный законный способ нарисовать поле с возможным flash:
   как только базовое значение сменилось (нажал → данные обновились), flash отпускает.
   Сравнение с f.base, а НЕ с textContent: пока flash активен, в DOM лежит его текст,
   поэтому «базу» можно взять только из аргумента вызывающего.
   flashNode УДАЛЁН намеренно (26.11): старый паттерн `flashNode(id) || base`
   держал поле 4 с по таймеру независимо от того, изменились ли данные, — это и был
   D3-iv («нажал — то же окно»). Два механизма на одно = гарантированный рассинхрон. */
function shown(id, base) {
  const f = UI.flash;
  if (f && f.id === id && Date.now() < f.until && f.base === base) return f.text;
  return base;
}

/* ───────── защита от двойного тапа (D3-ii) ───────── */
/* Клик-клик по строке-переключателю делал «закрыть+открыть» = визуально ничего.
   Одноразовый guard: повторный тап в течение 300 мс игнорируется целиком. */
function tapOK(key) {
  const now = Date.now();
  const prev = UI.tapAt[key] || 0;
  if (now - prev < 300) return false;
  UI.tapAt[key] = now;
  return true;
}

/* ───────── отправка состояния панели (D3-ii) ───────── */
/* Раньше /api/ui уходил без ожидания: два быстрых тапа могли долететь в обратном
   порядке, и после перезагрузки страницы восстанавливалась НЕ та панель (U-D12).
   Цепочка промисов гарантирует: сервер получает правки в порядке тапов. */
function apiUi(panel) {
  UI.uiSeq += 1;
  const mine = UI.uiSeq;
  UI.uiChain = UI.uiChain.then(async () => {
    // более поздний тап уже в очереди — этот ответ не важен, но слать надо по порядку
    const j = await api("/api/ui", {panel: panel});
    if (!j.ok && mine === UI.uiSeq) flash(byId("status"), j.error || T.not_saved);
  });
  return UI.uiChain;
}

/* ───────── словарь в одну линию: человек/числа ───────── */
/* Числительные — из словаря (window.words), НЕ отсюда: свои русские литералы
   в app.js сделали бы тест §4 «единый словарь» бумажкой. */
const W = window.words;
function peopleWord(n) { return W.peopleWord(n); }
function msgWord(n) { return W.msgWord(n); }
function targetWord(n) { return W.targetWord(n); }

function typeName(t) {
  const k = String(t || "").toUpperCase();
  if (k === "CHANNEL") return T.type_channel;
  if (k === "BOT") return T.type_bot;
  if (k === "USER" || k === "PRIVATE") return T.type_private;
  if (k === "GROUP") return T.type_group_small;
  return T.type_group;
}
function roleName(s) {
  const k = String(s || "").toUpperCase();
  if (k === "OWNER" || k === "CREATOR") return T.who_owner;
  if (k === "ADMINISTRATOR") return T.who_admin;
  if (k === "NOT_PARTICIPANT") return T.who_not_member;
  if (k === "RESTRICTED") return T.who_restricted;
  // BA-9 (26.11): движок помечает ЛС ролью "ЛС" (engine.py open_chat). Без явного
  // маппинга она попадала в default → «участник», то есть строка участника
  // личной переписки подписывалась как групповая роль. Существующая строка
  // словаря, нового литерала не заводим (§4 единый словарь).
  if (k === "ЛС") return T.type_private;
  return T.who_member;
}

/* ───────── транспорт ───────── */
async function api(path, body) {
  try {
    const opt = {cache: "no-store"};
    if (body !== undefined) {
      opt.method = "POST";
      opt.headers = {"Content-Type": "application/json"};
      opt.body = JSON.stringify(body);
    }
    const r = await fetch(path, opt);
    const t = await r.text();
    let j = {};
    try { j = JSON.parse(t); } catch (e) { j = {ok: false, error: T.err_notjson}; }
    j._code = r.status;
    return j;
  } catch (e) {
    return {ok: false, dead: true, error: T.status_dead};
  }
}

/* ───────── монтаж статики: ровно один раз ───────── */
const DEEP = {where: "panelWhere", who: "panelWho", look: "panelLook",
              rules: "panelRules", log: "panelLog", chats: "panelChats", err: "panelErr"};

function mount() {
  setText(byId("whoLabel"), T.who_label);
  setText(byId("whatNew"), T.what_new);
  setText(byId("lblAbout"), T.look_about);
  setText(byId("lblRef"), T.look_now);
  setText(byId("lookTitle"), T.look_title);
  setText(byId("rulesTitle"), T.rules_title);
  setText(byId("rulesNewLbl"), T.rules_new);
  setText(byId("lblRuleWho"), T.rules_who);
  setText(byId("lblRuleTrigger"), T.rules_trigger);
  setText(byId("lblRuleMode"), T.rules_mode);
  setText(byId("lblRuleCd"), T.rules_cooldown);
  setText(byId("lblRuleRate"), T.rules_rate);
  setText(byId("ruleWarn"), T.rules_warn);
  setText(byId("logTitle"), T.log_title);
  setText(byId("logRun"), T.log_run);
  setText(byId("chatsTitle"), T.chats_title);
  setText(byId("errTitle"), T.err_title);
  setText(byId("sweepCancel"), T.sweep_cancel);
  setText(byId("whoDone"), T.who_done);
  setAttr(byId("searchQ"), "placeholder", T.where_search_ph);
  setAttr(byId("whoFilter"), "placeholder", T.who_filter_ph);

  fillSelect(byId("ruleTrigger"), [["sticker", T.rules_trigger_sticker],
                                   ["any_media", T.rules_trigger_media],
                                   ["text", T.rules_trigger_text]]);
  fillSelect(byId("ruleMode"), [["reply", T.rules_mode_reply],
                                ["plain", T.rules_mode_plain],
                                ["mirror", T.rules_mode_mirror]]);

  for (const n of document.querySelectorAll("[data-close]")) {
    setText(n, T.close);
    // Единая точка закрытия: [data-close] внутри панели «who» обязан проходить через
    // подтверждение несохранённого выбора (D2), иначе кнопка «закрыть» в панели
    // обошла бы guard и молча потеряла черновик.
    n.addEventListener("click", closePanel);
  }
  byId("rowWhere").addEventListener("click", () => {
    if (!tapOK("where")) return;                       // D3-ii
    if (UI.panel === "where") closePanel(); else gotoPanel("where");
  });
  byId("rowWho").addEventListener("click", openWho);
  byId("whatNew").addEventListener("click", () => setScope("new"));
  byId("whatAll").addEventListener("click", () => setScope("all"));
  byId("power").addEventListener("click", onPower);
  byId("more").addEventListener("click", toggleMore);
  byId("searchQ").addEventListener("input", (e) => { UI.searchQ = e.target.value; scheduleSearch(); });
  byId("whoFilter").addEventListener("input", (e) => { UI.whoFilter = e.target.value; renderWho(); });
  byId("whoDone").addEventListener("click", saveTargets);
  byId("sweepGo").addEventListener("click", doSweep);
  byId("sweepCancel").addEventListener("click", cancelSweep);
  byId("chatsReload").addEventListener("click", reloadInventory);
  byId("ruleCreate").addEventListener("click", createRule);
  byId("logRun").addEventListener("click", runSelftest);
}

function fillSelect(sel, pairs) {
  const sig = pairs.map(p => p[0] + "=" + p[1]).join("|");
  if (sel.dataset.sig === sig) return;      // не пересобирать: иначе сбрасывается выбор
  sel.dataset.sig = sig;
  sel.textContent = "";
  for (const [v, label] of pairs) {
    const o = document.createElement("option");
    o.value = v; o.textContent = label;
    sel.appendChild(o);
  }
}

/* ───────── панели ───────── */
function setPanel(name, persist) {
  UI.panel = name || null;
  // U-D18 (найдено живьём 25.11): панель «who» восстанавливается из state при
  // загрузке страницы, но черновик целей тогда пуст → список показывает
  // «выбери, чьи сообщения удалять» и ни одной галочки, хотя цель сохранена.
  // Черновик обязан засеиваться из сервера при ВХОДЕ в панель, а не только в openWho().
  if (UI.panel === "who" && !UI.whoSeeded) seedWho();
  if (!UI.panel) UI.whoSeeded = false;
  for (const [k, id] of Object.entries(DEEP)) setHidden(byId(id), UI.panel !== k);
  renderSweepBox();
  renderMore();
  if (UI.panel === "who") renderWho();
  if (UI.panel === "where") renderSearch();
  if (UI.panel === "look") renderLook();
  if (UI.panel === "rules") renderRules();
  if (UI.panel === "log") renderLog();
  if (UI.panel === "chats") { renderChats(); loadChats(); }
  if (UI.panel === "err") renderErr();
  if (persist !== false) apiUi(UI.panel);        // D3-ii: по порядку, не fire-and-forget
}

function toggleMore() {
  const box = byId("panelMore");
  const open = box.hasAttribute("hidden");
  setHidden(box, !open);
  setText(byId("more"), open ? T.hide : T.more);
  if (open) renderMore();
}

function renderMore() {
  const box = byId("moreList");
  const map = ["look", "rules", "log", "chats"];
  const c = (DATA && DATA.chat) || {};
  const inv = (DATA && DATA.inventory_counts) || {};
  const nRules = ((c.reply_rules) || []).length;
  const nDeleted = (DATA && DATA.demon && DATA.demon.deleted) || 0;
  const nChats = (inv.groups || 0) + (inv.channels || 0) + (inv.bots || 0) + (inv.users || 0);
  const suffix = [
    "",                                       // аватар и описание: состояние видно внутри панели
    nRules ? W.rulesWord(nRules) : "",         // ответ стикером: сколько правил
    nDeleted ? String(nDeleted) : "",          // журнал: сколько удалено
    nChats ? W.chatsWord(nChats) : "",         // чаты: сколько всего
  ];
  reconcile(box, T.more_items, (it, i) => "m" + i, mkRow, (node, label, i) => {
    rowParts(node, ["t", "s"]);
    setText(node.querySelector(".t"), label);
    const extra = suffix[i] ? ` · ${suffix[i]}` : "";
    setText(node.querySelector(".s"),
            UI.panel === map[i] ? T.close : (extra || "›"));
    node.onclick = () => {
      if (!tapOK("more:" + i)) return;                 // D3-ii: двойной тап = ничего
      if (UI.panel === map[i]) closePanel(); else gotoPanel(map[i]);
    };
  });
}

/* ───────── 1. ГДЕ ───────── */
function peoplePhrase(n) {
  // «0 человек» — ложное утверждение: Telegram не отдаёт число участников у базовых
  // групп. Если числа нет, показываем только тип (U-D6 решаем ЧЕСТНО, а не нулём).
  return n > 0 ? " · " + peopleWord(n) : "";
}

function renderWhere() {
  const c = (DATA && DATA.chat) || null;
  if (!c) {
    setText(byId("whereTitle"), T.where_hint);
    setText(byId("whereSub"), shown("whereSub", T.where_search_ph));
    setStyle(byId("ava"), "backgroundImage", "");
    setText(byId("whereMark"), T.where_change);
    return;
  }
  const n = c.members_count || (c.members || []).length;
  setText(byId("whereTitle"), c.title || T.where_hint);
  setText(byId("whereSub"), shown("whereSub", `${typeName(c.type)}${peoplePhrase(n)}`));
  setText(byId("whereMark"), UI.panel === "where" ? T.close : T.where_change);
  const ava = byId("ava");
  const url = "/api/ref/img?kind=current&t=" + (c.avatar_stamp || 0);
  if (ava.dataset.url !== url) {
    ava.dataset.url = url;
    const probe = new Image();
    probe.onload = () => setStyle(ava, "backgroundImage", `url(${url})`);
    probe.onerror = () => setStyle(ava, "backgroundImage", "");
    probe.src = url;
  }
}

function renderSearch() {
  const box = byId("searchList");
  const q = (UI.searchQ || "").trim();
  if (!q) { reconcile(box, [], () => "x", mkRow, () => {}); ensureEmpty(box, T.where_hint); return; }
  const res = UI.searchRes || {chats: [], people: []};
  // BA-2 (замерено 26.11 живьём, а не «по чтению кода»). Было ДВА вызова reconcile
  //   на один #searchList: чаты, затем люди. reconcile удаляет узлы, ключа которых нет
  //   в ЕГО списке (app.js:92), поэтому второй вызов стирал результат первого:
  //   прогон q=«зеленое» дал 2 узла после chats и 0 после people (wiped=2). Отсюда
  //   «при поиске работает только текст» — список чатов не доходил до экрана, а
  //   ensureEmpty печатал пустую строку, видя непустые rows.length. Префиксы ключей
  //   ("c"/"p") спасали от коллизии КЛЮЧЕЙ, но не от области УДАЛЕНИЯ — именно поэтому
  //   уверенно звучащий комментарий скрывал баг.
  //   Вариант «добавить скоуп в reconcile» отвергнут: items.forEach вставляет узел по
  //   индексу i, так что люди встали бы ПЕРЕД чатами и порядок снова бы скакал.
  //   Правило: ОДИН писатель на контейнер (тот же класс, что M9/BA-4).
  // BA-2b: один id приходит и в chats (ЛС из корзины users), и в people — проверено
  //   q=«duster»: chats(1) id 1751101721 + people(1) тот же id. Две строки с одним id
  //   и РАЗНЫМ действием = «нажал не ту → ничего не произошло». Чат приоритетнее:
  //   он даёт переход, поэтому человек с уже показанным id не дублируется.
  const seen = new Set();
  const items = [];
  for (const it of (res.chats || [])) {
    seen.add(String(it.id));
    items.push({kind: "chat", it: it});
  }
  // F2: люди ищутся ГЛОБАЛЬНО — по всем известным чатам и инвентарю, а не только в
  // активном: «выбор любого пользователя… в разных тгк их разное количество».
  for (const it of (res.people || [])) {
    if (seen.has(String(it.id))) continue;
    items.push({kind: "person", it: it});
  }
  reconcile(box, items, x => x.kind + x.it.id, mkRow, (node, x) => {
    rowParts(node, ["t", "s"]);
    const here = String(x.it.id) === String(DATA.active);
    setClass(node, "on", here);
    if (x.kind === "chat") {
      const it = x.it;
      setText(node.querySelector(".t"), it.title || T.chat_noname);
      setText(node.querySelector(".s"),
              `${typeName(it.type)}${peoplePhrase(it.members_count || 0)}` +
              (here ? " · " + T.here : ""));
      node.onclick = () => openChat(it);
      return;
    }
    // BA-1 (жалоба 26.11: «вбиваю юзернейм, он выбирается — и снова зелёное яблоко»).
    // Было: closePanel() + flash с текстом «открой чат и отметь человека». Панель
    // закрывалась, активный чат НЕ менялся — юзер оставался на «Зелёное яблоко».
    // Явное указание: «мне нужно чтобы я мог выбираткь группы, чат, или лс» → тап по
    // человеку ПЕРЕХОДИТ к нему. open_chat (engine.py:197-230) принимает id юзера:
    // norm_candidates даёт [-100…, id, -id], голую цифру добивает fallback
    // get_users → get_chat. Права на удаление в ЛС обнуляет сервер (BA-5) — верное
    // поведение Telegram, оно показывается текстом, а не молчанием кнопки.
    const it = x.it;
    setText(node.querySelector(".t"), (it.name || T.chat_noname)
            + (it.username ? " @" + it.username : ""));
    setText(node.querySelector(".s"), `${T.search_people} · ${msgWord(it.msgs || 0)}`
            + ((it.chats || []).length > 1 ? ` · ${W.chatsWord(it.chats.length)}` : "")
            + (here ? " · " + T.here : ""));
    node.onclick = () => openChat(it);
  });
  ensureEmpty(box, items.length ? "" : T.search_none);
}

function scheduleSearch() {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(async () => {
    const q = (UI.searchQ || "").trim();
    if (!q) { UI.searchRes = null; renderSearch(); return; }
    const j = await api("/api/search?q=" + encodeURIComponent(q));
    if (j.ok) { UI.searchRes = j; renderSearch(); }
  }, 250);
}

async function openChat(it) {
  // BA-14 (26.11): поиск обязан сбрасываться при смене чата. Старый код оставлял
  // UI.searchQ/UI.searchRes, и скрытая панель хранила список результатов ЧУЖОГО
  // чата: живой док показал после перехода из ЛС в группу строки
  // ["chat-1003845264517","chat-5127336958"] при panel=null. При повторном открытии
  // юзер видит старый результат, как будто «сменилось не то» — тот же класс
  // «устаревшее значение живёт в DOM» (U-D18), только на списке.
  UI.searchQ = "";
  UI.searchRes = null;
  const sq = byId("searchQ");
  if (sq) sq.value = "";
  // BA-4: проверка несохранённого выбора — ДО сетевого вызова. Порядок важен:
  // если спросить после /api/open, чат сменился бы на сервере, а UI отказался бы
  // обновиться — худшее из состояний (смена без отражения на экране).
  if (!gotoPanel(null)) return;
  clearFlash();
  const j = await api("/api/open", {q: String(it.id)});
  if (!j.ok) { flash(byId("whereSub"), j.error || T.not_saved); return; }
  DATA = j;
  UI.whoSel = ((j.chat || {}).live || {}).targets ? (j.chat.live.targets || []).slice() : [];
  UI.pickSha = ""; UI.packStk = []; UI.packOpen = ""; UI.ask = null; UI.scope = "new";
  renderAll();
}

/* ───────── 2. КОГО ───────── */
function targetsOf() {
  // BA-10: сервер отдаёт ЭФФЕКТИВНЫЕ цели (state_blob.effective_targets): в ЛС
  // это собеседник, даже когда live.targets пуст. Читать сырой список здесь =
  // показать «не выбраны цели» и заблокировать кнопку при работающей чистке —
  // тот же класс лжи, что «чистка невозможна: нет прав» (PLAN_DM_RIGHTS §2, звено 4).
  // Fallback на live.targets остаётся для старых движков (поле может отсутствовать).
  const eff = (DATA && DATA.effective_targets) || null;
  if (Array.isArray(eff)) return eff;
  const c = (DATA && DATA.chat) || {};
  return ((c.live || {}).targets) || [];
}
function nameOf(id) {
  const ms = ((DATA && DATA.chat) || {}).members || [];
  const u = ms.find(x => x.id === id);
  if (!u) return String(id);
  return u.name || (u.username ? "@" + u.username : String(id));
}

function renderWhoRow() {
  // D1 (26.11): при открытой панели строка обязана зеркалить ЧЕРНОВИК.
  // Раньше она всегда брала серверный список, поэтому снятая галочка не давала
  // видимого результата до «готово» — отсюда «не получается убрать человека».
  // Маркер «не сохранено» обязателен: без него строка врёт в другую сторону
  // (утверждает, что цель снята, хотя сервер её держит).
  const open = UI.panel === "who";
  const ids = open ? UI.whoSel : targetsOf();
  const txt = ids.length ? ids.map(nameOf).join(", ") : T.who_none;
  const dirty = open && draftDiffers();
  setText(byId("whoMain"), shown("whoMain", dirty ? txt + " · " + T.who_dirty : txt));
  setClass(byId("whoMain"), "dim", !ids.length);
  setText(byId("whoMark"), open ? T.close : T.who_add);
}

/* Черновик отличается от сервера? Сравнение множеств, а не строк: порядок
   в черновике намеренно свободный (push), и по строковому сравнению каждое
   добавление выглядело бы как несохранённое. */
function draftDiffers() {
  const a = UI.whoSel.slice().sort((x, y) => x - y);
  const b = targetsOf().slice().sort((x, y) => x - y);
  if (a.length !== b.length) return true;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return true;
  return false;
}

/* D3-i: порядок строк вычисляется ОДИН раз при входе в панель.
   Пока панель открыта, он зафиксирован в UI.whoOrder — иначе снятая галочка
   уезжает вниз списка под палец, и следующий тап выбирает другого человека. */
function cmpWhoSeed(a, b) {
  const sa = UI.whoSel.includes(a.id) ? 1 : 0, sb = UI.whoSel.includes(b.id) ? 1 : 0;
  if (sa !== sb) return sb - sa;
  if ((b.msgs || 0) !== (a.msgs || 0)) return (b.msgs || 0) - (a.msgs || 0);
  return String(a.name || "").localeCompare(String(b.name || ""));
}
function seedWhoOrder(ms) {
  return (ms || []).slice().sort(cmpWhoSeed).map(u => u.id);
}

function seedWho() {
  UI.whoSel = targetsOf().slice();
  UI.whoSeeded = true;
  UI.whoAskClose = false;
  UI.whoOrder = seedWhoOrder(((DATA && DATA.chat) || {}).members);
}

function openWho() {
  // Подтверждение отмены (второй тап «закрыть» при несохранённом выборе) обязан
  // сработать СРАЗУ. Причина найдена живьём 26.11: естественный жест — быстрый
  // двойной тап (~150-250 мс), а guard от двойного тапа его проглатывал, и
  // сообщение «тапни ещё раз» висело после второго тапа — то есть ровно тот же
  // симптом D3 («нажал — то же окно»), который мы этим фиксом лечим.
  // Guard защищает ПЕРЕКЛЮЧЕНИЕ панели; подтверждение — не переключение.
  const confirming = UI.panel === "who" && UI.whoAskClose && draftDiffers();
  if (!confirming && !tapOK("who")) return;
  if (UI.panel === "who") { closePanel(); return; }
  UI.whoFilter = "";
  byId("whoFilter").value = "";
  seedWho();
  gotoPanel("who");
}

/* D2 (26.11, вариант B1): закрытие с несохранённым выбором НЕ теряет его молча.
   Первый тап «закрыть» спрашивает, второй отменяет черновик. Опасное действие
   (ДОБАВЛЕНИЕ цели = включить удаления этого человека) невозможно случайным
   закрытием; защитное (снятие) применяется сразу, без подтверждения.
   Возвращает true = панель надо закрыть, false = остались открыты (ждём второй тап). */
/* D2/BA-4 (26.11): уход из панели «ЦЕЛИ» с несохранённым выбором — ЕДИННАЯ точка.
   closePanel() закрывал панель и спрашивал подтверждение, но СМЕНА панели
   (rowWhere:262, moreList:343) и смена чата (openChat:460) звали setPanel напрямую
   и теряли черновик молча. Проверено чтением кода: при уходе кто-либо из них
   `UI.whoSeeded` оставался true, а следующий вход в openWho() делает seedWho()
   из сервера — то есть выбор юзера исчезает без единого вопроса.
   Достижимо живьём: открыть «ЦЕЛИ», добавить человека тапом (без «готово»),
   нажать «ГДЕ» → список сбрасывается.
   Возвращает false = остаться на текущей панели (запрошено подтверждение). */
function leaveWho(target) {
  if (UI.panel !== "who" || target === "who" || !draftDiffers()) return true;
  if (!UI.whoAskClose) {
    UI.whoAskClose = true;
    renderWho();
    // Текст обязан быть на ВИДИМОМ поле: при уходе на другую панель #whoNote
    // скрывается вместе с ней, и юзер не увидел бы, чего от него хотят.
    // #whoMain — строка «ЦЕЛИ», она видна всегда.
    flash(byId("whoMain"), T.who_close_dirty(T.close));
    return false;
  }
  resetWhoDraft();
  return true;
}

/* Черновик отменён: состояние и видимый note приходят в соответствие с сервером
   сразу. Отдельная функция, потому что путей ухода несколько, а две копии одной
   логики разъезжаются (M9-класс). */
function resetWhoDraft() {
  UI.whoAskClose = false;
  UI.whoSel = [];
  UI.whoSeeded = false;
  setText(byId("whoNote"), targetWord(targetsOf().length));
}

/* Единственная точка ПЕРЕКЛЮЧЕНИЯ панели: закрытие = gotoPanel(null).
   Вне её setPanelhas права не быть ни у кого — иначе появляется путь, минующий
   подтверждение несохранённого выбора. */
function gotoPanel(name, persist) {
  if (!leaveWho(name)) return false;
  setPanel(name, persist);
  return true;
}

function closePanel() {
  return gotoPanel(null);
}

function renderWho() {
  const c = (DATA && DATA.chat) || {};
  const ms = (c.members || []).slice();
  const f = (UI.whoFilter || "").trim().toLowerCase().replace(/^@/, "");
  const rows = ms.filter(u => {
    if (!f) return true;
    return (`${u.name || ""} ${u.username || ""} ${u.id}`).toLowerCase().includes(f);
  });
  // U-D16 + D3-i (26.11): сортировка «цели первыми» УБРАНА из перерисовки.
  // Она переселяла снятую строку вниз списка под палец (док 26.11: 1751101721 был
  // 2-м, стал 3-м), а полл каждые 4 с дёргал порядок по msgs. Порядок теперь
  // вычислен один раз при входе (UI.whoOrder); новых участников дописываем в конец.
  const rank = new Map(UI.whoOrder.map((id, i) => [id, i]));
  const tail = UI.whoOrder.length;
  rows.sort((a, b) => {
    const ra = rank.has(a.id) ? rank.get(a.id) : tail;
    const rb = rank.has(b.id) ? rank.get(b.id) : tail;
    if (ra !== rb) return ra - rb;
    return String(a.name || "").localeCompare(String(b.name || ""));
  });
  const box = byId("whoList");
  reconcile(box, rows, u => String(u.id), () => {
    const b = mkRow();
    b.appendChild(mkSpan("box"));
    rowParts(b, ["t", "s"]);
    return b;
  }, (node, u) => {
    rowParts(node, ["box", "t", "s"]);
    const on = UI.whoSel.includes(u.id);
    const me = u.id === DATA.me_id;
    setClass(node, "on", on);
    setClass(node, "self", me);
    setText(node.querySelector(".t"),
            (u.name || T.chat_noname) + (me ? " · " + T.who_me : ""));
    setText(node.querySelector(".s"),
            `${roleName(u.status)} · ${msgWord(u.msgs || 0)}`);
    setText(node.querySelector(".box"), on ? "✓" : "");
    node.onclick = async () => {
      if (me) { flash(byId("whoNote"), T.who_not_self); return; }
      // D3-ii: двойной тап по строке раньше делал «снять+добавить» = визуально ничего
      if (!tapOK("row:" + u.id)) return;
      clearFlash();
      const k = UI.whoSel.indexOf(u.id);
      if (k < 0) {
        // ДОБАВЛЕНИЕ цели — разрушающее: оно включает удаления этого человека,
        // поэтому требует явного «готово» и нигде не применяется молча.
        UI.whoSel.push(u.id);
        UI.whoAskClose = false;
        renderWho();
        return;
      }
      // СНЯТИЕ цели — только прекращает удаления, значит безопасно и сразу:
      // это и есть ответ на жалобу «не получается убрать человека».
      UI.whoSel.splice(k, 1);
      UI.whoAskClose = false;
      renderWho();
      await applyTargets();
    };
  });
  ensureEmpty(box, rows.length ? "" : T.search_none);
  // D2: при запросе подтверждения закрытия note обязан говорить именно это,
  // а не «2 цели» — иначе второй тап неотличим от обычного.
  // BA-10: в личной переписке цель не выбирается из списка (собеседник = цель по
  // определению), поэтому показываем это, а не «выбери, чьи сообщения удалять».
  const dmHint = ((DATA && DATA.chat) || {}).dm;
  const note = (UI.whoAskClose && draftDiffers()) ? T.who_close_dirty(T.close)
                 : dmHint ? T.power_dm_hint
                 : targetWord(UI.whoSel.length);
  setText(byId("whoNote"), shown("whoNote", note));
  // D1/D3-iii: список без этих двух вызовов живёт отдельно от итоговой строки и
  // счётчика — галочка снималась, а «ЦЕЛИ · 2» и имена не менялись до 4 секунд.
  renderWhoRow();
  renderCounts();
}

/* Отправить черновик на сервер. Общий путь для «готово» и для мгновенного снятия:
   два места с одинаковой логикой обязательно разъехались бы (M9-класс бага). */
async function applyTargets() {
  const j = await api("/api/targets", {ids: UI.whoSel});
  if (!j.ok) { flash(byId("whoNote"), j.error || T.not_saved); return false; }
  DATA = j;
  // Сервер может разрешить запись И одновременно пояснить её («себя снял из целей»).
  // Раньше note бросался: проверка шла только по j.ok, поэтому отказ с человеческим
  // текстом и успех с пояснением выглядели одинаково молча (BA-7, 26.11).
  if (j.note) flash(byId("whoNote"), j.note);
  renderWho();
  renderPower();
  renderStatus();
  renderMore();
  return true;
}

async function saveTargets() {
  if (!tapOK("whoDone")) return;
  const n = UI.whoSel.length;
  if (await applyTargets()) {
    UI.whoSeeded = false;      // черновик применён — при следующем входе засев заново
    UI.whoAskClose = false;
    flash(byId("whoNote"), T.who_saved(n));
    renderWho();
    renderCounts();
  }
}

/* ───────── 3. ЧТО ───────── */
function setScope(m) {
  if (UI.scope === m) return;
  clearFlash();
  UI.scope = m;
  renderWhat();
  if (m === "all") askSweep();
  else cancelSweep();
}

function renderWhat() {
  const c = (DATA && DATA.chat) || {};
  const n = c.last_count;
  setText(byId("whatNew"), T.what_new);
  setText(byId("whatAll"),
          (typeof n === "number" && n > 0) ? T.what_all_count(n) : T.what_all);
  setAttr(byId("whatNew"), "aria-checked", UI.scope === "new" ? "true" : "false");
  setAttr(byId("whatAll"), "aria-checked", UI.scope === "all" ? "true" : "false");
}

async function askSweep() {
  if (!targetsOf().length) { flash(byId("sweepNote"), T.who_none); return; }
  UI.ask = {found: null, scanned: 0, all: false, job: null};
  renderSweepBox();
  const j = await api("/api/count", {all: false, limit: 200});
  if (!j.ok) { UI.ask = null; flash(byId("sweepNote"), j.error || T.not_saved); renderSweepBox(); return; }
  UI.ask.job = j.job;
  pollCount();
}

function cancelSweep() {
  clearTimeout(countTimer);
  if (UI.ask && UI.ask.found) {
    const c = DATA && DATA.chat;
    if (c) c.last_count = UI.ask.found;
  }
  UI.ask = null;
  renderSweepBox();
  renderWhat();
}

async function pollCount() {
  clearTimeout(countTimer);
  if (!UI.ask || !UI.ask.job) return;
  const j = await api("/api/count/result?job=" + UI.ask.job);
  if (!j.ok) { UI.ask = null; renderSweepBox(); return; }
  UI.ask.found = j.found || 0;
  UI.ask.scanned = j.scanned || 0;
  UI.ask.all = !!j.all;
  renderSweepBox();
  if (j.state === "run") { countTimer = setTimeout(pollCount, 500); return; }
  if (DATA && DATA.chat) DATA.chat.last_count = UI.ask.found;
  renderWhat();
}

function renderSweepBox() {
  const show = !!UI.ask;
  setHidden(byId("panelSweep"), !show);
  setHidden(byId("power"), show);      // подтверждение ЗАМЕЩАЕТ строку пуска (§3)
  if (!show) return;
  const f = UI.ask.found;
  setText(byId("sweepAsk"),
          f === null ? T.sweep_counting : (f ? T.sweep_ask(f) : T.sweep_nothing));
  setHidden(byId("sweepGo"), !f);
  if (f) setText(byId("sweepGo"), T.sweep_go(f));
  const parts = [T.sweep_irreversible];
  if (UI.ask.scanned) parts.push(T.sweep_scanned(UI.ask.scanned));
  if (UI.ask.all) parts.push(T.sweep_scope_all);
  setText(byId("sweepNote"), shown("sweepNote", parts.join(" · ")));
}

async function doSweep() {
  if (UI.busy) return;
  clearFlash();
  if (!UI.ask || !UI.ask.found) return;
  UI.busy = true;
  setText(byId("sweepGo"), T.sweep_doing);
  const j = await api("/api/sweep", {confirm: true, all: !!UI.ask.all, limit: 200});
  UI.busy = false;
  if (!j.ok) { flash(byId("sweepNote"), j.error || T.not_saved); renderSweepBox(); return; }
  const removed = j.removed || 0;
  UI.ask = null; UI.scope = "new";
  if (DATA && DATA.chat) DATA.chat.last_count = 0;
  await refresh();
  renderAll();
  flash(byId("status"), T.sweep_done(removed));
}

/* ───────── 4. ПУСК + статус ───────── */
function powerBlocked() {
  if (!DATA || !DATA.chat) return T.power_no_chat;
  const r = (DATA.chat.rights) || {};
  if (!r.can_delete || DATA.can_act === false) {
    return T.power_no_rights + (DATA.can_act_why ? " · " + DATA.can_act_why : "");
  }
  if (!targetsOf().length) return T.power_no_targets;
  return "";
}

function renderPower() {
  const b = byId("power");
  const blocked = powerBlocked();
  const live = !!(DATA && DATA.chat && (DATA.chat.live || {}).enabled);
  setClass(b, "run", live && !blocked);
  setClass(b, "off", !!blocked);
  setText(b, blocked ? blocked : (live ? T.power_stop : T.power_start));
  b.dataset.mode = blocked ? "blocked" : (live ? "run" : "idle");
}

async function onPower() {
  if (byId("power").dataset.mode === "blocked") {
    if (!targetsOf().length && DATA && DATA.chat) openWho();
    return;
  }
  const live = !!(DATA && DATA.chat && (DATA.chat.live || {}).enabled);
  const want = !live;
  const b = byId("power");
  clearFlash();
  b.style.opacity = ".6";
  const j = await api("/api/live", {enabled: want});
  b.style.opacity = "";
  if (!j.ok) { flash(byId("status"), j.error || T.not_saved); return; }
  DATA = j;
  renderAll();
  if (want && UI.scope === "all") askSweep();
}

function renderStatus() {
  const n = byId("status");
  if (UI.dead || !DATA) { setText(n, T.status_dead + " · " + T.status_dead_how); return; }
  const live = !!(DATA.chat && (DATA.chat.live || {}).enabled);
  const d = DATA.demon || {};
  const del = d.deleted || 0;
  const at = d.started || "";
  const base = del ? T.status_deleted(del, at) : (live ? T.status_live(at) : T.status_idle);
  setText(n, shown("status", base));
}

/* ───────── ещё: аватар и описание ───────── */
function renderLook() {
  const c = (DATA && DATA.chat) || {};
  const refs = c.refs || {}, w = c.watch || {};
  setText(byId("lookAbout"), (c.about && c.about.trim()) ? c.about : T.look_about_empty);
  const av = refs.avatar || {};
  setText(byId("lookRef"), av.saved_at ? T.look_ref_when(av.saved_at) : T.look_ref_none);
  const la = byId("lookAva");
  const url = "/api/ref/img?t=" + (c.avatar_stamp || 0);
  if (la.dataset.url !== url) {
    la.dataset.url = url;
    const probe = new Image();
    probe.onload = () => setStyle(la, "backgroundImage", `url(${url})`);
    probe.onerror = () => setStyle(la, "backgroundImage", "");
    probe.src = url;
  }
  const rows = [["avatar", T.look_avatar], ["about", T.look_about], ["title", T.look_title_name]];
  const box = byId("lookList");
  reconcile(box, rows, r => r[0], () => {
    const b = mkRow();
    rowParts(b, ["t", "s"]);
    const tg = document.createElement("button");
    tg.className = "tgl"; tg.appendChild(document.createElement("i"));
    b.appendChild(tg);
    return b;
  }, (node, r) => {
    rowParts(node, ["t", "s", "tgl"]);
    const k = r[0], on = !!w[k], armed = !!(refs[k] || {}).armed;
    setText(node.querySelector(".t"), r[1]);
    setText(node.querySelector(".s"),
            `${on ? T.look_watch_on : T.look_watch_off} · ${armed ? T.look_restore_on : T.look_restore_off}`);
    setClass(node.querySelector(".tgl"), "on", on);
    node.querySelector(".tgl").onclick = async (ev) => {
      ev.stopPropagation();
      clearFlash();
      const j = await api("/api/watch", {[k]: !on});
      if (!j.ok) { flash(byId("lookNote"), j.error || T.not_saved); return; }
      DATA = j; renderLook(); renderStatus();
    };
  });
  ensureEmpty(box, "");
  setText(byId("lookNote"),
          shown("lookNote", `${T.look_how_arm} ${T.look_avatar}/${T.look_about}/${T.look_title_name}`));
}

/* ───────── ещё: ответ стикером ───────── */
function renderRules() {
  const c = (DATA && DATA.chat) || {};
  const rs = c.reply_rules || [];
  const ms = c.members || [];
  const box = byId("ruleList");
  reconcile(box, rs, r => r.id, () => {
    const b = mkRow();
    rowParts(b, ["t", "s"]);
    const tg = document.createElement("button");
    tg.className = "tgl"; tg.appendChild(document.createElement("i"));
    b.appendChild(tg);
    return b;
  }, (node, r) => {
    rowParts(node, ["t", "s", "tgl"]);
    setText(node.querySelector(".t"), nameOf(r.who));
    setText(node.querySelector(".s"),
            `${W.sentWord(r.sent_total || 0)} · ${r.mode === "reply" ? T.rules_mode_short_reply : T.rules_mode_short_plain} · ${T.rules_rate_short(r.max_per_min)}`);
    setClass(node.querySelector(".tgl"), "on", !!r.enabled);
    node.querySelector(".tgl").onclick = async (ev) => {
      ev.stopPropagation();
      clearFlash();
      const j = await api("/api/rules", {op: "toggle", id: r.id, enabled: !r.enabled});
      if (!j.ok) { flash(byId("ruleNote"), j.error || T.not_saved); return; }
      DATA = j; renderRules();
    };
    node.onclick = async () => {
      setText(node.querySelector(".s"), T.rules_testing);
      const j = await api("/api/rules/test", {rule_id: r.id});
      flash(byId("ruleNote"), j.ok ? T.rules_test_ok : `${T.rules_test_fail} · ${j.error || j.skipped || ""}`);
      if (j.ok) { DATA = await api("/api/state"); renderRules(); }
      else renderRules();
    };
  });
  ensureEmpty(box, rs.length ? "" : T.rules_none);

  const seen = (DATA.packs_seen) || [];
  const pbox = byId("packList");
  reconcile(pbox, seen, p => p.short_name, mkRow, (node, p) => {
    rowParts(node, ["t", "s"]);
    setText(node.querySelector(".t"), p.short_name);
    setText(node.querySelector(".s"),
            UI.packOpen === p.short_name ? W.stickWord((UI.packStk || []).length) : "");
    setClass(node, "on", UI.packOpen === p.short_name);
    node.onclick = async () => {
      setText(node.querySelector(".t"), T.packs_loading);
      const j = await api("/api/stk/pack", {short_name: p.short_name});
      if (!j.ok) { flash(byId("ruleNote"), j.error || T.not_saved); renderRules(); return; }
      UI.packOpen = p.short_name; UI.packStk = j.stickers || [];
      renderRules();
    };
  });
  ensureEmpty(pbox, seen.length ? "" : T.packs_none);

  const g = byId("stkGrid");
  reconcile(g, UI.packStk, s => s.sha256, () => {
    const d = document.createElement("button");
    d.className = "cell";
    const lb = document.createElement("div"); lb.className = "c";
    d.appendChild(lb);
    return d;
  }, (node, s) => {
    let img = node.querySelector("img");
    if (s.kind === "static") {
      if (!img) { img = document.createElement("img"); node.insertBefore(img, node.firstChild); }
      if (img.dataset.src !== s.sha256) { img.dataset.src = s.sha256; img.src = "/api/stk/file/" + s.sha256; }
    } else if (img) { node.removeChild(img); }
    setText(node.querySelector(".c"),
            s.kind === "static" ? (s.emoji || "") : (s.kind === "video" ? T.sticker_video : T.sticker_animated));
    setClass(node, "on", UI.pickSha === s.sha256);
    node.onclick = () => { UI.pickSha = s.sha256; renderRules(); };
  });

  const cand = ms.filter(u => u.id !== DATA.me_id);
  fillSelect(byId("ruleWho"),
             [["", T.rules_who_ph]].concat(cand.map(u => [String(u.id),
              (u.name || T.chat_noname) + (u.username ? " @" + u.username : "")])));

  const btn = byId("ruleCreate");
  const who = byId("ruleWho").value;
  const mode = byId("ruleMode").value;
  // F1 mirror: байты берутся из ВХОДЯЩЕГО стикера, поэтому выбор из хранилища не нужен.
  // Скрывать выбор надо вместе с причиной, иначе юзер ищет «почему нельзя выбрать».
  const needSticker = mode !== "mirror";
  setHidden(byId("packList"), !needSticker);
  setHidden(byId("stkGrid"), !needSticker);
  setHidden(byId("rulesNewLbl"), !needSticker);
  let reason = "";
  if (needSticker && !seen.length) reason = T.packs_none;
  else if (needSticker && !UI.packOpen) reason = T.packs_pick_first;
  else if (needSticker && !UI.packStk.length) reason = T.packs_loading;
  else if (needSticker && !UI.pickSha) reason = T.rules_need_sticker;
  else if (!who) reason = T.rules_need_who;
  else if (mode === "mirror" && byId("ruleTrigger").value !== "sticker") reason = T.rules_mirror_no_sticker;
  setHidden(btn, !!reason);          // U-D10: мёртвой кнопки нет — есть причина
  setText(byId("ruleNote"), shown("ruleNote", reason));
  if (!reason) setText(btn, T.rules_create);
}

async function createRule() {
  clearFlash();
  const who = byId("ruleWho").value;
  const cd = byId("ruleCd").value.trim();
  const mx = byId("ruleRate").value.trim();
  const j = await api("/api/rules", {
    who: Number(who), sha256: UI.pickSha,
    trigger: byId("ruleTrigger").value, mode: byId("ruleMode").value,
    cooldown_sec: cd === "" ? 20 : Number(cd),
    max_per_min: mx === "" ? 2 : Number(mx),
    pack: UI.packOpen,
  });
  if (!j.ok) { flash(byId("ruleNote"), j.error || T.not_saved); return; }
  DATA = j; UI.pickSha = "";
  flash(byId("ruleNote"), T.saved);
  renderRules(); renderCounts();
}

/* ───────── ещё: журнал ───────── */
function renderLog() {
  const d = (DATA && DATA.demon) || {};
  const rows = (d.events || []).slice(0, 20);
  const box = byId("logList");
  reconcile(box, rows, (e, i) => "e" + i, () => {
    const b = document.createElement("div");
    b.className = "item";
    rowParts(b, ["t", "s"]);
    return b;
  }, (node, e, i) => {
    rowParts(node, ["t", "s"]);
    setText(node.querySelector(".t"), e.text || "");
    setText(node.querySelector(".s"), e.t || "");
    setClass(node, "no", e.ok === false);
  });
  ensureEmpty(box, rows.length ? "" : T.log_empty);
  setText(byId("logMissed"), T.log_missed(d.missed || 0));
}

/* F5: единственный способ доказать живой пайплайн без сообщения от цели.
   Полигон — Saved Messages: посторонние не видят, чистка в группе не трогается. */
async function runSelftest() {
  if (UI.busy) return;
  UI.busy = true;
  clearFlash();
  const btn = byId("logRun");
  setText(btn, T.log_running);
  const j = await api("/api/selftest", {});
  UI.busy = false;
  setText(btn, T.log_run);
  if (j.ok) flash(byId("logMissed"), `${T.log_ok} · ${T.log_ms(j.ms || 0)}`);
  else flash(byId("logMissed"), `${T.log_fail} · ${j.error || j.skipped || ""}`);
  await refresh();
}

/* ───────── ещё: чаты (грузятся отдельно: 863 строки не должны ехать в опросе) ───────── */
let CHATS = null;

async function loadChats() {
  if (CHATS) return;
  setText(byId("chatsNote"), T.chats_reloading(0));
  const j = await api("/api/chats");
  if (j.ok) { CHATS = j.chats || {}; renderChats(); }
  else flash(byId("chatsNote"), j.error || T.not_saved);
}

function renderChats() {
  const inv = CHATS || {};
  const rows = [];
  for (const k of ["groups", "channels", "bots", "users"]) {
    for (const r of (inv[k] || [])) rows.push(r);
  }
  const box = byId("chatsList");
  reconcile(box, rows, r => String(r.id), mkRow, (node, r) => {
    rowParts(node, ["t", "s"]);
    setText(node.querySelector(".t"), r.title || T.chat_noname);
    const bits = [typeName(r.type) + peoplePhrase(r.members_count || 0)];
    if (r.cleaning) bits.push(T.chats_clean_on);
    if (r.targets) bits.push(T.chats_targets(r.targets));
    if (String(r.id) === String(DATA.active)) bits.push(T.chats_here);
    setText(node.querySelector(".s"), bits.filter(Boolean).join(" · "));
    setClass(node, "on", String(r.id) === String(DATA.active));
    node.onclick = () => openChat(r);
  });
  ensureEmpty(box, rows.length ? "" : T.chats_none);
  const busy = UI.invBusy;
  setText(byId("chatsReload"),
          busy ? T.chats_reloading((DATA.inventory_counts || {}).read || 0) : T.chats_reload);
  setText(byId("chatsNote"), shown("chatsNote", UI.invNote || ""));
}

async function reloadInventory() {
  if (UI.invBusy) return;
  clearFlash();
  UI.invBusy = true; renderChats();
  const j = await api("/api/inventory", {});
  if (!j.ok) { UI.invBusy = false; UI.invNote = j.error || T.not_saved; renderChats(); return; }
  const tick = async () => {
    const r = await api("/api/count/result?job=" + j.job);
    if (r.ok && r.state === "run") {
      if (DATA) DATA.inventory_counts = Object.assign({}, DATA.inventory_counts, {read: r.scanned || 0});
      renderChats();
      setTimeout(tick, 1500);
    } else {
      UI.invBusy = false;
      CHATS = null;                      // список устарел — тянем заново с сервера
      await refresh();
      await loadChats();
      UI.invNote = r.ok ? T.saved : (r.note || T.not_saved);
      renderChats();
    }
  };
  tick();
}

/* ───────── ещё: неполадки ───────── */
function renderErr() {
  const d = (DATA && DATA.demon) || {};
  const rows = d.errors || [];
  const box = byId("errList");
  reconcile(box, rows, (e, i) => "x" + i, () => {
    const b = document.createElement("div");
    b.className = "item";
    rowParts(b, ["t"]);
    return b;
  }, (node, e, i) => {
    rowParts(node, ["t"]);
    setText(node.querySelector(".t"), `${e.t || ""} ${e.where || ""}: ${e.e || ""}`);
  });
  ensureEmpty(box, rows.length ? "" : T.err_none);
}

/* ───────── счётчики: число только когда оно >1 (§2 «ЛЮДИ 1» запрещено) ───────── */
function renderCounts() {
  const n = targetsOf().length;
  setText(byId("whoLabel"), n > 1 ? `${T.who_label} · ${n}` : T.who_label);
  setText(byId("rulesNewLbl"), T.rules_new);
}

/* ───────── общий рендер: только точечные правки ───────── */
function renderAll() {
  renderWhere();
  renderWhoRow();
  renderWhat();
  renderPower();
  renderStatus();
  renderSweepBox();
  renderMore();
  renderCounts();
  renderErr();
  if (UI.panel === "who") renderWho();
  if (UI.panel === "where") renderSearch();
  if (UI.panel === "look") renderLook();
  if (UI.panel === "rules") renderRules();
  if (UI.panel === "log") renderLog();
  if (UI.panel === "chats") renderChats();
  if (UI.panel === "err") renderErr();
}

function renderDead() {
  UI.dead = true;
  setText(byId("whereTitle"), T.status_dead);
  setText(byId("whereSub"), T.status_dead_how);
  setText(byId("power"), T.status_dead);
  setClass(byId("power"), "run", false);
  setClass(byId("power"), "off", true);
  byId("power").dataset.mode = "blocked";
  setText(byId("status"), T.status_dead_how);
}

/* ───────── опрос: 4 с, пауза в фоне ───────── */
async function refresh() {
  const j = await api("/api/state");
  if (!j.ok) { renderDead(); return; }
  UI.dead = false;
  DATA = j;
  renderAll();
}

function schedulePoll() {
  clearTimeout(pollTimer);
  if (document.hidden) return;
  pollTimer = setTimeout(async () => {
    await refresh();
    schedulePoll();
  }, 4000);
}

document.addEventListener("visibilitychange", () => {
  if (document.hidden) clearTimeout(pollTimer);
  else { refresh(); schedulePoll(); }
});

/* ───────── старт ───────── */
mount();
(async () => {
  await refresh();
  const p = DATA && DATA.ui && DATA.ui.panel;
  if (p && DEEP[p]) gotoPanel(p, false);      // U-D12: открытая панель переживает перезагрузку
  setText(byId("more"), T.more);
  schedulePoll();
})();
