#!/usr/bin/env python3
"""Считать скобки честно: строки, char-литералы и комментарии не в счёт.

Причина: наивный `src.count('{')` в validate_before_push.py поймал store.rs как
«91 vs 90», но он же считает `{e}` внутри format-строк и скобки в комментариях.
Ложноположительный линтер игнорируют, поэтому считаем только код.
Заодно показывает, где глубина уходит в минус, и какие открытия не закрылись.
"""
import re
import sys

def strip(src: str) -> str:
    """Заменяет строки/char-литералы/комментарии ПРОБЕЛАМИ, сохраняя длину и
    позиции перевода строки.

    ★ Почему именно так: первая версия вставляла один пробел вместо всего
    вырезанного блока, из-за чего индексы в cleaned и src расходились, и
    `line_of(src, i)` выдавал НЕВЕРНЫЕ номера строк (он показал «несбаланс на
    137 и 217» в файле, который был сбалансирован). Позиционно-совпадающая
    замена делает строки честными.
    """
    out = list(src)
    i = 0
    n = len(src)

    def blank(a, b):
        for k in range(a, min(b, n)):
            if src[k] != "\n":
                out[k] = " "

    while i < n:
        c = src[i]
        # raw-строки: r"…", r#"…"#, r##"…"##, а также байтовые br#"…"# / b r#"…"#
        # ★ Терминатор для r#…" — это "…#, а не #"…: первая версия искала
        #   delim[1:], то есть #", и не находила его ⇒ стирала ВСЁ ДО КОНЦА ФАЙЛА,
        #   из-за чего `mod tests` и одна из функций выглядели «незакрытыми»
        #   (ложное «не пушить»). Считаем число решёток и строим терминатор честно.
        m = re.match(r'(?:b)?r(#*)"', src[i:])
        if m:
            hashes = m.group(1)
            term = '"' + hashes
            start = i + len(m.group(0))
            end = src.find(term, start)
            j = n if end < 0 else end + len(term)
            blank(i, j)
            i = j
            continue
        if c == '"' or (c == "b" and i + 1 < n and src[i + 1] == '"'):
            j = i + 1 + (1 if c == "b" else 0)
            j += 1  # открывающая кавычка
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
        if c == "'":
            # char-литерал или lifetime ('_, 'a). Литерал: 'x', '\n', '\u{..}'
            j = i + 1
            if j < n and src[j] == "\\":
                j += 2
                if src[i + 2:i + 3] == "u":
                    k = src.find("}", i)
                    if k > 0:
                        j = k + 1
            else:
                j += 1
            if j < n and src[j] == "'":
                blank(i, j + 1)
                i = j + 1
                continue
            # иначе это lifetime — оставляем как код (скобок там нет)
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
    return "".join(out)


def line_of(src: str, idx: int) -> int:
    return src.count("\n", 0, idx) + 1


def check(path: str) -> int:
    src = open(path, encoding="utf-8").read()
    clean = strip(src)
    depth = 0
    stack = []
    bad = []
    for i, ch in enumerate(clean):
        if ch == "{":
            depth += 1
            stack.append(line_of(src, i))
        elif ch == "}":
            depth -= 1
            if depth < 0:
                bad.append(f"закрывающая }} без открывающей на строке {line_of(src, i)}")
                depth = 0
                stack = []
            elif stack:
                stack.pop()
    print(f"{path}: глубина на конце = {depth}; открытых незакрытых = {len(stack)}")
    if stack:
        print("  строки незакрытых {:", stack[:12])
    for b in bad:
        print("  " + b)
    return 0 if depth == 0 and not bad else 1


if __name__ == "__main__":
    rc = 0
    for p in sys.argv[1:]:
        rc |= check(p)
    sys.exit(rc)
