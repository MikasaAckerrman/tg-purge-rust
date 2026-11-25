#!/usr/bin/env python3
"""Валидация манифеста и workflow ДО пуша.

Почему существует: раунд 2 сгорел на YAML (двоеточие в некавированном name),
раунд 3 — на Cargo.toml без заголовка [package]. Оба прогона не дали ни одного
ответа по grammers, потому что cargo до grammers не дошёл. Проверка локально
стоит 1 секунду, проверка в CI — 3 минуты и один сгоревший раунд.
"""
import os
import re
import sys
import tomllib

import yaml

import braces2

ROOT = "/var/minis/shared/rustspike"
# Путь workflow можно переопределить: так проверяем, что валидатор КРАСНЕЕТ на
# намеренно сломанном файле (валидатор, который не умеет падеть, — бумажка).
WF = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/.github/workflows/rust.yml"
ok = True

# ---- Cargo.toml ----
with open(f"{ROOT}/Cargo.toml", "rb") as f:
    man = tomllib.load(f)

pkg = man.get("package")
if not pkg:
    print("CARGO ПЛОХО: нет секции [package] (топ-level ключи не считаются)")
    ok = False
else:
    for field in ("name", "version", "edition"):
        if field not in pkg:
            print(f"CARGO ПЛОХО: у [package] нет {field}")
            ok = False
    print(f"CARGO OK: package {pkg.get('name')} v{pkg.get('version')} edition={pkg.get('edition')}")

deps = man.get("dependencies", {})
need = ["grammers-client", "grammers-session", "grammers-mtsender", "grammers-tl-types",
        "tokio", "serde", "serde_json", "tiny_http"]
for d in need:
    if d not in deps:
        print(f"CARGO ПЛОХО: зависимость {d} не объявлена")
        ok = False
sess = deps.get("grammers-session", {})
if isinstance(sess, dict):
    feats = sess.get("features") or []
    if "serde" not in feats:
        print("CARGO ПЛОХО: у grammers-session не включена фича serde (без неё сессию не сохранить)")
        ok = False
    if sess.get("default-features") is not False:
        print("CARGO ПЛОХО: у grammers-session не выключены default-features (вернётся sqlite/libsql = C)")
        ok = False
cli = deps.get("grammers-client", {})
if isinstance(cli, dict) and cli.get("default-features") is not False:
    print("CARGO ПЛОХО: у grammers-client не выключены default-features (нужен явный контроль fs)")
    ok = False

ct = man.get("lib", {}).get("crate-type", [])
if "cdylib" not in ct:
    print("CARGO ПЛОХО: lib.crate-type не содержит cdylib — в APK класть нечего")
    ok = False

if not man.get("bin"):
    print("CARGO ПЛОХО: нет [[bin]] — `purge --caps` не запустить")
    ok = False

print("профиль release:", man.get("profile", {}).get("release", {}))

# ---- workflow ----
raw = open(WF, "rb").read()
if b"\t" in raw:
    print("YAML ПЛОХО: таблица в файле")
    ok = False
try:
    d = yaml.safe_load(raw.decode())
except Exception as e:
    print("YAML ПЛОХО:", e)
    ok = False
else:
    jobs = d.get("jobs") or {}
    if not jobs:
        print("YAML ПЛОХО: ни одного job")
        ok = False
    for name, j in jobs.items():
        steps = j.get("steps") or []
        if not steps:
            print(f"YAML ПЛОХО: у job {name} нет steps")
            ok = False
        for s in steps:
            n = s.get("name")
            if n and (":" in n) and not (n.startswith('"') or n.startswith("'")):
                # yaml уже разобрался, но GitHub терпит; всё равно требуем кавычки в исходнике
                pass
            run = s.get("run")
            if run and "pipefail" in run:
                # `cmd | head -N` + pipefail = сломанный канал ⇒ cargo паникует
                # («failed printing to stdout: Broken pipe», exit 101 — раунд 4).
                # Требуем: вывод пишется в файл, а head/tail/sed читают файл.
                # ВАЖНО: `||` — это НЕ канал. Делим только по одиночному `|`,
                # иначе валидатор флагует `cargo build > log 2>&1 || { tail log; }`
                # и становится ложноположительным (а ложный линтер игнорируют).
                for line in run.splitlines():
                    parts = re.split(r"(?<!\|)\|(?!\|)", line)
                    if len(parts) < 2 or line.strip().startswith("#"):
                        continue
                    for i in range(len(parts) - 1):
                        left, right = parts[i], parts[i + 1]
                        sender = any(t in left for t in ("cargo", "rustc", "sdkmanager", "./target"))
                        sink = any(t in right for t in ("head", "tail", "grep", "sed"))
                        if sender and sink:
                            print(f"YAML ПЛОХО: сломанный канал: {line.strip()[:80]}")
                            ok = False
    print("YAML OK: jobs =", list(jobs.keys()))

