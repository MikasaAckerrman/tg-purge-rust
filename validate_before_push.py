#!/usr/bin/env python3
"""Валидация манифеста и workflow ДО пуша.

Почему существует: раунд 2 сгорел на YAML (двоеточие в некавированном name),
раунд 3 — на Cargo.toml без заголовка [package]. Оба прогона не дали ни одного
ответа по grammers, потому что cargo до grammers не дошёл. Проверка локально
стоит 1 секунду, проверка в CI — 3 минуты и один сгоревший раунд.
"""
import os
import re
import subprocess
import sys
import tomllib

import yaml

import braces2

ROOT = os.environ.get("TGPURGE_ROOT") or os.path.dirname(os.path.abspath(__file__))
# ★ Раунд 22: раньше здесь был хардкод "/var/minis/shared/rustspike", и красный вход
#   на КОПИИ дерева (восстановленные ошибки CI-шагов) НЕ КРАСНЕЛ: валидатор читал
#   боевой workflow и докладывал «ГОТОВО К ПУШУ» о файле, которого он не проверял.
#   Тот же класс, что BA-13 в тестах (test_dispatch читал engine.py по абсолютному
#   пути → 0 падений при применённом откате): неатрибуируемая проверка = декорация.
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
# ★ Список зависимостей НЕ хардкодим (жёсткий список устаревает и сам вводит в заблуждение:
#   он уже однажды требовал tiny_http после того, как я её вынес). Проверяем СИММЕТРИЮ:
#   объявленное — используется, используемое — объявлено. Это ловит оба класса ошибок:
#   «unused dependency» (раунд 11: cargo warn'инг на tiny_http) и «не объявлен» (E0432).
deps_us = {d.replace("-", "_") for d in deps}
all_code = ""
for f in sorted(os.listdir(f"{ROOT}/src")):
    if f.endswith(".rs"):
        with open(f"{ROOT}/src/{f}", encoding="utf-8") as fh:
            all_code += braces2.blanks(fh.read()) + "\n"

RESERVED = {"std", "core", "crate", "self", "super"}
# `use X as Y;` — X используется через алиас Y, поэтому ищем и алиасы.
# (grammers_tl_types в caps.rs объявлен именно как `use grammers_tl_types as tl;` —
#  без этого проверка давала ЛОЖНОЕ «не используется».)
alias_pairs = re.findall(r"^\s*use\s+([a-z_][a-z0-9_]*)\s+as\s+([a-z_][a-z0-9_]*)\s*;",
                         all_code, re.M)
aliases = {a: c for c, a in alias_pairs}
aliased_crates = {c for c, _ in alias_pairs}

# имя собственного lib-крета — не зависимость (main.rs делает `use purge::caps;`)
own_names = {man["package"]["name"].replace("-", "_")}
lib_name = (man.get("lib") or {}).get("name")
if lib_name:
    own_names.add(lib_name.replace("-", "_"))

for d in sorted(deps_us):
    if re.search(rf"\b{re.escape(d)}::", all_code) or d in aliased_crates:
        continue
    print(f"CARGO ПЛОХО: зависимость {d} объявлена, но в коде не используется "
          f"(cargo даёт unused-dependency; лишний вес в .so)")
    ok = False

used_first = set(re.findall(r"^\s*use\s+([a-z_][a-z0-9_]*)::", all_code, re.M))
for u in sorted(used_first - RESERVED - deps_us - own_names - set(aliases)):
    print(f"CARGO ПЛОХО: код использует {u}::, но такой зависимости нет в Cargo.toml")
    ok = False
print(f"CARGO OK: симметрия ({len(deps_us)} объявлено, алиасов: {len(aliases)}, "
      f"собственные имена: {sorted(own_names)})")
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
# ★ 26.11: проверка переведена с cdylib на rlib. Причина — замер раунда 18, а не вкус:
#   `nm -D --defined-only libpurge.so` дал 0 символов T (strip=true + ни одного
#   `#[no_mangle] extern "C"`), значит JNI-мост из Kotlin звать нечем, а его починка
#   требует unsafe (ломает `#![deny(unsafe_code)]`). APK запускает бинарь `purge serve`
#   как ДОЧЕРНИЙ ПРОЦЕСС и говорит с ним по тому же loopback HTTP (PLAN_RUST §11),
#   поэтому обязательное требование теперь — наличие [[bin]], а не cdylib.
if "rlib" not in ct:
    print(f"CARGO ПЛОХО: lib.crate-type не содержит rlib — движок не сможет использовать слои")
    ok = False
