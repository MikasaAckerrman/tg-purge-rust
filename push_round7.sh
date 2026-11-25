#!/bin/sh
# Раунд 7. Пушим только после: (1) самопроверка токенайзера, (2) красный тест
# валидатора на заведомо сломанном YAML, (3) зелёная валидация текущих файлов.
# Раунды 1-6 сгорели на МОИХ ошибках (pipe-маска, YAML-двоеточие, Cargo без
# [package], SIGPIPE-паника, panic=abort, 23 догадки о типах grammers).
set -u
R=/var/minis/shared/rustspike
OUT=$R/gate_r7.txt
: > $OUT
say() { echo "$@" >> $OUT; }
cd $R || exit 1

say "=== 1) самопроверка токенайзера ==="
if python3 -u braces2.py --selftest > /tmp/self.txt 2>&1; then
  say "ok: $(tail -1 /tmp/self.txt)"
else
  say "ПЛОХО: токенайзер не проходит свои кейсы"; cat /tmp/self.txt >> $OUT; exit 1
fi

say "=== 2) валидатор обязан КРАСНЕТЬ на поломанном входе ==="
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
if python3 -u validate_before_push.py /tmp/bad.yml > /tmp/red.txt 2>&1; then
  say "ПЛОХО: валидатор пропустил известную поломку"; cat /tmp/red.txt >> $OUT; exit 1
fi
say "ok: $(grep -m1 'YAML ПЛОХО' /tmp/red.txt)"

say "=== 3) валидация текущих файлов ==="
if ! python3 -u validate_before_push.py > /tmp/green.txt 2>&1; then
  say "ВАЛИДАЦИЯ УПАЛА — не пушим"; cat /tmp/green.txt >> $OUT; exit 1
fi
say "ok:"; cat /tmp/green.txt >> $OUT

say "=== 4) push ==="
git add -A
git -c commit.gpgsign=false commit -q -m "раунд 7: 23 ошибки раунда 6 починены по исходникам vendor (PeerRef::to_ambient_ref, get_permissions(chat,user), invoke(&req), channel:InputChannel, update::Message), NDK-каталог по факту установки"
git push -q origin main 2>&1 | grep -vi 'x-access-token' | head -3 >> $OUT
say "push exit=$? HEAD=$(git rev-parse --short HEAD)"

say "=== 5) ждём CI ==="
sleep 360
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
say "=== 6) аннотации (там ::error с полным логом rustc) ==="
for JID in $(curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs/$RID/jobs" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
for j in d.get('jobs',[]):
    if j.get('conclusion')=='failure': print(j['id'])
" 2>/dev/null); do
  say "--- job $JID ---"
  curl -s -m 30 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/check-runs/$JID/annotations" \
    | python3 -c "
import sys,json
d=json.load(sys.stdin)
if isinstance(d,dict): print('  ERR', d.get('message')); sys.exit()
for a in d:
    if a.get('annotation_level')=='warning': continue
    print(f\"  [{a.get('annotation_level')}] {a.get('path')}:{a.get('start_line')}\")
    for line in (a.get('message') or '').split(chr(10))[:80]:
        print('      ' + line[:200])
" >> $OUT 2>&1
done

say ""
say "=== 7) забираем логи-артефакты и чистим ANSI ==="
curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs/$RID/artifacts" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
for a in d.get('artifacts',[]): print(a['id'], a['name'], a['size_in_bytes'])
" >> $OUT 2>&1
rm -rf $R/logs7 && mkdir -p $R/logs7 && cd $R/logs7
for pair in $(curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs/$RID/artifacts" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
for a in d.get('artifacts',[]): print(f\"{a['id']}:{a['name']}\")
"); do
  id="${pair%%:*}"; nm="${pair##*:}"
  curl -sL -m 90 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/artifacts/$id/zip" -o $nm.zip >/dev/null 2>&1
  mkdir -p $nm && (cd $nm && unzip -o ../$nm.zip >/dev/null 2>&1)
done
python3 - <<'PY'
import re, os, glob
for p in glob.glob("**/*.log", recursive=True):
    raw=open(p,"rb").read().decode("utf-8","replace")
    clean=re.sub(r"\x1b\[[0-9;]*m","",raw)
    open(p+".clean","w",encoding="utf-8").write(clean)
    lines=clean.splitlines()
    errs=[i for i,l in enumerate(lines) if l.startswith("error")]
    print(f"--- {p}: {len(lines)} строк, ошибок {len(errs)}")
    for i in errs[:12]:
        for k in range(i, min(i+6, len(lines))):
            print("   ", lines[k][:170])
        print("   ---")
PY
cd $R
say "R7_DONE"
