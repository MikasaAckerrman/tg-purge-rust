#!/bin/sh
# Раунд 15. Раунд 14: android снова зелёный, host упал на юнит-тестах, и — главное —
# отчёт НЕ сохранился (шаги после падения пропускаются), поэтому я чинил бы вслепую.
# Теперь: `if: always()` на сборе диагностики + ассерт на него в валидаторе.
# Сюда же: CLI входа --auth / --auth-status (Ф-R3) и 17 локальных тестов util.rs.
set -u
R=/var/minis/shared/rustspike
OUT=$R/gate_r15.txt
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
      - name: "сохранить отчёт"
        uses: actions/upload-artifact@v4
        with:
          name: host-report
          path: test.log
EOF
python3 -u validate_before_push.py /tmp/bad.yml > /tmp/red.txt 2>&1 && { say "ПЛОХО: валидатор слеп"; exit 1; }
say "ok красный: $(grep -m1 -E 'YAML ПЛОХО' /tmp/red.txt)"
python3 -u validate_before_push.py > /tmp/green.txt 2>&1 || { say "ВАЛИДАЦИЯ УПАЛА"; cat /tmp/green.txt >> $OUT; exit 1; }
say "ok зелёный:"; grep -E "PURE OK|CARGO OK: симметрия|YAML OK" /tmp/green.txt >> $OUT

say "=== 2) push ==="
git add -A
git -c commit.gpgsign=false commit -q -m "раунд 15: if: always() на диагностике + ассерт, CLI входа --auth/--auth-status, util.rs 17 тестов (confirm_send, check_phone, пути)"
git push -q origin main 2>&1 | grep -vi 'x-access-token' | head -3 >> $OUT
say "push exit=$? HEAD=$(git rev-parse --short HEAD)"

say "=== 3) ждём CI ==="
sleep 400
A="Authorization: Bearer $GITHUB_PAT"
RID=$(curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs?per_page=1" \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['workflow_runs'][0]['id'])" 2>/dev/null)
say "run_id=$RID"
python3 -u pull_logs.py "$RID" > /tmp/logs.txt 2>&1
say "=== 4) вердикт ==="
cat /tmp/logs.txt >> $OUT
say ""
say "=== 5) тест-лог (ошибки компиляции тестов) ==="
sed -n '1,80p' logs_pull/host-report/test.log.clean >> $OUT 2>/dev/null || say "test.log нет"
say ""
say "=== 6) CAPS-таблица ==="
cat logs_pull/host-report/caps.txt >> $OUT 2>/dev/null || say "caps.txt нет"
say "R15_DONE"
