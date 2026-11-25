#!/bin/sh
# Выложить Rust-спайк в CI и получить вердикт гейта R1.
# Всё через bg_run: shell_execute в этой сессии завис на drain, а bg-канал работает.
set -u
R=/var/minis/shared/rustspike
A="Authorization: Bearer $GITHUB_PAT"
OUT=$R/gate_r1.txt
: > $OUT

say() { echo "$@" >> $OUT; }

say "=== 0) проверка полей Message (чтобы не гадать) ==="
grep -rhoE "pub fn (id|user_id|outgoing|peer_id|text)\b" $R/vendor/grammers-client-0.12.0/src/types/message.rs 2>/dev/null | sort -u | tr '\n' ' ' >> $OUT
say ""

say "=== 1) создание репозитория ==="
curl -s -m 30 -H "$A" -X POST "https://api.github.com/user/repos" \
  -d '{"name":"tg-purge-rust","private":false,"description":"Rust-спайк TGPURGE на grammers 0.12: гейты сборки под Android и компиляционные доказательства API","has_issues":false,"has_wiki":false}' \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
print('repo:', d.get('full_name'), '| private:', d.get('private'), '| err:', d.get('message',''))
" >> $OUT 2>&1

cd $R || { say "нет каталога"; exit 1; }
rm -rf .git
git init -q -b main
git config user.email minis@localhost
git config user.name Minis
# vendor/ не пушим (он в .gitignore) — CI возьмёт grammers с crates.io
git add -A
git -c commit.gpgsign=false commit -q -m "Rust-спайк: гейт R1 (сборка Android) + caps (компиляционные доказательства grammers 0.12)"
URL="https://x-access-token:${GITHUB_PAT}@github.com/MikasaAckerrman/tg-purge-rust.git"
git remote add origin "$URL"
say "=== 2) push ==="
git push -q origin main 2>&1 | grep -vi 'x-access-token' | head -3 >> $OUT
say "push exit=$?"
say "HEAD=$(git rev-parse --short HEAD)"

sleep 25
say "=== 3) запуски CI ==="
curl -s -m 25 -H "$A" "https://api.github.com/repos/MikasaAckerrman/tg-purge-rust/actions/runs?per_page=4" \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
print('total', d.get('total_count'), d.get('message',''))
for r in d.get('workflow_runs',[]):
    print(f\"  id={r['id']} {r['name']} | {r['status']} | {r.get('conclusion')} | sha {r['head_sha'][:7]}\")
" >> $OUT 2>&1
say "DONE"
