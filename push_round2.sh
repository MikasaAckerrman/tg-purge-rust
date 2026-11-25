#!/bin/sh
# Пуш исправлений гейта R1 + вердикт. Всё через bg_run (shell_execute завис на drain).
set -u
R=/var/minis/shared/rustspike
A="Authorization: Bearer $GITHUB_PAT"
OUT=$R/gate_r1_round2.txt
: > $OUT
say() { echo "$@" >> $OUT; }

cd $R || exit 1
git add -A
git -c commit.gpgsign=false commit -q -m "гейт R1 раунд 2: pipefail в CI, caps по фактам из исходников (AutoSleep не объявлен, Message::id() метод)"
git push -q origin main 2>&1 | grep -vi 'x-access-token' | head -3 >> $OUT
say "push exit=$? HEAD=$(git rev-parse --short HEAD)"

sleep 150
RID=$(curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs?per_page=1" \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['workflow_runs'][0]['id'])" 2>/dev/null)
say "run_id=$RID"
curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs/$RID" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
print('state:', d.get('status'), '/', d.get('conclusion'), '| attempt', d.get('run_attempt'))
" >> $OUT 2>&1

curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs/$RID/jobs" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
for j in d.get('jobs',[]):
    print(f\"\nJOB {j['name']}: {j['status']} / {j.get('conclusion')}\")
    for s in j.get('steps',[]):
        c = s.get('conclusion') or 'running'
        mark = {'success':'OK  ','failure':'FAIL','skipped':'skip','neutral':'neutral'}.get(c,'run ')
        print(f\"   {mark} {s['number']:>2} {s['name']}\")
" >> $OUT 2>&1

say ""
say "=== логи упавших job (текстовые, не zip) ==="
for JID in $(curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs/$RID/jobs" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
for j in d.get('jobs',[]):
    if j.get('conclusion')=='failure': print(j['id'])
" 2>/dev/null); do
  say "--- job $JID ---"
  curl -s -m 40 -H "$A" -H "Accept: application/vnd.github.raw" \
    "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/jobs/$JID" 2>/dev/null | head -5 >> $OUT
  # логи отдельных шагов недоступны через REST без annotations; берём аннотации
  curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/check-runs/$JID/annotations" \
    | python3 -c "
import sys,json
d=json.load(sys.stdin)
if isinstance(d,dict): print('  ERR', d.get('message')); sys.exit()
for a in d[:12]: print('  -', a.get('annotation_level'), a.get('message')[:160])
" >> $OUT 2>&1
done
say "ROUND2_DONE"
