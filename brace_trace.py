#!/usr/bin/env python3
"""Найти, где именно в Rust-файле теряется баланс фигурных скобок.

Почему это нужно: мой `braces.py` показывал «глубина 1» в src/auth.rs, но
печатал неверные номера строк (он же утверждал len(clean)==len(src)). Чтобы
не гадать, здесь отдельная, максимально простая реализация: посимвольный
токенайзер, который вырезает строки/char/комментарии, и построчный отчёт
по глубине. Плюс проверка инварианта длины.

Использование: python3 brace_trace.py <файл.rs>
"""
import sys


def scan(src: str):
    """Возвращает (clean, events): clean — код без строк/комментариев той же длины,
    events — список (строка, символ, глубина_до, глубина_после, текст_строки)."""
    out = list(src)
    n = len(src)
    i = 0

    def blank(a, b):
        for k in range(a, min(b, n)):
            if src[k] != "\n":
                out[k] = " "

    while i < n:
        ch = src[i]
        # raw-строки: r"…" / r#"…"# / br#"…"# / rb#"…"#
        m = len(ch)
        if src.startswith(("r\"", "r#"), i) or src.startswith(("br\"", "br#", "rb\"", "rb#"), i):
            j = i
            while j < n and src[j] not in "\"":
                j += 1
            hashes = src[i:j].count("#") if "#" in src[i:j] else 0
            # находим открывающую кавычку после решёток
            q = src.find('"', i)
            if q < 0:
                blank(i, n)
                i = n
                continue
            term = '"' + "#" * hashes
            end = src.find(term, q + 1)
            k = n if end < 0 else end + len(term)
            blank(i, k)
            i = k
            continue
        if ch == '"' or (ch == "b" and i + 1 < n and src[i + 1] == '"'):
            j = i + 1 + (1 if ch == "b" else 0) + 1
            while j < n:
                if src[j] == "\\":
                    j += 2
                    continue
                if src[j] == '"':
                    j += 1
                    break
                j += 1
            blank(i, j)
            i = j
            continue
        if ch == "'":
            j = i + 1
            if j < n and src[j] == "\\":
                j += 2
                if j < n and src[j] == "{":
                    k = src.find("}", i)
                    if k > 0:
                        j = k + 1
            elif j < n:
                j += 1
            if j < n and src[j] == "'":
                blank(i, j + 1)
                i = j + 1
                continue
            i += 1
            continue
        if src.startswith("/*", i):
            depth = 1
            j = i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth += 1
                    j += 2
                elif src.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            blank(i, j)
            i = j
            continue
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
            continue
        i += 1
    clean = "".join(out)
    assert len(clean) == n, f"инвариант длины нарушен: {len(clean)} != {n}"
    return clean


def main(path: str) -> int:
    src = open(path, encoding="utf-8").read()
    clean = scan(src)
    lines = src.splitlines()
    depth = 0
    events = []
    for i, ch in enumerate(clean):
        if ch not in "{}":
            continue
        ln = src.count("\n", 0, i) + 1
        d0 = depth
        depth += 1 if ch == "{" else -1
        events.append((ln, ch, d0, depth, lines[ln - 1].strip()[:100]))
    print(f"{path}: длина {len(src)}, конечная глубина {depth}")
    print("последние 20 событий:")
    for e in events[-20:]:
        print(f"  L{e[0]:>4} {e[1]} {e[2]}->{e[3]}  {e[4]}")
    # где глубина впервые перестала возвращаться к 0
    tops = [e for e in events if e[3] == 0]
    if tops:
        last_top = tops[-1][0]
        print(f"последнее закрытие верхнего уровня: строка {last_top}")
        print("всё после него висит внутри неоткрытой области:")
        for e in events:
            if e[0] > last_top:
                print(f"  L{e[0]:>4} {e[1]} {e[2]}->{e[3]}  {e[4]}")
    return 0 if depth == 0 else 1


if __name__ == "__main__":
    rc = 0
    for p in sys.argv[1:]:
        rc |= main(p)
    sys.exit(rc)
