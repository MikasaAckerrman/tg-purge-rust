import sys, yaml, json, urllib.request, os

P = "/var/minis/shared/rustspike/.github/workflows/rust.yml"
raw = open(P, "rb").read()
print("bytes:", len(raw), "| tabs:", raw.count(b"\t"), "| CR:", raw.count(b"\r"))
try:
    d = yaml.safe_load(raw.decode())
except Exception as e:
    print("YAML ОШИБКА:", e)
    sys.exit(1)
print("ключи верха:", list(d.keys()))
jobs = d.get("jobs", {})
print("jobs:", list(jobs.keys()))
for name, j in jobs.items():
    steps = j.get("steps", [])
    print(f"  {name}: runs-on={j.get('runs-on')!r} steps={len(steps)}")
    for s in steps:
        print("     -", (s.get("name") or s.get("uses") or "?")[:60])

tok = os.environ.get("GITHUB_PAT")
def api(path):
    r = urllib.request.Request("https://api.github.com" + path, headers={
        "Authorization": "Bearer " + tok, "Accept": "application/vnd.github+json"})
    with urllib.request.urlopen(r, timeout=30) as x:
        return json.load(x)

R = "repos/MikasaAckerrman/tg-purge-rust"
try:
    ws = api(f"/{R}/actions/workflows")
    print("workflows total:", ws.get("total_count"))
    for w in ws.get("workflows", []):
        print("   ", w["id"], w["name"], w["state"], w["path"], "| badge:", (w.get("badge_url") or "")[-30:])
except Exception as e:
    print("workflows err:", e)
try:
    runs = api(f"/{R}/actions/runs?per_page=3")
    print("runs total:", runs.get("total_count"))
    for r in runs.get("workflow_runs", []):
        print("   ", r["id"], r["name"], r["status"], r.get("conclusion"),
              "jobs_url_ok" if r.get("run_attempts") else "", "| attempt", r.get("run_attempt"),
              "| created", r["created_at"])
except Exception as e:
    print("runs err:", e)
