#!/bin/sh
# Раунд 9. Динамика гейта R1: раунд 6 → 23 ошибки, 7 → 11, 8 → 5.
# Каждое исправление подкреплено строкой из vendor-исходника, и каждая догадка
# добавлена в validate_before_push.py, чтобы падать ДО 6-минутного CI-прогона.
set -u
R=/var/minis/shared/rustspike
OUT=$R/gate_r9.txt
: > $OUT
say() { echo "$@" >> $OUT; }
cd $R || exit 1

say "=== 1) обязательные проверки (токенайзер, красный тест, валидация) ==="
python3 -u braces2.py --selftest > /tmp/self.txt 2>&1 || { say "ПЛОХО: токенайзер"; cat /tmp/self.txt >> $OUT; exit 1; }
say "ok: $(tail -1 /tmp/self.txt)"
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
python3 -u validate_before_push.py /tmp/bad.yml > /tmp/red.txt 2>&1 && { say "ПЛОХО: валидатор не поймал ловушку"; exit 1; }
say "ok красный: $(grep -m1 'YAML ПЛОХО' /tmp/red.txt)"
python3 -u validate_before_push.py > /tmp/green.txt 2>&1 || { say "ВАЛИДАЦИЯ УПАЛА"; cat /tmp/green.txt >> $OUT; exit 1; }
say "ok зелёный"

say "=== 2) push ==="
git add -A
git -c commit.gpgsign=false commit -q -m "раунд 9: unit-варианты TL без скобок (enums.rs:73-77), limit() через переприсваивание, cargo-ndk без пинны версии"
git push -q origin main 2>&1 | grep -vi 'x-access-token' | head -3 >> $OUT
say "push exit=$? HEAD=$(git rev-parse --short HEAD)"

say "=== 3) ждём CI ==="
sleep 360
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
say "=== 4) аннотации упавших ==="
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
    for line in (a.get('message') or '').split(chr(10))[:70]:
        print('      ' + line[:200])
" >> $OUT 2>&1
done

say ""
say "=== 5) логи-артефакты ==="
rm -rf $R/logs9 && mkdir -p $R/logs9 && cd $R/logs9
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
import re, glob
for p in sorted(glob.glob("**/*.log", recursive=True)):
    raw=open(p,"rb").read().decode("utf-8","replace")
    clean=re.sub(r"\x1b\[[0-9;]*m","",raw)
    open(p+".clean","w",encoding="utf-8").write(clean)
    lines=clean.splitlines()
    errs=[i for i,l in enumerate(lines) if l.startswith("error")]
    print(f"--- {p}: {len(lines)} строк, {len(errs)} ошибок")
    for i in errs[:16]:
        for k in range(i, min(i+6, len(lines))):
            print("   ", lines[k][:175])
        print("   ---")
PY
cd $R
say "R9_DONE"
