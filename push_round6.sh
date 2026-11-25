#!/bin/sh
# Раунд 6. Пушим ТОЛЬКО если: (1) самопроверка токенайзера зелёная,
# (2) красный тест на заведомо сломанном YAML пойман, (3) валидатор зелёный.
# Все три проверки — про МОИ OWN ошибки автора пайплайна: раунды 1-5 сгорели
# именно на них (pipe-маска, YAML-двоеточие, Cargo без [package], SIGPIPE-паника,
# panic=abort против cargo test --release), а не на grammers.
set -u
R=/var/minis/shared/rustspike
OUT=$R/gate_r6.txt
: > $OUT
say() { echo "$@" >> $OUT; }
cd $R || exit 1

say "=== 1) самопроверка токенайзера (без неё баланс скобок — гадание) ==="
if python3 -u braces2.py --selftest > /tmp/self.txt 2>&1; then
  say "ok: $(tail -1 /tmp/self.txt)"
else
  say "ПЛОХО: токенайзер сам не проходит свои кейсы"
  cat /tmp/self.txt >> $OUT
  exit 1
fi

say "=== 2) валидатор обязан КРАСНЕТЬ на pipe-ловушке раунда 4 ==="
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
  say "ПЛОХО: валидатор пропустил известную поломку"
  cat /tmp/red.txt >> $OUT
  exit 1
fi
say "ok: $(grep -m1 'YAML ПЛОХО' /tmp/red.txt)"

say "=== 3) валидация текущих файлов ==="
if ! python3 -u validate_before_push.py > /tmp/green.txt 2>&1; then
  say "ВАЛИДАЦИЯ УПАЛА — не пушим"
  cat /tmp/green.txt >> $OUT
  exit 1
fi
say "ok:"
cat /tmp/green.txt >> $OUT

say "=== 4) push ==="
git add -A
git -c commit.gpgsign=false commit -q -m "раунд 6: auth.rs (async get_me, PeerId->i64, fat-handle), braces2 с обязательной самопроверкой, логи наружу через ::error"
git push -q origin main 2>&1 | grep -vi 'x-access-token' | head -3 >> $OUT
say "push exit=$? HEAD=$(git rev-parse --short HEAD)"

say "=== 5) ждём CI и читаем вердикт ==="
sleep 330
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
say "=== 6) АННОТАЦИИ: теперь там полный лог компилятора (::error::) ==="
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
    lvl=a.get('annotation_level')
    if lvl=='warning': continue
    msg=(a.get('message') or '')
    print(f\"  [{lvl}] {a.get('path')}:{a.get('start_line')}\")
    for line in msg.split(chr(10))[:70]:
        print('      ' + line[:200])
" >> $OUT 2>&1
done
say "R6_DONE"
