#!/usr/bin/env python3
"""Забрать логи CI-прогона наружу (артефакты host-logs/android-logs).

Зачем скрипт, а not shell: раунд 10 потерял секцию логов, потому что в POSIX-sh
heredoc `python3 - <<'PY'` внутри for-цикла со вложенным `$(curl …)` съел часть
вывода. Python здесь читаемый и проверяемый, а shell-обёртка — только вызов.
"""
import io
import json
import os
import re
import subprocess
import sys
import urllib.request
import zipfile

R = "/var/minis/shared/rustspike"
API = "https://api.github.com"
REPO = "MikasaAckerrman/tg-purge-rust"
TOKEN = os.environ.get("GITHUB_PAT", "")
HDR = {"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"}
ANSI = re.compile(rb"\x1b\[[0-9;]*m")


def api(path):
    req = urllib.request.Request(API + path, headers=HDR)
    with urllib.request.urlopen(req, timeout=40) as r:
        return json.load(r)


def strip(p):
    raw = open(p, "rb").read()
    clean = ANSI.sub(b"", raw).decode("utf-8", "replace")
    out = p + ".clean"
    with open(out, "w", encoding="utf-8") as f:
        f.write(clean)
    return clean


def main():
    run_id = sys.argv[1] if len(sys.argv) > 1 else None
    if not run_id:
        runs = api(f"/repos/{REPO}/actions/runs?per_page=1")
        run_id = str(runs["workflow_runs"][0]["id"])
    print(f"run_id={run_id}")
    run = api(f"/repos/{REPO}/actions/runs/{run_id}")
    print(f"state: {run.get('status')} / {run.get('conclusion')}")

    jobs = api(f"/repos/{REPO}/actions/runs/{run_id}/jobs")
    for j in jobs.get("jobs", []):
        print(f"\nJOB {j['name']}: {j['status']} / {j.get('conclusion')}")
        for s in j.get("steps", []):
            c = s.get("conclusion") or "running"
            mark = {"success": "OK  ", "failure": "FAIL", "skipped": "skip "}.get(c, "run ")
            print(f"   {mark}{s['number']:>2} {s['name']}")

    arts = api(f"/repos/{REPO}/actions/runs/{run_id}/artifacts")
    print(f"\nартефакты: {arts.get('total_count')}")
    dest = f"{R}/logs_pull"
    subprocess.run(["rm", "-rf", dest], check=False)
    os.makedirs(dest, exist_ok=True)
    for a in arts.get("artifacts", []):
        name = a["name"]
        zpath = f"{dest}/{name}.zip"
        # curl, не urllib: скачивание артефакта редиректит на подписанный URL,
        # и urllib там ловит 401 («Server failed to authenticate»), а curl -L
        # проходит (проверено в раундах 6-8).
        rc = subprocess.call([
            "curl", "-sL", "-m", "120", "-H", f"Authorization: Bearer {TOKEN}",
            a["archive_download_url"], "-o", zpath,
        ])
        if rc != 0 or not os.path.exists(zpath) or os.path.getsize(zpath) == 0:
            print(f"  {name}: НЕ СКАЧАЛСЯ (curl rc={rc})")
            continue
        with zipfile.ZipFile(zpath) as z:
            z.extractall(f"{dest}/{name}")
        print(f"  {name}: {a['size_in_bytes']} Б → {dest}/{name}")
        for root, _, files in os.walk(f"{dest}/{name}"):
            for fn in files:
                if not fn.endswith(".log"):
                    continue
                p = os.path.join(root, fn)
                clean = strip(p)
                lines = clean.splitlines()
                errs = [i for i, l in enumerate(lines) if l.startswith("error")]
                print(f"\n--- {fn}: {len(lines)} строк, {len(errs)} ошибок")
                shown = 0
                for i in errs:
                    for k in range(i, min(i + 7, len(lines))):
                        print("   ", lines[k][:175])
                    print("   ---")
                    shown += 1
                    if shown >= 12:
                        break
                if not errs:
                    print("   (ошибок нет) последние 6 строк:")
                    for l in lines[-6:]:
                        print("   ", l[:175])


if __name__ == "__main__":
    main()
