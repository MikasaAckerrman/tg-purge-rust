#!/bin/sh
# Раунд 3: YAML валидируется ДО пуша (раунд 2 сгорел с 0 jobs из-за двоеточия
# в некавированном name — это моя ошибка авторства CI, а не grammers).
set -u
R=/var/minis/shared/rustspike
OUT=$R/gate_r3.txt
: > $OUT
say() { echo "$@" >> $OUT; }

cd $R || exit 1

say "=== 1) валидация YAML до пуша ==="
if python3 -c "
import yaml,sys
d=yaml.safe_load(open('.github/workflows/rust.yml'))
jobs=d.get('jobs',{})
print('jobs:', list(jobs.keys()))
for n,j in jobs.items(): print('  ', n, 'steps', len(j.get('steps',[])))
" 2>&1; then say "yaml OK"; else say "YAML ПЛОХОЙ — не пушим"; exit 1; fi

say "=== 2) валидация Cargo.toml (parsecrate) ==="
python3 -c "
import tomllib
d=tomllib.load(open('Cargo.toml','rb'))
print('package:', d['package']['name'])
print('deps:', sorted(d['dependencies'].keys()))
print('features client:', d['dependencies']['grammers-client'])
" 2>&1 | tee -a $OUT

say "=== 3) push ==="
git add -A
git -c commit.gpgsign=false commit -q -m "раунд 3: YAML в кавычках (раунд 2 сгорел на двоеточии в name), serde-фича сессии без sqlite"
git push -q origin main 2>&1 | grep -vi 'x-access-token' | head -3 >> $OUT
say "push exit=$? HEAD=$(git rev-parse --short HEAD)"

say "=== 4) ждём CI и читаем вердикт ==="
sleep 210
A="Authorization: Bearer $GITHUB_PAT"
RID=$(curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs?per_page=1" \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['workflow_runs'][0]['id'])" 2>/dev/null)
say "run_id=$RID"
curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs/$RID" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
print('state:', d.get('status'), '/', d.get('conclusion'), '| attempts', d.get('run_attempt'), '| created', d.get('created_at'))
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
say "R3_DONE"
