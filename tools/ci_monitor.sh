#!/bin/sh
# Монитор CI раунда 21: поллит run 45801133421 каждые 30 с, пишет статус в лог.
# Завершается сам на completed (или через ~30 попыток = 15 минут), чтобы не висеть вечно.
RUN=45806501656
REPO=MikasaAckerrman/tg-purge-rust
i=0
while [ $i -lt 30 ]; do
  i=$((i+1))
  out=$(curl -s -m 30 -H "Authorization: Bearer $GITHUB_PAT" -H "Accept: application/vnd.github+json" \
        "https://api.github.com/repos/$REPO/actions/runs/$RUN")
  st=$(printf '%s' "$out" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('status'),d.get('conclusion'))" 2>/dev/null)
  echo "$(date +%H:%M:%S) попытка $i: $st"
  case "$st" in
    completed*)
      echo "ИТОГ: $st"
      printf '%s' "$out" | python3 -c "
import sys,json
d=json.load(sys.stdin)
print('  conclusion:', d.get('conclusion'))
print('  updated:', d.get('updated_at'), 'run_started:', d.get('run_started_at'))
print('  html_url:', d.get('html_url'))
"
      exit 0
      ;;
  esac
  sleep 30
done
echo "ВЫХОД ПО ЛИМИТУ: статус так и не стал completed за 15 минут"
exit 1