if not man.get("bin"):
    print("CARGO ПЛОХО: нет [[bin]] — APK нечего запускать как процесс (путь 26.11)")
    ok = False
else:
    print(f"CARGO OK: crate-type={ct}, bins={[b.get('name') for b in man['bin']]}")

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
        # раунд 14: шаг сбора диагностики БЕЗ `if: always()` пропускается после
        # падения предыдущего ⇒ test.log не сохранился и раунд 15 чинился бы вслепую.
        # ★ Проверка обязана быть на УРОВНЕ ШАГА: первая версия легла внутрь
        #   `if run and "pipefail" in run`, и у шагов без `run:` (upload-artifact)
        #   она не выполнялась вовсе — красный тест проходил молча.
        name = s.get("name") or ""
        if "сохранить отчёт" in name or "логи" in name:
            if s.get("if") != "always()":
                print(f"YAML ПЛОХО: шаг {name!r} собирает диагностику без if: always() "
                      f"— при провале он пропустится")
                ok = False
    print("YAML OK: jobs =", list(jobs.keys()))

# ---- чистая логика: локальный прогон тестов БЕЗ CI (главная экономия раунда 13) ----
# Модули, зависящие только от std ⇒ rustc 1.83 компилирует и тестирует их за доли
# секунды, тогда как CI-прогон стоит 6 минут. Каждый такой модуль обязан быть здесь,
# иначе его логика проверяется «на глаз» в логах упавшего раннера.
# Модули, тестируемые ЛОКАЛЬНО без CI. Делятся на два класса, и путать их нельзя:
#   std-only  → гоняются голым `rustc --test` (зависимостей нет вообще)
#   dep-based → гоняются только через cargo (нужны serde_json / tiny_http),
#               поэтому их покрает tools/localtest; голый rustc на них ПАДАЕТ
#               (проверено 26.11: PURE ПЛОХО на shape.rs/api.rs был багом валидатора,
#               а не боевого кода — локальный rustc не знает про crates.io).
STD_ONLY = ("rate.rs", "util.rs")
DEP_BASED = ("shape.rs", "api.rs")
PURE_MODULES = STD_ONLY + DEP_BASED

for mod in PURE_MODULES:
    path = f"{ROOT}/src/{mod}"
    if not os.path.exists(path):
        print(f"PURE ПЛОХО: нет src/{mod} (чистая логика обязана быть локально тестируемой)")
        ok = False
        continue
    with open(path, encoding="utf-8") as fh:
        raw = fh.read()
    # ★ Проверяем по КОДУ, а не по файлу: в doc-заголовке rate.rs намеренно упомянут
    #   grammers-tl-types (объяснение, зачем модуль вынесен), и наивная проверка
    #   `"grammers" in raw` ловила бы собственный комментарий.
    #   api.rs тоже обязан быть без grammers — в этом весь смысл локальных тестов.
    code = braces2.blanks(raw)
    if re.search(r"\buse\s+grammers", code) or re.search(r"\bgrammers_[a-z_]+::", code):
        print(f"PURE ПЛОХО: src/{mod} обязан быть без зависимостей от grammers — иначе теряет смысл локальный прогон")
        ok = False
    if "#[cfg(test)]" not in braces2.blanks(raw):
        print(f"PURE ПЛОХО: src/{mod} не содержит #[cfg(test)] — локально тестировать нечего")
        ok = False
    if mod not in STD_ONLY:
        # dep-based (serde_json/tiny_http) голым rustc НЕ собрать — он не знает про
        # crates.io. Их покрает tools/localtest; здесь только сверим, что он их берёт.
        hpath = f"{ROOT}/tools/localtest/main.rs"
        harness = open(hpath, encoding="utf-8").read() if os.path.exists(hpath) else ""
        if f"../../src/{mod}" not in harness:
            print(f"PURE ПЛОХО: src/{mod} не подключён в tools/localtest/main.rs — он не тестируется")
            ok = False
        else:
            print(f"PURE OK: src/{mod} покрыт localtest (cargo, а не голый rustc)")
        continue
    exe = f"/tmp/pure_test_{mod.replace('.', '_')}"
    p = subprocess.run(["rustc", "--test", "--edition", "2021", path, "-o", exe],
                       capture_output=True, text=True)
    if p.returncode != 0:
        print(f"PURE ПЛОХО: rustc не собрал тесты src/{mod}")
        print((p.stderr or p.stdout)[:1500])
        ok = False
        continue
    r = subprocess.run([exe], capture_output=True, text=True)
    tail = (r.stdout or "").strip().splitlines()[-1:] or ["(нет вывода)"]
    if r.returncode != 0:
        print(f"PURE ПЛОХО: юнит-тесты src/{mod} упали")
        print("\n".join((r.stdout or "").splitlines()[-40:])[:2000])
        ok = False
    else:
        print(f"PURE OK: src/{mod} — {tail[0]}")

