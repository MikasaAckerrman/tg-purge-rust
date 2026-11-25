#!/usr/bin/env python3
"""Минимальный токенайзер Rust-кода: вырезает строки/char/комментарии, считает скобки.

Замена braces.py и brace_trace.py: обе они молчали о баге, который я сам же и
написал — из-за неверного порядка ветвей детектор строк не срабатывал, и
`format!("{e}")` считался как код. Ложный линтер хуже отсутствия: он сказал
«НЕ ПУШИТЬ» на правильном файле и «всё ок» на неправильном.

Инвариант: clean той же длины, что src (позиции строк совпадают).
Проверка самоделки обязательна: `--selftest` гоняет набор кейсов, где ответ
известен заранее, и падает, если хоть один не совпал.
"""
import sys

CODE = "code"
SQ = "sq"          # '...'-литерал или lifetime
DQ = "dq"          # "..."
RAW = "raw"        # r"…" / r#"…"#
BCOM = "bcom"      # /* … */
LCOM = "lcom"      # // …


def strip(src: str) -> str:
    """Код без строковых литералов и комментариев (той же длины, \n сохранены)."""
    out = list(src)
    n = len(src)
    i = 0
    state = CODE
    raw_hashes = 0
    bdepth = 0

    def blank(a, b):
        for k in range(a, min(b, n)):
            if src[k] != "\n":
                out[k] = " "

    while i < n:
        if state == CODE:
            # порядок проверок ВАЖЕН: сначала комментарии, потом raw/byte-строки,
            # потом обычные строки, потом char.
            if src.startswith("//", i):
                state = LCOM
                i += 2
                continue
            if src.startswith("/*", i):
                state = BCOM
                bdepth = 1
                i += 2
                continue
            # raw-строка: r"…, r"…, br"…, rb"… (решётки перед кавычкой)
            j = i
            if src[j] == "b" and j + 1 < n and src[j + 1] == "r":
                j += 2
            elif src[j] == "r" and j + 1 < n and src[j + 1] == "b":
                j += 2
            elif src[j] == "r":
                j += 1
            if j < n and src[j] == "#":
                h = 0
                while j < n and src[j] == "#":
                    h += 1
                    j += 1
                if j < n and src[j] == '"':
                    state = RAW
                    raw_hashes = h
                    i = j + 1          # встали сразу после открывающей кавычки
                    continue
            if j != i:
                # был префикс r/b/br/rb без кавычки — это не строка (lifetime/идент.)
                pass
            # обычная строка "…" или байтовая b"…"
            if src[i] == '"' or (src[i] == "b" and i + 1 < n and src[i + 1] == '"'):
                state = DQ
                i += 2 if src[i] == "b" else 1
                continue
            # char-литерал: 'x', '\n', '\u{1F600}'. Lifetime ('a, '_) пропускаем.
            if src[i] == "'":
                j = i + 1
                if j < n and src[j] == "\\":
                    j += 2
                    if j - 2 < n and src[j - 2 : j] == "u":
                        k = src.find("}", i)
                        if k > 0:
                            j = k + 1
                    if j < n and src[j] == "'":
                        blank(i, j + 1)
                        i = j + 1
                        state = CODE
                        continue
                elif j < n and src[j] not in ("#", "<", "(") and (j + 1 >= n or src[j + 1] == "'"):
                    blank(i, j + 2)
                    i = j + 2
                    state = CODE
                    continue
                i += 1
                continue
            i += 1
            continue

        if state == DQ:
            if src[i] == "\\":
                i += 2
                continue
            if src[i] == '"':
                blank(i - (2 if src[i - 1 : i] else 1), i + 1)  # не важно: ниже общий blank
                state = CODE
                i += 1
                continue
            if src[i] == "\n":  # незакрытая строка — не бушим бесконечно
                state = CODE
                i += 1
                continue
            i += 1
            continue

        if state == RAW:
            term = '"' + "#" * raw_hashes
            if src.startswith(term, i):
                state = CODE
                i += len(term)
                continue
            i += 1
            continue

        if state == BCOM:
            if src.startswith("/*", i):
                bdepth += 1
                i += 2
                continue
            if src.startswith("*/", i):
                bdepth -= 1
                i += 2
                if bdepth == 0:
                    state = CODE
                continue
            i += 1
            continue

        if state == LCOM:
            if src[i] == "\n":
                state = CODE
            i += 1
            continue

    # второй проход: заменяем пробелами всё, что было внутри литералов/комментариев
    # (первый проход только определяет границы, чтобы не дублировать логику)
    return "".join(out)


