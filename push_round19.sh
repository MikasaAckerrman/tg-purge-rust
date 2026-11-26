#!/bin/sh
# Раунд 19. Гейт R1 зелёный (раунд 18), добавлен СЛОЙ ФОРМЫ JSON:
#   src/shape.rs (boльше serde_json, без grammers) ⇒ тестируется ЛОКАЛЬНО:
#   tools/localtest (cargo test, 18 passed, 0 warning) + tools/parity_json.py
#   (паритет с Python-эталоном core/model.py::no_null на 157 структурах).
# Паритет уже нашёл три реальных дрейфа таблиц ключей — это и есть его цель.
set -u
R=/var/minis/shared/rustspike
OUT=$R/gate_r19.txt
: > $OUT
say() { echo "$@" >> $OUT; }
cd $R || exit 1

say "=== 1) локальные проверки (все обязаны быть зелёными) ==="
python3 -u braces2.py --selftest > /tmp/self.txt 2>&1 || { say "ПЛОХО: токенайзер"; cat /tmp/self.txt >> $OUT; exit 1; }
say "ok: $(tail -1 /tmp/self.txt)"
python3 -u braces2.py src/*.rs > /tmp/br.txt 2>&1 || { say "ПЛОХО: скобки"; cat /tmp/br.txt >> $OUT; exit 1; }
say "ok: скобки сбалансированы во всех src/*.rs"
python3 -u doctest_scan.py > /tmp/dt.txt 2>&1 || { say "ПЛОХО: doctest"; cat /tmp/dt.txt >> $OUT; exit 1; }
say "ok: $(tail -1 /tmp/dt.txt)"
sh red_test_doctest.sh > /tmp/red.txt 2>&1 || { say "ПЛОХО: красный тест doctest"; cat /tmp/red.txt >> $OUT; exit 1; }
grep -q "rc=1 (ожидаем 1)" /tmp/red.txt || { say "ПЛОХО: сканер не краснеет"; exit 1; }
say "ok: doctest-сканер краснеет на ловушке и зеленеет на легальном"

say "=== 2) localtest (боевой shape.rs, cargo) ==="
(cd tools/localtest && timeout 400 cargo test > /tmp/lt.txt 2>&1) || { say "ПЛОХО: localtest"; tail -30 /tmp/lt.txt >> $OUT; exit 1; }
grep -E "^test result" /tmp/lt.txt | tail -1 >> $OUT
nwarn=$(grep -c "^warning" /tmp/lt.txt || true)
say "warning'ов в localtest: $nwarn"
[ "$nwarn" = "0" ] || { say "ПЛОХО: warning в боевом shape.rs"; exit 1; }

say "=== 3) паритеты ==="
python3 -u tools/parity.py > /tmp/p1.txt 2>&1 || { say "ПАРИТЕТ окон УПАЛ"; tail -20 /tmp/p1.txt >> $OUT; exit 1; }
head -1 /tmp/p1.txt >> $OUT
python3 -u tools/parity_json.py > /tmp/p2.txt 2>&1 || { say "ПАРИТЕТ JSON УПАЛ"; tail -20 /tmp/p2.txt >> $OUT; exit 1; }
head -1 /tmp/p2.txt >> $OUT

say "=== 4) валидация пуша ==="
python3 -u validate_before_push.py > /tmp/v.txt 2>&1 || { say "ВАЛИДАЦИЯ УПАЛА"; tail -40 /tmp/v.txt >> $OUT; exit 1; }
say "ok:"; grep -E "PURE OK|LOCALTEST OK|CARGO OK: симметрия|YAML OK|DOCTEST OK" /tmp/v.txt >> $OUT

say "=== 5) Python-набор (эталон паритетов) ==="
(cd /var/minis/shared/tg && python3 -W ignore -m unittest discover -s tests -t . > /tmp/py.txt 2>&1) || { say "ПЛОХО: Python-тесты"; tail -20 /tmp/py.txt >> $OUT; exit 1; }
grep -E "^Ran |^OK|^FAILED" /tmp/py.txt >> $OUT

say "=== 6) push ==="
git add -A
git -c commit.gpgsign=false commit -q -m "раунд 19: слой формы JSON src/shape.rs (локальные тесты 18/0 warning + паритет no_null 157 структур), AMBIGUOUS_KEYS для ключей с путезависимым типом"
git push -q origin main 2>&1 | grep -vi 'x-access-token' | head -3 >> $OUT
say "push exit=$? HEAD=$(git rev-parse --short HEAD)"

say "=== 7) ждём CI ==="
sleep 400
A="Authorization: Bearer $GITHUB_PAT"
RID=$(curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs?per_page=1" \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['workflow_runs'][0]['id'])" 2>/dev/null)
say "run_id=$RID"
python3 -u pull_logs.py "$RID" > /tmp/logs.txt 2>&1
say "=== 8) вердикт ==="
cat /tmp/logs.txt >> $OUT
say ""
say "=== 9) warning/error по логам ==="
for f in logs_pull/host-report/build.log.clean logs_pull/host-report/test.log.clean logs_pull/android-report/android.log.clean; do
  [ -f "$f" ] || continue
  say "--- $(basename $f): warning=$(grep -c '^warning' $f || true) error=$(grep -c '^error' $f || true) ---"
  grep -E "^warning|^error" "$f" | head -5 >> $OUT
done
grep -E "^test result" logs_pull/host-report/test.log.clean >> $OUT 2>/dev/null || true
say "R19_DONE"