# ---- полнота match по своим enum-ам (раунд 15: E0004 на CodeError::NotE164) ----
# Rust ловит это только при сборке, а сборка grammers-кода = 6 минут CI.
# Проверяем узкий, но реальный класс: `impl From<Enum> for ДругойEnum` обязан
# покрывать ВСЕ варианты либо иметь `_ =>`.
ENUM_RE = re.compile(r"^(?:pub )?enum\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{(.*?)\n\}", re.M | re.S)
VAR_RE = re.compile(r"^\s*(?:///[^\n]*\n\s*)?([A-Z][A-Za-z0-9_]*)\s*(?:\(|,)")


def enum_variants(body: str):
    out = []
    for line in body.splitlines():
        m = VAR_RE.match(line)
        if m:
            out.append(m.group(1))
    return out


enums = {}
for f in sorted(os.listdir(f"{ROOT}/src")):
    if not f.endswith(".rs"):
        continue
    code = braces2.blanks(open(f"{ROOT}/src/{f}", encoding="utf-8").read())
    for m in ENUM_RE.finditer(code):
        enums[m.group(1)] = enum_variants(m.group(2))

for f in sorted(os.listdir(f"{ROOT}/src")):
    if not f.endswith(".rs"):
        continue
    code = braces2.blanks(open(f"{ROOT}/src/{f}", encoding="utf-8").read())
    for m in re.finditer(r"impl From<([A-Za-z0-9_]+)> for ([A-Za-z0-9_]+)", code):
        src_enum = m.group(1)
        if src_enum not in enums:
            continue
        start = m.start()
        block = code[start:start + 2500]
        # тело impl: до следующего `impl ` или `#[cfg(test)]` на том же уровне
        nxt = re.search(r"\n(?:impl |#\[cfg\(test\)\]|pub fn |pub async fn )", block[len(m.group(0)):])
        if nxt:
            block = block[:len(m.group(0)) + nxt.start()]
        missing = [v for v in enums[src_enum] if f"{src_enum}::{v}" not in block]
        if missing and "_ =>" not in block and "_, " not in block:
            print(f"SRC ПЛОХО: src/{f}: impl From<{src_enum}> не покрывает варианты {missing} "
                  f"и нет `_ =>` → E0004 на сборке")
            ok = False