def blanks(src: str) -> str:
    """Позиционно-точное вырезание: возвращает строку той же длины, где литералы
    и комментарии заменены пробелами (кроме переводов строк)."""
    out = list(src)
    n = len(src)
    i = 0
    state = CODE
    raw_hashes = 0
    bdepth = 0
    lit_start = 0

    def close(a, b):
        for k in range(a, min(b, n)):
            if src[k] != "\n":
                out[k] = " "

    while i < n:
        if state == CODE:
            if src.startswith("//", i):
                state, lit_start = LCOM, i
                i += 2
                continue
            if src.startswith("/*", i):
                state, lit_start, bdepth = BCOM, i, 1
                i += 2
                continue
            # raw: r"…", r#"…"#, br#"…"#, rb"…"
            j = i
            if src.startswith("br", j) or src.startswith("rb", j):
                j += 2
            elif src[j : j + 1] in ("r", "b") and src.startswith("r", j + 1):
                j += 2
            elif src[j : j + 1] == "r":
                j += 1
            if j < n and src[j] == "#":
                h = 0
                while j < n and src[j] == "#":
                    h += 1
                    j += 1
                if j < n and src[j] == '"':
                    state, raw_hashes, lit_start = RAW, h, i
                    i = j + 1
                    continue
            if src[i] == '"' or (src.startswith('b"', i)):
                state, lit_start = DQ, i
                i += 2 if src[i] == "b" else 1
                continue
            if src[i] == "'":
                j = i + 1
                if j < n and src[j] == "\\":
                    j += 2
                    if src[j - 2 : j] == "u":
                        k = src.find("}", i)
                        if k > 0:
                            j = k + 1
                    if j < n and src[j] == "'":
                        close(i, j + 1)
                        i = j + 1
                        continue
                    i += 1
                    continue
                if j < n and src[j] != "'" and j + 1 < n and src[j + 1] == "'":
                    close(i, j + 2)
                    i = j + 2
                    continue
                i += 1  # lifetime
                continue
            i += 1
            continue

        if state == DQ:
            if src[i] == "\\":
                i += 2
                continue
            if src[i] == '"':
                close(lit_start, i + 1)
                state = CODE
                i += 1
                continue
            if src[i] == "\n":
                close(lit_start, i)
                state = CODE
                i += 1
                continue
            i += 1
            continue

        if state == RAW:
            term = '"' + "#" * raw_hashes
            if src.startswith(term, i):
                close(lit_start, i + len(term))
                state = CODE
                i += len(term)
                continue
            i += 1
            continue

        if state == BCOM:
            if src.startswith("/*", i):
                bdepth += 1
                i += 2
                continue
            if src.startswith("*/", i):
                bdepth -= 1
                i += 2
                if bdepth == 0:
                    close(lit_start, i)
                    state = CODE
                continue
            i += 1
            continue

        if state == LCOM:
            if src[i] == "\n":
                close(lit_start, i)
                state = CODE
            i += 1
            continue

    if state in (DQ, RAW, BCOM, LCOM):
        close(lit_start, n)
    res = "".join(out)
    assert len(res) == n, f"инвариант длины нарушен: {len(res)} != {n}"
    return res


import re


