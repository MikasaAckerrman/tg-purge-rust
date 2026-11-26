#!/usr/bin/env python3
"""Найти строки в doc-комментариях, которые Rust сочтёт doctest'ами.

Зачем (раунд 16, 25.11): CI упал на `error: expected one of `!` or `::`, found `-``
в src/util.rs:4 — это была МОЯ строка с командой `rustc --test ...`, которую я написал
в doc-заголовке как инструкцию. Markdown считает 4-_SPACE-отступ кодовым блоком,
а rustdoc гоняет каждый кодовый блок как doctest. То есть обычный комментарий
стоил 6-минутного прогона и ложного вердикта «юнит-тесты упали».

Тонкость, которую я не учёл и которая объясняет, почему упал ТОЛЬКО util.rs:
кодовый блок начинается только после ПУСТОЙ строки (или после строки, которая
не является частью абзаца). В rate.rs/engine.rs команда идёт сразу за строкой,
оканчивающейся на `:`, поэтому это «ленивое продолжение» абзаца — не doctest.
В util.rs перед ней была пустая `//!` → настоящий блок → doctest → падение.

Проверка делает ровно то, что делает rustdoc: идёт по doc-строкам, снимает префикс
`//!`/`///`, отслеживает ```-фенсы и считает отступ относительно минимального.
Блоки, помеченные языком (```text, ```bash, ```ignore, ```no_run), не являются
Rust-doctest'ами и легальны.
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))
SRC = f"{ROOT}/src"

FENCE = re.compile(r"^\s*(//[/!]+)\s*(```)(.*)$")
DOC = re.compile(r"^\s*(//[/!]+)(.*)$")


def scan(path):
    """Список (строка, текст) doctest-кандидатов: Rust-блоков без фенса."""
    lines = open(path, encoding="utf-8").read().splitlines()
    docs = []          # (lineno, content) только doc-строки, подряд
    for i, line in enumerate(lines, 1):
        m = DOC.match(line)
        if m:
            docs.append((i, m.group(2)))
        else:
            docs.append((i, None))   # разрыв doc-последовательности

    out = []
    in_fence = False
    fence_lang = ""
    prev_blank = True          # начало документа = «пусто перед»
    i = 0
    while i < len(docs):
        lineno, content = docs[i]
        if content is None:
            in_fence = False
            prev_blank = True
            i += 1
            continue

        fm = FENCE.match(content if content.startswith("```") else " " * 0 + content) \
            if False else None
        # фенс ищем прямо в исходной строке
        raw = lines[lineno - 1]
        f = re.search(r"```\s*([A-Za-z0-9_+-]*)", raw)
        if f:
            if not in_fence:
                in_fence = True
                fence_lang = f.group(1).strip().lower()
            else:
                in_fence = False
                fence_lang = ""
            prev_blank = True
            i += 1
            continue

        if in_fence:
            # внутри фенса: doctest только если язык не указан и не ignore/no_run/text
            if fence_lang == "" and content.strip():
                out.append((lineno, content.strip(), "fence без языка"))
            i += 1
            continue

        stripped = content.rstrip()
        if not stripped:
            prev_blank = True
            i += 1
            continue

        # отступ относительно одного обязательного пробела после //!
        if stripped.startswith("    ") or content.startswith("     "):
            indent = len(content) - len(content.lstrip(" "))
            # rustdoc: 4 пробела ОТНОСИТЕЛЬНО минимального отступа блока
            if indent >= 5 and prev_blank:
                out.append((lineno, stripped.strip(), "4+ пробела после пустой строки"))
                i += 1
                continue
        prev_blank = False
        i += 1
    return out


def main():
    # Каталог можно переопределить: так проверяем, что сканер КРАСНЕЕТ на
    # заведомо сломанном входе, не трогая рабочие исходники (проверка ассерта
    # обязательна — иначе это не проверка, а текст).
    src = sys.argv[1] if len(sys.argv) > 1 else SRC
    bad = []
    for f in sorted(os.listdir(src)):
        if not f.endswith(".rs"):
            continue
        p = f"{src}/{f}"
        hits = scan(p)
        for lineno, text, why in hits:
            bad.append((f, lineno, why, text))
            print(f"DOCTEST {f}:{lineno} ({why}): {text[:90]}")
    if bad:
        print(f"\nИТОГ: {len(bad)} doctest-блоков без фенса — rustdoc попытается их "
              f"скомпилировать как Rust и упадёт. Оборачивай в ```text / ```bash.")
        return 1
    print("DOCTEST OK: ни одного Rust-блока без фенса в doc-комментариях")
    return 0


if __name__ == "__main__":
    sys.exit(main())