# ---- локальный cargo-тест боевого JSON-слоя (без CI и без grammers) ----
# serde_json собирается на cargo 1.83 (замерено 26.11), grammers — нет. Значит
# форма JSON-ответов обязана проверяться здесь за ~25 с, а не шестью минутами CI.
LT = f"{ROOT}/tools/localtest"
if os.path.exists(f"{LT}/Cargo.toml"):
    p = subprocess.run(["cargo", "test", "-q"], cwd=LT, capture_output=True, text=True)
    if p.returncode != 0:
        print("LOCALTEST ПЛОХО: cargo test в tools/localtest упал")
        print((p.stdout or "")[-2500:], (p.stderr or "")[:800])
        ok = False
    else:
        line = [l for l in (p.stdout or "").splitlines() if "test result" in l]
        print(f"LOCALTEST OK: {line[-1].strip() if line else 'боевой src/shape.rs зелёный'}")
    w = subprocess.run(["cargo", "build", "--release", "-q"], cwd=LT,
                       capture_output=True, text=True)
    nwarn = (w.stdout or "").count("warning:") + (w.stderr or "").count("warning:")
    if nwarn:
        print(f"LOCALTEST ПЛОХО: {nwarn} warning'ов в боевом src/shape.rs")
        for l in ((w.stdout or "") + (w.stderr or "")).splitlines():
            if l.startswith("warning"):
                print("   ", l[:150])
        ok = False
    else:
        print("LOCALTEST OK: release-сборка харнеса без warning'ов")
    # Харнес обязан вызывать публичные формы shape.rs, иначе «never used» маскирует
    # реальную мёртвую логику: allow(dead_code) на модуле оправдан ТОЛЬКО тем, что
    # в lib.rs они pub-API. Проверим, что хотя бы часть действительно используется.
    harness = open(f"{LT}/main.rs", encoding="utf-8").read()
    for sym in ("no_null", "count_ru"):
        if sym not in harness:
            print(f"LOCALTEST ПЛОХО: харнес не вызывает {sym} — тесты не покрывают форму")
            ok = False
    if "#[allow(dead_code)]" not in harness:
        print("LOCALTEST ПЛОХО: харнес без обоснованного allow(dead_code) — "
              "warning'ы боевого файла вернутся")
        ok = False
else:
    print("LOCALTEST ПЛОХО: нет tools/localtest/Cargo.toml — JSON-слой не тестируется локально")
    ok = False

# ---- UI-ассеты: копия в Rust-репо обязана быть побайтово идентична источнику ----
# Две копии одного UI = рассинхрон, и это ровно тот класс «нажал — не изменилось».
# CI не видит песочницу, поэтому проверка здесь, локально (26.11).
PY_UI = "/var/minis/shared/tg/ui"
RS_UI = f"{ROOT}/assets/ui"
if os.path.isdir(PY_UI) and os.path.isdir(RS_UI):
    for f in ("index.html", "app.css", "app.js", "strings.js"):
        a, b = f"{PY_UI}/{f}", f"{RS_UI}/{f}"
        if not os.path.exists(b):
            print(f"UI ПЛОХО: в Rust-репо нет assets/ui/{f} — APK получит белый экран")
            ok = False
            continue
        da, db = open(a, "rb").read(), open(b, "rb").read()
        if da != db:
            print(f"UI ПЛОХО: assets/ui/{f} ДРЕЙФИТ от tg/ui/{f} ({len(da)} vs {len(db)} байт)")
            ok = False
        else:
            print(f"UI OK: {f} побайтово идентичен ({len(da)} байт)")
elif not os.path.isdir(RS_UI):
    print("UI ПЛОХО: нет assets/ui — путь «APK запускает бинарь» некомплектен")
    ok = False

# ---- артефакты CI против Cargo.toml (раунд 22) ----
# Поймано провалом CI раунда 21: я сменил crate-type cdylib→rlib, поправил только
# один шаг и оставил `ls target/release/libpurge.so` в шаге «размер артефактов»
# (host) → `ls: cannot access ... ` → exit 2. Правило: меняя ФОРМУ артефакта,
# обязан пройти ВСЕ упоминания имени в workflow, а не только редактируемый шаг.
wf = open(f"{ROOT}/.github/workflows/rust.yml", encoding="utf-8").read()


def yaml_code(src):
    """Workflow БЕЗ YAML-комментариев.

    Нужно, потому что собственные комментарии раунда 22 («…pie executable…», «…ls
    не находит libpurge.so…») удовлетворяли предикатам лучше, чем код: валидатор
    докладывал «CI OK» о файле, где ошибка стояла в команде, просто потому что то же
    слово было в пояснении. Тот же класс, что `strip_js_comments` в UI-тестах
    (26.11: ассерт краснел на прозе, а не на коде).
    """
    out = []
    for line in src.splitlines():
        s = line.lstrip()
        out.append("" if s.startswith("#") else line)
    return "\n".join(out)


