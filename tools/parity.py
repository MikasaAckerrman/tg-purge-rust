#!/usr/bin/env python3
"""Паритет Rust ↔ Python по скользящему окну лимитов (гейт R3/R4, PLAN_RUST §3).

Зачем: Python-движок (`core/model.py::next_allowed`) уже в бою, Rust-порт написан
с нуля. «Тесты зелёные» НЕ доказывают, что две реализации считают одинаково — ровно
так я поймал баг индекса в Rust `Window::wait` (раунд 12). Здесь матрица сценариев
прогоняется с обеих сторон и сравниваются числа в миллисекундах.

Запуск (локально, без CI и без grammers):
    rustc --crate-type=lib --edition 2021 src/rate.rs -o /tmp/librate.rlib
    rustc --edition 2021 tools/parity.rs --extern rate=/tmp/librate.rlib -o /tmp/parity
    python3 tools/parity.py
"""
import itertools
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# Python-эталон живёт в рабочем проекте движка (не в rust-директории):
# /var/minis/shared/tg/core/model.py — тот самый код, который уже сутки в бою.
PY_ENGINE = os.environ.get("TGPURGE_PY", "/var/minis/shared/tg")
if not os.path.exists(f"{PY_ENGINE}/core/model.py"):
    print(f"НЕТ Python-эталона: {PY_ENGINE}/core/model.py")
    sys.exit(2)
sys.path.insert(0, PY_ENGINE)

from core import model as M  # noqa: E402

RLIB = "/tmp/librate.rlib"
DRIVER = "/tmp/parity_driver"


def build():
    """Собираем rate.rs как lib и драйвер паритета. Ошибка сборки = провал проверки."""
    for cmd in (
        ["rustc", "--crate-type=lib", "--edition", "2021", f"{ROOT}/src/rate.rs", "-o", RLIB],
        ["rustc", "--edition", "2021", f"{ROOT}/tools/parity.rs",
         "--extern", f"rate={RLIB}", "-o", DRIVER],
    ):
        p = subprocess.run(cmd, capture_output=True, text=True)
        if p.returncode != 0:
            print("СБОРКА ПАРИТЕТА УПАЛА:", " ".join(cmd))
            print((p.stderr or p.stdout)[:2500])
            return False
    return True


def scenarios():
    """Матрица: limit 0..4, span 10/60/3600, истории разной формы, now в разных точках.

    limit=0 обязана означать «без потолка» (U-D20: старое `or DEFAULT` превращало 0 в 12).
    Отрицательные смещения проверяют окно до «сейчас» (история старше now).
    """
    out = []
    for limit in (0, 1, 2, 3, 4):
        for span in (10, 60, 3600):
            for hits in (
                (),
                (0,),
                (0, 10),
                (0, 10, 20),
                (0, 1, 2, 3, 4),
                (-30, 0),
                (0, 30, 31),
                (5, 5),
                (0, 59, 60, 61),
            ):
                for now in (-1, 0, 1, 5, 9, 10, 11, 20, 21, 30, 59, 60, 61, 120, 3600):
                    out.append((limit, span, hits, now))
    # редкие, но важные формы: очень длинные истории и huge span (подлёт Instant)
    for limit in (1, 2):
        out.append((limit, 60 * 60 * 24 * 3600, (0, 1, 2), 3))
        out.append((limit, 60, tuple(range(0, 40, 3)), 41))
    return out


def py_wait(limit, span, hits, now):
    """Python-эталон: та же логика, что в бою (core/model.py::next_allowed).

    Эталон оперирует абстрактными «временами» (числа), поэтому base = 0 и смения
    подаются как есть. next_allowed(history, now, limit, span) -> секунд ожидания.
    """
    return M.next_allowed(list(hits), float(now), limit, float(span))


def main():
    if not build():
        return 1
    scen = scenarios()
    lines = []
    for limit, span, hits, now in scen:
        toks = [str(limit), str(span), str(len(hits)), *[str(h) for h in hits], str(now)]
        lines.append(" ".join(toks))
    p = subprocess.run([DRIVER], input="\n".join(lines) + "\n",
                       capture_output=True, text=True, timeout=120)
    if p.returncode != 0:
        print("ДРАЙВЕР УПАЛ:", (p.stderr or p.stdout)[:1500])
        return 1
    got = [x.strip() for x in p.stdout.splitlines() if x.strip()]
    if len(got) != len(scen):
        print(f"РАСХОЖДЕНИЕ ФОРМАТА: сценариев {len(scen)}, ответов {len(got)}")
        return 1
    bad = []
    for (limit, span, hits, now), g in zip(scen, got):
        rust_ms = int(g)
        py_ms = int(round(py_wait(limit, span, hits, now) * 1000))
        if rust_ms != py_ms:
            bad.append((limit, span, hits, now, py_ms, rust_ms))
    print(f"сценариев: {len(scen)} | несовпадений: {len(bad)}")
    for b in bad[:25]:
        limit, span, hits, now, py_ms, rust_ms = b
        print(f"  limit={limit} span={span} hits={hits} now={now} "
              f"→ Python {py_ms} мс, Rust {rust_ms} мс")
    if bad:
        print("\nВЫВОД: реализации считают РАЗНО — Rust-порт не готов к гейту R3.")
        return 1
    print("ВЫВОД: Rust и Python совпадают по всем сценариям матрицы.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
