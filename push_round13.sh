#!/bin/sh
# Раунд 13. Раунд 12 дал: build.log 0 warning, 32 из 33 тестов зелёные, одна
# ПАНИКА в чистой логике (Window::wait — неверный индекс, 50 с вместо 40).
# Чтобы такие баги не стоили 6-минутного прогона, чистая логика вынесена в
# std-only `src/rate.rs` и гоняется ЛОКАЛЬНО за 0.03 с (rustc 1.83 может).
set -u
R=/var/minis/shared/rustspike
OUT=$R/gate_r13.txt
: > $OUT
say() { echo "$@" >> $OUT; }
cd $R || exit 1

say "=== 1) обязательные проверки (валидатор сам гоняет rate.rs локально) ==="
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
say "ok зелёный: $(grep -m1 'RATE OK' /tmp/green.txt)"

say "=== 2) push ==="
git add -A
git -c commit.gpgsign=false commit -q -m "раунд 13: чистая логика в std-only rate.rs (локальные тесты 0.03 с), фикс индекса Window + fail-closed, engine slim до grammers-кода"
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
say "R13_DONE"
