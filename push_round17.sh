#!/bin/sh
# Раунд 17. Раунд 16: ANDROID снова зелёный (11 шагов), host упал на шаге 6 —
# и это была НЕ ошибка логики, а МОЯ doc-строка: `rustc --test …` с отступом в
# 4 пробела после пустой `//!` = кодовый блок Markdown = DOCTEST, который rustdoc
# пытался скомпилировать как Rust («expected one of `!` or `::`, found -»).
#诊断: if: always() доработал (test.log приехал), поэтому я увидел точную строку
# за один проход, а не за три. Теперь этот класс ловит doctest_scan.py + красный
# тест на нём (red_test_doctest.sh: 3 ловушки пойманы, 2 легальных формы — нет).
set -u
R=/var/minis/shared/rustspike
OUT=$R/gate_r17.txt
: > $OUT
say() { echo "$@" >> $OUT; }
cd $R || exit 1

say "=== 1) обязательные проверки ==="
python3 -u braces2.py --selftest > /tmp/self.txt 2>&1 || { say "ПЛОХО: токенайзер"; cat /tmp/self.txt >> $OUT; exit 1; }
say "ok: $(tail -1 /tmp/self.txt)"
python3 -u braces2.py src/*.rs > /tmp/br.txt 2>&1 || { say "ПЛОХО: скобки"; cat /tmp/br.txt >> $OUT; exit 1; }
say "ok: скобки сбалансированы"

say "=== 2) красный тест doctest-сканера (обязан краснеть) ==="
sh red_test_doctest.sh > /tmp/red_dt.txt 2>&1 || { say "КРАСНЫЙ ТЕСТ СЛОМАЛСЯ"; cat /tmp/red_dt.txt >> $OUT; exit 1; }
grep -q "rc=1 (ожидаем 1)" /tmp/red_dt.txt || { say "ПЛОХО: сканер не краснеет на ловушке"; cat /tmp/red_dt.txt >> $OUT; exit 1; }
grep -q "rc=0 (ожидаем 0)" /tmp/red_dt.txt || { say "ПЛОХО: сканер краснеет на легальном"; cat /tmp/red_dt.txt >> $OUT; exit 1; }
say "ok: $(grep -c DOCTEST /tmp/red_dt.txt) строк DOCTEST, красный/зелёный как ожидается"

say "=== 3) валидация текущих файлов ==="
cat > /tmp/bad.yml <<'EOF'
name: bad
on:
  push:
    branches: [main]
jobs:
  host:
    runs-on: ubuntu-latest
    steps:
      - name: "ловушка раунда 4"
        run: |
          set -o pipefail
          cargo tree --depth 1 2>&1 | head -20
      - name: "сохранить отчёт"
        uses: actions/upload-artifact@v4
        with:
          name: host-report
          path: test.log
EOF
python3 -u validate_before_push.py /tmp/bad.yml > /tmp/red.txt 2>&1 && { say "ПЛОХО: валидатор слеп"; exit 1; }
say "ok красный: $(grep -m1 -E 'YAML ПЛОХО' /tmp/red.txt)"
python3 -u validate_before_push.py > /tmp/green.txt 2>&1 || { say "ВАЛИДАЦИЯ УПАЛА"; cat /tmp/green.txt >> $OUT; exit 1; }
say "ok зелёный:"; grep -E "PURE OK|CARGO OK: симметрия|YAML OK|DOCTEST OK" /tmp/green.txt >> $OUT

say "=== 4) паритет Rust↔Python ==="
python3 -u tools/parity.py > /tmp/par.txt 2>&1 || { say "ПАРИТЕТ УПАЛ"; cat /tmp/par.txt >> $OUT; exit 1; }
head -2 /tmp/par.txt >> $OUT

say "=== 5) push ==="
git add -A
git -c commit.gpgsign=false commit -q -m "раунд 17: doctest-ловушка в doc-заголовке (rustc-команда отступом = кодовый блок = doctest), фенс \`\`\`text + doctest_scan.py + красный тест на нём"
git push -q origin main 2>&1 | grep -vi 'x-access-token' | head -3 >> $OUT
say "push exit=$? HEAD=$(git rev-parse --short HEAD)"

say "=== 6) ждём CI ==="
sleep 400
A="Authorization: Bearer $GITHUB_PAT"
RID=$(curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs?per_page=1" \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['workflow_runs'][0]['id'])" 2>/dev/null)
say "run_id=$RID"
python3 -u pull_logs.py "$RID" > /tmp/logs.txt 2>&1
say "=== 7) вердикт ==="
cat /tmp/logs.txt >> $OUT
say ""
say "=== 8) ошибки компиляции/тестов (если есть) ==="
for f in logs_pull/host-report/build.log.clean logs_pull/host-report/test.log.clean; do
  [ -f "$f" ] || continue
  say "--- $f ---"
  grep -E "^error|^warning|test result|panicked" "$f" | head -14 >> $OUT
done
say ""
say "=== 9) CAPS-таблица ==="
cat logs_pull/host-report/caps.txt >> $OUT 2>/dev/null || say "caps.txt нет"
say "=== 10) android-артефакт ==="
cat logs_pull/android-report/android_file.txt >> $OUT 2>/dev/null || say "android_file.txt нет"
say "R17_DONE"