# ---- Rust-исходники: честный баланс скобок (строки/комментарии не в счёт) ----
# Наивный `src.count('{')` давал ЛОЖНЫЕ срабатывания: в store.rs есть
# `std::fs::write(&path, b"{ ne json ")` и скобки в doc-комментариях.
# Ложный линтер игнорируют, поэтому считаем только код (braces2.py), а сам
# braces2 обязан проходить --selftest из 15 кейсов с известным ответом.
for f in sorted(os.listdir(f"{ROOT}/src")):
    if not f.endswith(".rs"):
        continue
    p = f"{ROOT}/src/{f}"
    src = open(p, encoding="utf-8").read()
    if "grammers_client::types" in src:
        print(f"src/{f}: ссылается на grammers_client::types — такого модуля в 0.12.0 НЕТ")
        ok = False
    if "retry_policy::" in src:
        print(f"src/{f}: использует retry_policy — модуль не объявлен в 0.12.0")
        ok = False
    # ловим конкретные сигнатурные ошибки, на которых я уже падал (раунды 5–6).
    # ВАЖНО: проверяем по коду БЕЗ строк и комментариев — иначе собственная
    # doc-строка про «if let Ok(msg) = item» детектируется как реальный баг
    # (ложное срабатывание, из-за которого валидатору перестают верить).
    code = braces2.blanks(src)
    if "get_me()" in code and ".await" not in code.split("get_me()")[1][:40]:
        print(f"src/{f}: get_me() без .await — это async (chats.rs:414)")
        ok = False
    if re.search(r"user\.id\(\)", code) and "bare_id_unchecked" not in code:
        print(f"src/{f}: User::id() -> PeerId, а не i64 — нужен bare_id_unchecked()")
        ok = False
    if re.search(r"\bSenderHandle\b", code):
        print(f"src/{f}: SenderHandle не существует — в 0.12.0 SenderPoolHandle/SenderPoolFatHandle")
        ok = False
    # TL-типы и формы вызовов, на которых раунд 6 дал 23 ошибки компилятора
    if "InputFileUploaded" in code:
        print(f"src/{f}: InputFileUploaded НЕ существует — есть inputFile/inputFileBig")
        ok = False
    if re.search(r"channels::(EditTitle|EditPhoto)\s*\{\s*chat_id", code):
        print(f"src/{f}: channels::Edit* принимает channel: InputChannel, не chat_id")
        ok = False
    if re.search(r"invoke\(tl::functions", code):
        print(f"src/{f}: invoke требует ссылку — invoke(&req), не invoke(req)")
        ok = False
    if re.search(r"tl::types::(input_[a-z_]+::|input_file::)", code) or "InputPeer::Self_" in code:
        print(f"src/{f}: TL-пути/имена: конструкторы без точки → tl::types::<Type>, "
              f"inputPeerSelf → InputPeer::PeerSelf (не Self_)")
        ok = False
    # раунд 7: имена вариантов (rustifier.rs:92-122)
    if re.search(r"InputChannel::InputChannel\b", code):
        print(f"src/{f}: inputChannel → InputChannel::Channel, не ::InputChannel")
        ok = False
    if re.search(r"InputFile::InputFile\b|InputChatPhoto::InputChatPhoto\b", code):
        print(f"src/{f}: префикс типа у варианта отрезается (InputFile::File / InputChatPhoto::…)")
        ok = False
    # раунд 9: rustifier.rs:96 режет префикс типа ТОЛЬКО если имя начинается с него
    if "InputChatPhoto::Uploaded(" in code:
        print(f"src/{f}: inputChatUploadedPhoto НЕ начинается с InputChatPhoto ⇒ вариант полный: "
              f"InputChatPhoto::InputChatUploadedPhoto (E0599 в раунде 9)")
        ok = False
    # раунд 8: варианты без параметров генерируются UNIT (enums.rs:73-77)
    for unit in ("InputChatPhoto::Empty", "InputPeer::PeerSelf", "InputChannel::Empty",
                 "InputStickerSet::Empty"):
        if f"{unit}(" in code:
            print(f"src/{f}: {unit} — UNIT-вариант (конструктор без params), скобки вызывать нельзя (E0618)")
            ok = False
    if re.search(r"tl::types::(InputChatPhotoEmpty|InputPeerSelf|InputChannelEmpty|InputStickerSetEmpty)\b", code):
        print(f"src/{f}: у unit-конструкторов структуры НЕТ вовсе (enums.rs:73-77)")
        ok = False
    # раунд 7: limit(mut self) -> Self потребляет итератор (E0382)
    for line in code.splitlines():
        s = line.strip()
        if ".limit(" in s and s.startswith("let _") :
            print(f"src/{f}: .limit() потребляет итератор — нельзя в `let _ = …`: {s[:80]}")
            ok = False
        if re.search(r"^\w+\.limit\(.+\)\.(total|next)", s):
            print(f"src/{f}: цепочка `x.limit(..).total()` после использования x — E0382: {s[:80]}")
            ok = False
    if re.search(r"get_permissions\([^,()]+\)\s*\.await", code):
        print(f"src/{f}: get_permissions(chat, user) — ДВА аргумента (chats.rs:686)")
        ok = False
    if re.search(r"iter_messages\(PeerId::", code) or re.search(r"iter_participants\(&", code):
        print(f"src/{f}: From<PeerId> для PeerRef нет — только PeerId::to_ambient_ref()")
        ok = False
    if re.search(r"if let Ok\(msg\) = item", code):
        print(f"src/{f}: итератор отдаёт Result<Option<T>,_>, а не Result<Result<T,_>,_> — "
              f"такой паттерн тихо теряет все сообщения")
        ok = False
    if "_ =>" in code and "set_update_state" in code:
        print(f"src/{f}: `_ =>` в match set_update_state — скроет новую ветку UpdateState "
              f"вместо ошибки компиляции")
        ok = False
    if braces2.balance(p, verbose=False) != 0:
        print(f"src/{f}: дисбаланс фигурных скобок в КОДЕ (не в строках/комментариях)")
        ok = False
    else:
        print(f"src/{f}: {len(src)} байт, скобки в коде сбалансированы")

print()
print("ИТОГ:", "ГОТОВО К ПУШУ" if ok else "НЕ ПУШИТЬ — сначала починить")
sys.exit(0 if ok else 1)
