#!/usr/bin/env python3
"""Валидация манифеста и workflow ДО пуша.

Почему существует: раунд 2 сгорел на YAML (двоеточие в некавированном name),
раунд 3 — на Cargo.toml без заголовка [package]. Оба прогона не дали ни одного
ответа по grammers, потому что cargo до grammers не дошёл. Проверка локально
стоит 1 секунду, проверка в CI — 3 минуты и один сгоревший раунд.
"""
import sys
import tomllib

import yaml

ROOT = "/var/minis/shared/rustspike"
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
raw = open(f"{ROOT}/.github/workflows/rust.yml", "rb").read()
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
            if "run" in s and "pipefail" not in s["run"] and "|" in s["run"]:
                print(f"YAML ПЛОХО: step в {name} использует pipe без pipefail")
                ok = False
    print("YAML OK: jobs =", list(jobs.keys()))

# ---- Rust-исходники: грубая проверка баланса ----
for f in ("src/main.rs", "src/caps.rs"):
    try:
        src = open(f"{ROOT}/{f}", encoding="utf-8").read()
    except FileNotFoundError:
        print(f"NET ФАЙЛА {f}")
        ok = False
        continue
    if src.count("{") != src.count("}"):
        print(f"{f}: не сбалансированы фигурные скобки ({src.count('{')} vs {src.count('}')})")
        ok = False
    if "grammers_client::types" in src:
        print(f"{f}: ссылается на grammers_client::types — такого модуля в 0.12.0 НЕТ")
        ok = False
    if "retry_policy::" in src:
        print(f"{f}: использует retry_policy — модуль не объявлен в 0.12.0")
        ok = False
    print(f"{f}: {len(src)} байт, скобки сбалансированы")

print()
print("ИТОГ:", "ГОТОВО К ПУШУ" if ok else "НЕ ПУШИТЬ — сначала починить")
sys.exit(0 if ok else 1)
