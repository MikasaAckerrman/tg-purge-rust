#!/usr/bin/env python3
"""Паритет Rust `shape::no_null` ↔ Python `core/model.py::no_null` на матрице структур.

Зачем: инвариант «в ответах нет null» (M4) уже стоил одного видимого бага на устройстве
(«@null» в APK-поиске). Если две реализации расходятся, UI на Rust начнёт показывать
то, чего не показывал на Python, — и это всплывёт только в бою. Как и с окном лимитов
(2029 сценариев, 0 расхождений), расхождение ловится матрицей, а не глазами.

Матрица: все ключи обеих таблиц (числовые/булевы) + неизвестные ключи + пересечения
форм (null на верхнем уровне, в массиве, в объекте внутри массива, глубина 3) +
нull-массивы и пустые объекты.

Запуск (локально, без CI и без grammers):
    cd tools/localtest && cargo build --release
    python3 tools/parity_json.py
"""
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PY_ENGINE = os.environ.get("TGPURGE_PY", "/var/minis/shared/tg")
DRIVER = f"{ROOT}/tools/localtest/target/release/localtest"

if not os.path.exists(f"{PY_ENGINE}/core/model.py"):
    print(f"НЕТ Python-эталона: {PY_ENGINE}/core/model.py")
    sys.exit(2)
if not os.path.exists(DRIVER):
    print(f"НЕТ Rust-драйвера: {DRIVER}\nСобери: cd tools/localtest && cargo build --release")
    sys.exit(2)

sys.path.insert(0, PY_ENGINE)
from core import model as M  # noqa: E402


def structures():
    """Матрица входов. Ключи берём ИЗ ТАБЛИЦ Rust-файла, чтобы тест не разъезжался
    с ними молча: читаем NUMERIC_KEYS/BOOL_KEYS источника как текст и парсим списки."""
    src = open(f"{ROOT}/src/shape.rs", encoding="utf-8").read()

    def table(name):
        blk = src[src.index(f"pub const {name}"):]
        end = blk.index("];")
        body = blk[:end]
        import re
        return re.findall(r'"([a-z_0-9]+)"', body)

    numeric = table("NUMERIC_KEYS")
    booleans = table("BOOL_KEYS")
    ambiguous = table("AMBIGUOUS_KEYS")
    unknown = ["note", "weird_key", "text", "detail"]

    out = []
    # 0) амбигуозные ключи: обязаны давать "" НА ЛЮБОМ уровне, а не 0/false
    for k in ambiguous:
        out.append({k: None})
        out.append({"watch": {k: None}})
        out.append({"refs": {k: {"armed": None, k: None}}})
        out.append({"a": [{"b": {k: None}}]})
    # 1) плоские объекты: каждый ключ со null
    for k in numeric + booleans + unknown:
        out.append({k: None})
    # 2) те же ключи с реальными значениями (не должны измениться)
    for k in numeric[:6] + booleans[:6] + unknown[:3]:
        out.append({k: 5, "keep": "x"})
    # 3) null внутри массива
    for k in numeric[:8] + booleans[:8] + unknown[:3]:
        out.append({"arr": [{k: None}]})
        out.append({"arr": [None, 1, {k: None}]})
    # 4) вложенность 3 уровня
    for k in numeric[:6] + booleans[:6] + unknown[:2]:
        out.append({"a": {"b": {"c": {k: None}}}})
    # 5) смешанные объекты целиком
    out.append({"username": None, "id": None, "can_delete": None, "msgs": None,
                "members": [{"id": None, "name": None, "is_bot": None}],
                "nested": {"deep": {"x": None, "unread": None}}})
    # 6) крайности
    out.append({})
    out.append([])
    out.append({"a": []})
    out.append({"a": {}})
    out.append({"a": None, "b": [None], "c": {"d": None}})
    out.append({"list": [None, None, None]})
    out.append({"matrix": [[{"id": None}], [{"note": None}]]})
    # 7) один и тот же ключ на разных уровнях (контекст не должен теряться)
    out.append({"id": None, "inner": {"id": None, "inner2": {"id": None}}})
    out.append({"note": None, "inner": {"note": None}})
    # 8) ключи, которых нет в таблицах, но похожи на числовые
    for k in ("count_x", "idx", "duplicated", "enabled_extra", "can_delete_all"):
        out.append({k: None})
    return out


def main():
    cases = structures()
    payload = json.dumps(cases, ensure_ascii=False)
    p = subprocess.run([DRIVER], input=payload, capture_output=True, text=True, timeout=120)
    if p.returncode != 0:
        print("RUST-ДРАЙВЕР УПАЛ:", (p.stderr or p.stdout)[:1200])
        return 1
    got = [x for x in p.stdout.splitlines() if x.strip()]
    if len(got) != len(cases):
        print(f"РАСХОЖДЕНИЕ ФОРМАТА: кейсов {len(cases)}, строк Rust {len(got)}")
        return 1
    bad = []
    for case, g in zip(cases, got):
        py = M.no_null(json.loads(json.dumps(case)))
        py_s = json.dumps(py, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
        ru_s = json.dumps(json.loads(g), ensure_ascii=False, sort_keys=True,
                          separators=(",", ":"))
        if py_s != ru_s:
            bad.append((case, py_s, ru_s))
    print(f"кейсов: {len(cases)} | несовпадений: {len(bad)}")
    for c, py_s, ru_s in bad[:15]:
        print(f"  вход: {json.dumps(c, ensure_ascii=False)[:110]}")
        print(f"    Python: {py_s[:150]}")
        print(f"    Rust:   {ru_s[:150]}")
    if bad:
        print("\nВЫВОД: формы РАЗЛИЧАЮТСЯ — Rust-движок покажет то, чего не показывал Python.")
        return 1
    print("ВЫВОД: Rust и Python дают идентичный no_null на всей матрице.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
