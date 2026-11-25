#!/bin/sh
# Раунд 14. ГЕЙТ R1 ПРОЙДЕН ПОЛНОСТЬЮ в раунде 13: run 45673919282 —
# host success + android success, все шаги зелёные, артефакт 426133 Б.
# Но таблица `--caps` (сам результат раунда) осталась недоступной: логи job
# требуют scope actions:run, которого у PAT нет. Теперь отчёт грузится ВСЕГДА.
# Сюда же: чистый разбор кода входа в std-only `util.rs` (9 локальных тестов).
set -u
R=/var/minis/shared/rustspike
OUT=$R/gate_r14.txt
: > $OUT
say() { echo "$@" >> $OUT; }
cd $R || exit 1

say "=== 1) обязательные проверки ==="
python3 -u braces2.py --selftest > /tmp/self.txt 2>&1 || { say "ПЛОХО: токенайзер"; cat /tmp/self.txt >> $OUT; exit 1; }
say "ok: $(tail -1 /tmp/self.txt)"
python3 -u braces2.py src/*.rs > /tmp/br.txt 2>&1 || { say "ПЛОХО: скобки"; cat /tmp/br.txt >> $OUT; exit 1; }
say "ok: скобки сбалансированы"
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
EOF
python3 -u validate_before_push.py /tmp/bad.yml > /tmp/red.txt 2>&1 && { say "ПЛОХО: валидатор слеп"; exit 1; }
say "ok красный: $(grep -m1 'YAML ПЛОХО' /tmp/red.txt)"
python3 -u validate_before_push.py > /tmp/green.txt 2>&1 || { say "ВАЛИДАЦИЯ УПАЛА"; cat /tmp/green.txt >> $OUT; exit 1; }
say "ok зелёный:"; grep -E "PURE OK|CARGO OK: симметрия|YAML OK" /tmp/green.txt >> $OUT

say "=== 2) push ==="
git add -A
git -c commit.gpgsign=false commit -q -m "раунд 14: util.rs (чистый разбор кода, 9 локальных тестов), отчёт CI грузится всегда включая caps-таблицу"
git push -q origin main 2>&1 | grep -vi 'x-access-token' | head -3 >> $OUT
say "push exit=$? HEAD=$(git rev-parse --short HEAD)"

say "=== 3) ждём CI ==="
sleep 380
A="Authorization: Bearer $GITHUB_PAT"
RID=$(curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs?per_page=1" \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['workflow_runs'][0]['id'])" 2>/dev/null)
say "run_id=$RID"
python3 -u pull_logs.py "$RID" > /tmp/logs.txt 2>&1
say "=== 4) вердикт ==="
cat /tmp/logs.txt >> $OUT
say ""
say "=== 5) Таблица CAPS (то, ради чего был раунд) ==="
cat logs_pull/host-report/caps.txt >> $OUT 2>/dev/null || say "caps.txt не найден"
say "=== 6) формат сессии ==="
cat logs_pull/host-report/session_format.txt >> $OUT 2>/dev/null || true
cat logs_pull/android-report/android_file.txt >> $OUT 2>/dev/null || true
say "R14_DONE"
