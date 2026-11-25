#!/bin/sh
# Раунд 16. Раунд 15 упал на E0004 (я добавил CodeError::NotE164 и не покрыл его в
# impl From) — и это ПЕРВОЕ падение, где `if: always()` сработал: test.log/build.log
# доехали, и я увидел точную строку, а не гадал. Теперь этот класс ловит валидатор
# локально (проверено красным прогоном).
# Сюда же: паритет Rust↔Python (2029 сценариев, 0 расхождений) и общий контракт
# «запрос из прошлого → консервативный отказ».
set -u
R=/var/minis/shared/rustspike
OUT=$R/gate_r16.txt
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

say "=== 2) паритет Rust↔Python (чистая логика лимитов) ==="
python3 -u tools/parity.py > /tmp/par.txt 2>&1 || { say "ПАРИТЕТ УПАЛ"; cat /tmp/par.txt >> $OUT; exit 1; }
cat /tmp/par.txt >> $OUT

say "=== 3) push ==="
git add -A
git -c commit.gpgsign=false commit -q -m "раунд 16: паритет Rust↔Python 2029/0, общий контракт отказа при запросе из прошлого, BadPhone + ассерт полноты impl From"
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
say "=== 6) ошибки компиляции (если были) ==="
grep -E "^error" logs_pull/host-report/build.log.clean 2>/dev/null | head -12 >> $OUT || say "build.log чист"
sed -n '1,40p' logs_pull/host-report/test.log.clean 2>/dev/null | grep -E "^error|test result|panicked" | head -12 >> $OUT || true
say ""
say "=== 7) CAPS-таблица ==="
cat logs_pull/host-report/caps.txt >> $OUT 2>/dev/null || say "caps.txt нет"
say "R16_DONE"