def fmt_lines(code: str):
    """Строки (номер, текст) ВНУТРИ вызовов формат-макросов, включая стартовую.

    Нужно, потому что println! у меня часто занимает несколько строк, и проверка
    «println! на той же строке» не ловит собственные баги (раунд 14).
    Скобочный баланс считается по коду, уже очищенному от строк и комментариев.
    """
    MACRO = re.compile(r"\b(?:println|print|format|eprintln|write|writeln)!?\s*\(")
    out = []
    depth = 0
    base = None
    for idx, line in enumerate(code.splitlines(), 1):
        inside = base is not None and depth > base
        if MACRO.search(line):
            inside = True
            if base is None:
                base = depth
        if inside:
            out.append((idx, line))
        for ch in line:
            if ch == "(":
                depth += 1
            elif ch == ")":
                depth -= 1
        if base is not None and depth <= base:
            base = None
    return out


def line_of(src: str, idx: int) -> int:
    return src.count("\n", 0, idx) + 1


def balance(path: str, verbose: bool = True) -> int:
    src = open(path, encoding="utf-8").read()
    clean = blanks(src)
    depth = 0
    events = []
    for i, ch in enumerate(clean):
        if ch not in "{}":
            continue
        ln = line_of(src, i)
        d0 = depth
        depth += 1 if ch == "{" else -1
        events.append((ln, ch, d0, depth, src.splitlines()[ln - 1].strip()[:96]))
    if verbose:
        print(f"{path}: глубина на конце {depth}, событий {len(events)}")
        for e in events[-14:]:
            print(f"  L{e[0]:>4} {e[1]} {e[2]}->{e[3]}  {e[4]}")
    return 0 if depth == 0 else 1


CASES = [
    # (код, ожидаемая глубина, описание)
    ('fn a() { let s = b"{ не json"; }', 0, "байтовая строка со скобкой"),
    ('fn a() { let s = "x{y}z"; }', 0, "обычная строка со скобками"),
    ('fn a() { let s = format!("{e}"); }', 0, "format с именованным аргументом"),
    ('fn a() { let s = format!("{} и {e}", x); }', 0, "format со смешанными плейсхолдерами"),
    ('fn a() { let s = r#"{"v":1}"#; }', 0, "raw-строка с JSON"),
    ('fn a() { let s = r##"y"##; }', 0, "raw с двумя решётками"),
    ('fn a() { let s = br#"z"#; }', 0, "byte raw"),
    ('fn a() { // }\n }', 0, "скобка в строчном комментарии"),
    ('fn a() { /* } */ }', 0, "скобка в блочном комментарии"),
    ('fn a() { /* } /* nested */ } */ }', 0, "вложенный блочный комментарий"),
    ('fn a() { let c = \'{\'; }', 0, "char-литерал со скобкой"),
    ('fn a() { let x = 1;', 1, "НЕЗАКРЫТОЙ блок (ожидаем 1)"),
    ('fn a() { } }', -1, "ЛИШНЯЯ закрывающая (ожидаем -1)"),
    ('fn a() { let t = (1, 2); }', 0, "кортеж, скобок нет"),
    ('fn a() { m.insert(1, ChannelState { id: 5, pts: 6 }); }', 0, "вложенный struct-литерал"),
]


def selftest() -> int:
    bad = 0
    for code, want, why in CASES:
        clean = blanks(code)
        d = clean.count("{") - clean.count("}")
        flag = "ok " if d == want else "FAIL"
        if d != want:
            bad += 1
        print(f"  {flag} {why:<38} ожидали {want:>2}, получили {d:>2}")
        if d != want:
            print(f"      clean = {clean!r}")
    print(f"\nсамопроверка: {len(CASES) - bad} из {len(CASES)} прошли")
    return 0 if bad == 0 else 1


if __name__ == "__main__":
    if "--selftest" in sys.argv:
        sys.exit(selftest())
    rc = 0
    for p in sys.argv[1:]:
        rc |= balance(p)
    sys.exit(rc)
