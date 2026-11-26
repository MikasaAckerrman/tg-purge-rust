#!/bin/sh
# Раунд 18. ГЕЙТ R1 ПОЛНОСТЬЮ ЗЕЛЁНЫЙ в раунде 17: host success (53 юнит-теста,
# 0 doctest-ловушек, caps-таблица 11/12) + android success (libpurge.so 1 207 376 Б,
# ELF aarch64 «for Android 24, built by NDK r27»). Остался ОДИН warning:
# `unused import: BufRead` — у Stdin read_line встроенный, а не через трейт.
# Цель раунда: сборка без единого warning (иначе они копятся и перестают читаться).
set -u
R=/var/minis/shared/rustspike
OUT=$R/gate_r18.txt
: > $OUT
say() { echo "$@" >> $OUT; }
cd $R || exit 1

say "=== 1) обязательные проверки ==="
python3 -u braces2.py --selftest > /tmp/self.txt 2>&1 || { say "ПЛОХО: токенайзер"; cat /tmp/self.txt >> $OUT; exit 1; }
say "ok: $(tail -1 /tmp/self.txt)"
python3 -u braces2.py src/*.rs > /tmp/br.txt 2>&1 || { say "ПЛОХО: скобки"; cat /tmp/br.txt >> $OUT; exit 1; }
say "ok: скобки сбалансированы"
python3 -u doctest_scan.py > /tmp/dt.txt 2>&1 || { say "ПЛОХО: doctest"; cat /tmp/dt.txt >> $OUT; exit 1; }
say "ok: $(tail -1 /tmp/dt.txt)"
sh red_test_doctest.sh > /tmp/red_dt.txt 2>&1 || { say "ПЛОХО: красный тест doctest"; cat /tmp/red_dt.txt >> $OUT; exit 1; }
grep -q "rc=1 (ожидаем 1)" /tmp/red_dt.txt || { say "ПЛОХО: сканер не краснеет"; exit 1; }
say "ok: doctest-сканер краснеет на ловушке и зеленеет на легальном"
python3 -u validate_before_push.py > /tmp/green.txt 2>&1 || { say "ВАЛИДАЦИЯ УПАЛА"; cat /tmp/green.txt >> $OUT; exit 1; }
say "ok зелёный:"; grep -E "PURE OK|CARGO OK: симметрия|YAML OK|DOCTEST OK" /tmp/green.txt >> $OUT

say "=== 2) паритет Rust↔Python ==="
python3 -u tools/parity.py > /tmp/par.txt 2>&1 || { say "ПАРИТЕТ УПАЛ"; cat /tmp/par.txt >> $OUT; exit 1; }
head -2 /tmp/par.txt >> $OUT

say "=== 3) push ==="
git add -A
git -c commit.gpgsign=false commit -q -m "раунд 18: убираю последний warning (unused import BufRead у Stdin встроенный read_line)"
git push -q origin main 2>&1 | grep -vi 'x-access-token' | head -3 >> $OUT
say "push exit=$? HEAD=$(git rev-parse --short HEAD)"

say "=== 4) ждём CI ==="
sleep 400
A="Authorization: Bearer $GITHUB_PAT"
RID=$(curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs?per_page=1" \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['workflow_runs'][0]['id'])" 2>/dev/null)
say "run_id=$RID"
python3 -u pull_logs.py "$RID" > /tmp/logs.txt 2>&1
say "=== 5) вердикт ==="
cat /tmp/logs.txt >> $OUT
say ""
say "=== 6) warning'ов должно быть НОЛЬ ==="
for f in logs_pull/host-report/build.log.clean logs_pull/host-report/test.log.clean logs_pull/android-report/android.log.clean; do
  [ -f "$f" ] || continue
  say "--- $f: warning=$(grep -c '^warning' $f || true) error=$(grep -c '^error' $f || true) ---"
  grep -E "^warning|^error" "$f" | head -6 >> $OUT
done
say ""
say "=== 7) CAPS + артефакты ==="
cat logs_pull/host-report/caps.txt >> $OUT 2>/dev/null || say "caps.txt нет"
cat logs_pull/android-report/android_file.txt >> $OUT 2>/dev/null || true
ls -la logs_pull/purge-android-arm64/ >> $OUT 2>/dev/null || true
say "R18_DONE"
