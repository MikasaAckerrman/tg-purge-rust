#!/bin/sh
# Раунд 5: (1) доказываем, что валидатор краснеет на реальной ловушке раунда 4,
# (2) валидируем текущие файлы, (3) пушим, (4) читаем вердикт CI.
set -u
R=/var/minis/shared/rustspike
OUT=$R/gate_r5.txt
: > $OUT
say() { echo "$@" >> $OUT; }
cd $R || exit 1

say "=== 1) валидатор обязан КРАСНЕТЬ на pipe-ловушке раунда 4 ==="
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
if python3 -u validate_before_push.py /tmp/bad.yml > /tmp/red.log 2>&1; then
  say "ПЛОХО: валидатор пропустил известную поломку (exit 0)"
  cat /tmp/red.log >> $OUT
  exit 1
fi
say "ок: валидатор поймал ловушку"
grep -E "YAML ПЛОХО" /tmp/red.log >> $OUT

say "=== 2) валидация текущих файлов ==="
if ! python3 -u validate_before_push.py > /tmp/green.log 2>&1; then
  say "ВАЛИДАЦИЯ УПАЛА — не пушим"
  cat /tmp/green.log >> $OUT
  exit 1
fi
say "ок:"
cat /tmp/green.log >> $OUT

say "=== 3) push ==="
git add -A
git -c commit.gpgsign=false commit -q -m "раунд 5: lib.rs+store.rs (персистентная сессия), CI без pipe-ловушек, NDK напрямую, тесты в dev-профиле"
git push -q origin main 2>&1 | grep -vi 'x-access-token' | head -3 >> $OUT
say "push exit=$? HEAD=$(git rev-parse --short HEAD)"

say "=== 4) ждём CI ==="
sleep 300
A="Authorization: Bearer $GITHUB_PAT"
RID=$(curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs?per_page=1" \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['workflow_runs'][0]['id'])" 2>/dev/null)
say "run_id=$RID"
curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs/$RID" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
print('state:', d.get('status'), '/', d.get('conclusion'))
" >> $OUT 2>&1
curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs/$RID/jobs" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
print('jobs:', d.get('total_count'))
for j in d.get('jobs',[]):
    print(f\"\nJOB {j['name']}: {j['status']} / {j.get('conclusion')}\")
    for s in j.get('steps',[]):
        c=s.get('conclusion') or 'running'
        mark={'success':'OK  ','failure':'FAIL','skipped':'skip'}.get(c,'run ')
        print(f\"   {mark} {s['number']:>2} {s['name']}\")
" >> $OUT 2>&1

say ""
say "=== 5) аннотации упавших ==="
for JID in $(curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs/$RID/jobs" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
for j in d.get('jobs',[]):
    if j.get('conclusion')=='failure': print(j['id'])
" 2>/dev/null); do
  say "--- job $JID ---"
  curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/check-runs/$JID/annotations" \
    | python3 -c "
import sys,json
d=json.load(sys.stdin)
if isinstance(d,dict): print('  ERR', d.get('message')); sys.exit()
for a in d[:25]: print('  -', a.get('annotation_level'), a.get('path'), a.get('start_line'), '|', (a.get('message') or '')[:260])
" >> $OUT 2>&1
done
say "R5_DONE"
