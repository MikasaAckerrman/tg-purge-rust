#!/bin/sh
# Прогон ПРЕДИКАТОВ шага «проверить артефакты» против РЕАЛЬНОГО снапшота артефакта,
# скачанного из CI, а не против моих представлений о нём.
#
# Зачем (раунд 23, 26.11): я упал на этом шаге ДВАЖДЫ подряд, и оба раза причина была
# одна — предикат написан против того, чего в артефакте нет:
#   раунд 21: требовал `Type:.*EXEC`, а aarch64-release собирается как PIE
#             («Type: DYN (Position-Independent Executable file)»);
#   раунд 22: требовал `ld-android` в выводе `readelf -d`, а PT_INTERP там НЕ ЖИВЁТ
#             (readelf -d печатает динамическую секцию; интерпретатор — это PT_INTERP,
#             его видно в `readelf -l` и в выводе `file`: «interpreter /system/bin/linker64»).
#             Проверка не могла пройти НИКОГДА — я её написал на догадке о формате.
# Локальный прогон против снапшота ловит оба класса ДО пуша и стоит секунды, тогда
# как CI — 6 минут и красный раунд.
#
# Использование:
#   sh tools/ci_gate_selftest.sh <каталог со снапшотом>
# где каталог содержит android_file.txt / hdr.txt / dyn.txt (как в артефакте
# android-report). Без аргумента — скачивает свежий android-report сам.
set -eu

REPO=MikasaAckerrman/tg-purge-rust
DIR="${1:-}"
if [ -z "$DIR" ]; then
  [ -n "${GITHUB_PAT:-}" ] || { echo "нет снапшота и нет GITHUB_PAT"; exit 2; }
  RUN=$(curl -s -m 30 -H "Authorization: Bearer $GITHUB_PAT" \
        "https://api.github.com/repos/$REPO/actions/workflows/rust.yml/runs?per_page=1" \
        | python3 -c "import sys,json;print(json.load(sys.stdin)['workflow_runs'][0]['id'])")
  AID=$(curl -s -m 30 -H "Authorization: Bearer $GITHUB_PAT" \
        "https://api.github.com/repos/$REPO/actions/runs/$RUN/artifacts" \
        | python3 -c "
import sys,json
for a in json.load(sys.stdin).get('artifacts',[]):
    if a['name']=='android-report': print(a['id']); break
")
  DIR=/tmp/ci_snap
  rm -rf "$DIR"; mkdir -p "$DIR"
  curl -s -m 60 -L -H "Authorization: Bearer $GITHUB_PAT" \
       "https://api.github.com/repos/$REPO/actions/artifacts/$AID/zip" -o /tmp/ar.zip
  unzip -o -q /tmp/ar.zip -d "$DIR"
  echo "снапшот скачан из run $RUN (artifact $AID) → $DIR"
fi

BIN="$DIR/purge"          # если в снапшоте есть сам бинарь, проверяем и на нём
cd "$DIR"
fails=0
chk() {  # chk <имя> <условие-ожидание-ok(0/1)> <команда>
  name="$1"; want="$2"; shift 2
  if "$@" >/dev/null 2>&1; then got=0; else got=1; fi
  if [ "$got" = "$want" ]; then
    echo "  OK   $name"
  else
    echo "  ПЛОХО $name: ожидалось $want, получено $got"
    fails=$((fails+1))
  fi
}

echo "=== предикаты против снапшота ==="
[ -f android_file.txt ] || { echo "нет android_file.txt — снапшот неполон"; exit 2; }
[ -f hdr.txt ] || { echo "нет hdr.txt"; exit 2; }
[ -f dyn.txt ] || { echo "нет dyn.txt"; exit 2; }

# 1) readelf -h: Class / Machine / Type
chk "hdr: ELF64"            0 grep -q "Class:.*ELF64" hdr.txt
chk "hdr: AArch64"          0 grep -q "AArch64" hdr.txt
chk "hdr: Type EXEC или DYN(PIE)" 0 grep -qE "Type:.*(EXEC|DYN)" hdr.txt
chk "hdr: НЕ только-EXEC (ловушка раунда 21)" 1 grep -q 'Type:\.\*EXEC"' hdr.txt

# 2) file: исполняемый, не shared object, Android-интерпретатор
chk "file: pie executable"  0 grep -qiE "pie executable|executable" android_file.txt
chk "file: не shared object" 1 grep -q "shared object" android_file.txt
chk "file: interpreter /system/bin/linker64" 0 grep -q "interpreter /system/bin/linker64" android_file.txt
chk "file: for Android"     0 grep -q "for Android" android_file.txt

# 3) NEEDED: только bionic
chk "dyn: NEEDED не пуст"   0 grep -q "NEEDED" dyn.txt
if grep "NEEDED" dyn.txt | grep -vqE "libc\.so|libandroid\.so|liblog\.so|libm\.so|libdl\.so"; then
  echo "  ПЛОХО dyn: есть не-Android зависимость в NEEDED:"
  grep "NEEDED" dyn.txt | sed 's/^/      /'
  fails=$((fails+1))
else
  echo "  OK   dyn: все NEEDED — bionic"
fi

# 4) Ловушка раунда 22: интерпретатор НЕ ищется в readelf -d
if grep -q "ld-android" dyn.txt; then
  echo "  справка: ld-android действительно в dyn.txt (значит проверка законна)"
else
  echo "  OK   dyn: ld-android там НЕТ — искать интерпретатор обязана в выводе file / readelf -l"
fi
if [ -f interp.txt ]; then
  chk "interp (readelf -l): PT_INTERP linker64" 0 grep -q "system/bin/linker64" interp.txt
fi

echo "=== итог ==="
if [ "$fails" -gt 0 ]; then
  echo "ПЛОХО: $fails предикатов не проходят против реального снапшота — CI упадёт"
  exit 1
fi
echo "OK: все предикаты проходят против снапшота артефакта"
