#!/bin/sh
# Раунд 12. Раунд 11: ANDROID-ЗЕЛЁНЫЙ (libpurge.so 307216 Б, ELF aarch64, NDK r27),
# host-сборка ОК, юнит-тесты упали на ОДНОЙ ошибке (E0277: unwrap_err требует T: Debug)
# плюс 4 warning'а — все мои. Здесь чиню все 5 и убираю хардкод списка зависимостей
# из валидатора (он сам стал источником лжи: требовал tiny_http после её выноса).
set -u
R=/var/minis/shared/rustspike
OUT=$R/gate_r12.txt
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
say "ok зелёный: $(grep -m1 'CARGO OK: симметрия' /tmp/green.txt)"

say "=== 2) push ==="
git add -A
git -c commit.gpgsign=false commit -q -m "раунд 12: unwrap_err требует T: Debug (явный match), выход run_stream по Dropped, убрать unused HashMap/tiny_http, симметрия зависимостей в валидаторе"
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
say "R12_DONE"
