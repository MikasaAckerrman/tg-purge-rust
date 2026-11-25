#!/bin/sh
# Вердикт гейта R1: читаем статусы job и шаги из CI.
set -u
A="Authorization: Bearer $GITHUB_PAT"
R=MikasaAckerrman/tg-purge-rust
OUT=/var/minis/shared/rustspike/gate_r1_verdict.txt
: > $OUT

RID=$(curl -s -m 25 -H "$A" "https://api.github.com/repos/$R/actions/runs?per_page=1" \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['workflow_runs'][0]['id'])" 2>/dev/null)
echo "run_id=$RID" >> $OUT

curl -s -m 25 -H "$A" "https://api.github.com/repos/$R/actions/runs/$RID/jobs" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
for j in d.get('jobs',[]):
    print(f\"\nJOB {j['name']}: {j['status']} / {j.get('conclusion')}  (started {j.get('started_at')})\")
    for s in j.get('steps',[]):
        mark = {'success':'OK ','failure':'FAIL','skipped':'skip','neutral':'neutral'}.get(s.get('conclusion') or '', 'run ')
        print(f\"   {mark} {s['number']:>2} {s['name']}\")
" >> $OUT 2>&1

echo "" >> $OUT
echo "=== сырой лог упавшего шага (первые 60 строк) ===" >> $OUT
JID=$(curl -s -m 25 -H "$A" "https://api.github.com/repos/$R/actions/runs/$RID/jobs" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
for j in d.get('jobs',[]):
    if j.get('conclusion')=='failure': print(j['id']); break
" 2>/dev/null)
if [ -n "${JID:-}" ]; then
  cd /tmp && rm -f jl.zip && mkdir -p jl && cd jl
  curl -sL -m 60 -H "$A" "https://api.github.com/repos/$R/actions/jobs/$JID/logs" -o l.zip -w "logs code=%{http_code} size=%{size_download}\n" >> $OUT
  if unzip -o l.zip >/dev/null 2>&1; then
    for f in *.txt; do echo "--- $f ---" >> $OUT; grep -inE "error|failed|cannot find|no such|warning: unused" "$f" | head -40 >> $OUT; done
  else
    echo "не zip (скорее всего 404: минуты private или нет прав на логи)" >> $OUT
  fi
fi
echo "VERDICT_DONE" >> $OUT
