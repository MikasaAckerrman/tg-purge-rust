#!/bin/sh
# Раунд 4: сначала локальная валидация (раунды 2 и 3 сгорели на моих же ошибках
# авторства: YAML-двоеточие и Cargo.toml без [package]), потом пуш, потом вердикт.
set -u
R=/var/minis/shared/rustspike
OUT=$R/gate_r4.txt
: > $OUT
say() { echo "$@" >> $OUT; }

cd $R || exit 1

say "=== 1) валидация ДО пуша ==="
if python3 -u validate_before_push.py >> $OUT 2>&1; then
  say "валидация прошла"
else
  say "ВАЛИДАЦИЯ УПАЛА — не пушим"
  exit 1
fi

say "=== 2) push ==="
git add -A
git -c commit.gpgsign=false commit -q -m "раунд 4: [package] в Cargo.toml (раунд 3 сгорел на нём), валидатор до пуша"
git push -q origin main 2>&1 | grep -vi 'x-access-token' | head -3 >> $OUT
say "push exit=$? HEAD=$(git rev-parse --short HEAD)"

say "=== 3) ждём CI ==="
sleep 240
A="Authorization: Bearer $GITHUB_PAT"
RID=$(curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs?per_page=1" \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['workflow_runs'][0]['id'])" 2>/dev/null)
say "run_id=$RID"
curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs/$RID" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
print('state:', d.get('status'), '/', d.get('conclusion'), '| created', d.get('created_at'))
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
say "=== 4) аннотации упавших job (точные строки ошибок компилятора) ==="
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
for a in d[:25]: print('  -', a.get('annotation_level'), a.get('path'), a.get('start_line'), '|', (a.get('message') or '')[:220])
" >> $OUT 2>&1
done
say "R4_DONE"