# ★ braces2.blanks здесь НЕ применяется: он затирает содержимое кавычек, а нужные
#   нам предикаты живут внутри кавычек (`grep -qE "Type:.*(EXEC|DYN)"`). Комментарии
#   YAML уже убраны yaml_code — именно они были причиной ложного «CI OK».
wf_code = yaml_code(wf)
lib_names = []
if "cdylib" not in str(man.get("lib", {}).get("crate-type", [])):
    lib_names.append(f"lib{man.get('lib', {}).get('name', man['package']['name'])}.so")
for bad in lib_names:
    # ищем только в командах run (комментарии YAML имеют право упоминать имя)
    hits = [l.strip() for l in wf_code.splitlines()
            if bad in l and not l.strip().startswith("#") and "echo" not in l
            and "if [" not in l and "::error" not in l]
    if hits:
        print(f"CI ПЛОХО: workflow ссылается на {bad}, которого не собирается (crate-type без cdylib):")
        for h in hits[:4]:
            print("   ", h[:110])
        ok = False
    else:
        print(f"CI OK: {bad} не требуется ни в одной команде (cdylib убран осознанно)")

# ---- гейт исполнимости обязан принимать PIE (раунд 22) ----
# Раунд 21 требовал `Type:.*EXEC`, а aarch64-release — PIE («DYN (Position-Independent
# Executable file)»): гейт отклонял РАБОЧИЙ артефакт. Гейт, ломающий охраняемое,
# хуже отсутствия гейта — он приучает игнорировать красные флаги.
# ★ Предикат смотрит СТРОКУ ПРОВЕРКИ, а не весь файл: первая версия искала
#   «pie executable» по всему workflow и оставалась зелёной на копии с возвращённым
#   `grep -q "Type:.*EXEC"`, потому что это слово жило в соседней (легальной)
#   проверке `file`. Вторая версия ловила любую строку со словом Type и краснела на
#   БОЕВОМ файле: `grep -E "Class:|Machine:|Type:" hdr.txt | tee -a …` — это печать
#   readelf, а не условие. Фильтр: только `if … grep … hdr.txt`.
type_lines = [l.strip() for l in wf_code.splitlines()
              if "Type:" in l and "grep" in l and "hdr.txt" in l
              and l.strip().startswith("if")]
if not type_lines:
    print("CI ПЛОХО: в workflow нет проверки Type у readelf — артефакт мог перестать быть исполняемым незамеченным")
    ok = False
else:
    bad = [l for l in type_lines if "DYN" not in l]
    for l in type_lines:
        print(f"  проверка Type: {l[:100]}")
    if bad:
        print(f"CI ПЛОХО: {len(bad)} из {len(type_lines)} проверок Type требуют только EXEC — aarch64-бинарь PIE и провалится:")
        for l in bad:
            print("   ", l[:110])
        ok = False
    else:
        print(f"CI OK: все {len(type_lines)} проверок Type принимают PIE (EXEC|DYN)")

# ★ Раунд 23: интерпретатор обязан искаться НЕ в readelf -d. Раунд 22 писал
#   `grep -q "ld-android" dyn.txt`, а PT_INTERP в динамической секции не живёт
#   (замер по артефакту: dyn.txt — 23 записи, ни одной про interp; интерпретатор
#   виден в `readelf -l` и в выводе `file`). Предикат не мог пройти никогда.
bad_interp = [l.strip() for l in wf_code.splitlines()
              if "ld-android" in l and "dyn.txt" in l]
if bad_interp:
    print("CI ПЛОХО: интерпретатор ищется в readelf -d (dyn.txt), где PT_INTERP не живёт:")
    for l in bad_interp[:3]:
        print("   ", l[:110])
    ok = False
elif "system/bin/linker64" not in wf_code:
    print("CI ПЛОХО: нет проверки Android-интерпретатора по file/readelf -l")
    ok = False
else:
    print("CI OK: интерпретатор проверяется по system/bin/linker64 (file / readelf -l)")

# ★ Закон общего вида: у гейта артефакта обязан быть локальный прогон против
#   РЕАЛЬНОГО снапшота, иначе он написан на догадке о формате. Два красных раунда
#   подряд (21, 22) — обе причины ловятся одной секундой локального прогона.
selftest = f"{ROOT}/tools/ci_gate_selftest.sh"
if not os.path.exists(selftest):
    print("CI ПЛОХО: нет tools/ci_gate_selftest.sh — предикаты артефакта не проверялись против снапшота")
    ok = False
else:
    st = open(selftest, encoding="utf-8").read()
    need = ["system/bin/linker64", "Type:.*(EXEC|DYN)", "NEEDED"]
    miss = [x for x in need if x not in st]
    if miss:
        print(f"CI ПЛОХО: ci_gate_selftest.sh не покрывает {miss} — часть ловушек раундов 21-22 снова открыта")
        ok = False
    else:
        print("CI OK: ci_gate_selftest.sh покрывает Type(PIE), интерпретатор и NEEDED")

# ---- doctest-ловушки в doc-комментариях (раунд 16: CI упал на моей же команде) ----
# rustdoc гоняет КАЖДЫЙ кодовый блок в //! как doctest. Команда `rustc --test ...`,
# набранная отступом в 4 пробела после пустой строки, ломает сборку тестов с
# «expected one of `!` or `::`, found -» — и выглядит как «упали юнит-тесты».
if os.path.exists(f"{ROOT}/doctest_scan.py"):
    d = subprocess.run([sys.executable, "-u", f"{ROOT}/doctest_scan.py"],
                       capture_output=True, text=True)
    if d.returncode != 0:
        print("DOCTEST ПЛОХО: в doc-комментариях есть Rust-блоки без фенса:")
        print((d.stdout or "")[:1500])
        ok = False
    else:
        print((d.stdout or "").strip().splitlines()[-1:][0] if d.stdout else "DOCTEST OK")
else:
    print("DOCTEST ПЛОХО: нет doctest_scan.py — ловушка раунда 16 не проверяется")
    ok = False

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
    # ВАЖНО: все проверки ниже идут по коду БЕЗ строк и комментариев — иначе
    # собственная doc-строка детектируется как реальный баг (ложное срабатывание,
    # из-за которого валидатору перестают верить).
    code = braces2.blanks(src)
    if "grammers_client::types" in src:
        print(f"src/{f}: ссылается на grammers_client::types — такого модуля в 0.12.0 НЕТ")
        ok = False
    # раунд 10: retry-политики РЕАЛЬНО есть в grammers-mtsender (configuration.rs:123,198)
    # и подключены через ConnectionParams::default(); мёртв только файл
    # grammers-client/src/client/retry_policy.rs. Утверждать обратное запрещает тест.
    if "retry_policy.rs" in code and "AutoSleep" not in code:
        print(f"src/{f}: утверждение про retry_policy без AutoSleep — это моя прошлая ошибка")
        ok = False
    # ловим конкретные сигнатурные ошибки, на которых я уже падал (раунды 5–6).
    if "get_me()" in code and ".await" not in code.split("get_me()")[1][:40]:
        print(f"src/{f}: get_me() без .await — это async (chats.rs:414)")
        ok = False
    if re.search(r"user\.id\(\)", code) and "bare_id_unchecked" not in code:
        print(f"src/{f}: User::id() -> PeerId, а не i64 — нужен bare_id_unchecked()")
        ok = False
    # раунд 14: User::first_name()/last_name()/username() -> Option<&str> (peer/user.rs:128,133,161).
    # В печати у Option НЕТ Display ⇒ не компилируется. Проверка обязана ловить и
    # многострочные println!: в раунде 14 наивная «println! в той же строке» НЕ поймала
    # мой собственный баг, потому что вызов был на отдельной строке внутри println!.
    for idx, line in braces2.fmt_lines(code):
        for m in ("first_name()", "last_name()", "username()"):
            if m in line and "unwrap" not in line and ":?" not in line and "match" not in line:
                print(f"src/{f}:{idx}: {m} -> Option<&str>, в печати нужен unwrap_or (или {{:?}}): {line.strip()[:90]}")
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
