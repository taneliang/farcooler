#!/usr/bin/env python3
"""No CI job runs past 85% of its timeout-minutes (ov-1056).

A job that hits its limit is cancelled with no verdict, so main goes red
without a failing test: the Mac job at 35 min (ov-1052), then Android at 25
(train 385, run 42688011276). Each was creeping up for days. This reads the
last five green runs of ci.yml on main and fails when any job's slowest run is
past 85% of the limit ci.yml gives it, so the limit is raised (or the job
trimmed) while it still passes.

  - A job's time is start to finish, setup included: what the timeout measures.
  - The figure checked is the max of the five. The p50 is printed beside it.
  - Skipped jobs and jobs without a timeout are ignored.
  - The limit comes from ci.yml. `timeout-minutes: ${{ ... && A || B }}` (the
    Rust matrix) reads as A for a macOS job and B otherwise.

  ./scripts/ci-job-time-check.py                      the table, then the verdict
  ./scripts/ci-job-time-check.py --limit-scale 0.5    pretend every limit is half
                                                      (proves the check fires)
  ./scripts/ci-job-time-check.py --self-test          the rule against planted runs

Needs `gh` with a token that can read Actions.
"""

import json
import pathlib
import re
import statistics
import subprocess
import sys
from datetime import datetime

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/ci.yml"
THRESHOLD = 0.85


def limits(text):
    """{job name: minutes} from ci.yml. A matrix name stays a template."""
    out = {}
    for block in re.split(r"\n(?=  [a-z0-9-]+:\n)", text):
        name = re.search(r"^    name: (.+)$", block, re.M)
        limit = re.search(r"^    timeout-minutes: (.+)$", block, re.M)
        if name and limit:
            out[name.group(1).strip()] = limit.group(1).strip()
    return out


def limit_for(job_name, limit_map):
    for template, raw in limit_map.items():
        if "${{" in template:
            os_part = re.search(r"\$\{\{ matrix\.os \}\}", template)
            pattern = re.escape(template).replace(re.escape("${{ matrix.os }}"), r"(.+)")
            m = re.fullmatch(pattern, job_name) if os_part else None
            if not m:
                continue
            os_name = m.group(1)
        elif template == job_name:
            os_name = ""
        else:
            continue
        if raw.isdigit():
            return int(raw)
        nums = re.search(r"&& (\d+) \|\| (\d+)", raw)
        if nums:
            return int(nums.group(1) if "macos" in os_name else nums.group(2))
    return None


def minutes(job):
    if job.get("conclusion") != "success" or not job.get("completedAt"):
        return None
    t = lambda s: datetime.fromisoformat(s.replace("Z", "+00:00"))
    return (t(job["completedAt"]) - t(job["startedAt"])).total_seconds() / 60


def evaluate(runs, limit_map, scale=1.0):
    """runs: a list of job lists. Returns (rows, names over the threshold)."""
    times = {}
    for jobs in runs:
        for job in jobs:
            m = minutes(job)
            if m is not None:
                times.setdefault(job["name"], []).append(m)
    rows, over = [], []
    for name in sorted(times):
        limit = limit_for(name, limit_map)
        if limit is None:
            continue
        limit *= scale
        ts = times[name]
        share = max(ts) / limit
        rows.append((name, limit, statistics.median(ts), max(ts), share, len(ts)))
        if share > THRESHOLD:
            over.append(name)
    return rows, over


def report(rows, over):
    print(f"{'job':<36}{'limit':>7}{'p50':>7}{'max':>7}{'max/limit':>11}{'runs':>6}")
    for name, limit, p50, mx, share, n in rows:
        flag = "  OVER 85%" if name in over else ""
        print(f"{name:<36}{limit:>7.1f}{p50:>7.1f}{mx:>7.1f}{share:>10.0%}{n:>6}{flag}")


def fetch(count):
    def gh(*a):
        return json.loads(subprocess.check_output(["gh", *a], text=True))
    ids = gh("run", "list", "--workflow", "ci.yml", "--branch", "main",
             "--status", "success", "--limit", str(count), "--json", "databaseId")
    return [gh("run", "view", str(i["databaseId"]), "--json", "jobs")["jobs"] for i in ids]


def job(name, start, end, conclusion="success"):
    return {"name": name, "conclusion": conclusion,
            "startedAt": f"2026-11-09T{start}:00Z", "completedAt": f"2026-11-09T{end}:00Z"}


def self_test():
    lm = limits("""
  android:
    name: Android
    timeout-minutes: 25
  rust:
    name: Rust (${{ matrix.os }})
    timeout-minutes: ${{ matrix.os == 'macos-latest' && 60 || 35 }}
""")
    assert limit_for("Android", lm) == 25 and limit_for("Rust (macos-latest)", lm) == 60
    assert limit_for("Rust (ubuntu-24.04)", lm) == 35 and limit_for("Nope", lm) is None
    runs = [[job("Android", "10:00", "10:20"), job("Rust (macos-latest)", "10:00", "10:40"),
             job("Rust (ubuntu-24.04)", "10:00", "10:20", "skipped")]]
    rows, over = evaluate(runs, lm)
    assert over == [], over                      # 80% and 67%; the skipped job is ignored
    runs.append([job("Android", "10:00", "10:22")])
    assert evaluate(runs, lm)[1] == ["Android"]  # 22/25 = 88%
    assert evaluate(runs, lm, scale=0.5)[1] == ["Android", "Rust (macos-latest)"]
    print("ci-job-time-check: self-test passed")


def main():
    args = sys.argv[1:]
    if "--self-test" in args:
        return self_test()
    scale = float(args[args.index("--limit-scale") + 1]) if "--limit-scale" in args else 1.0
    rows, over = evaluate(fetch(5), limits(WORKFLOW.read_text()), scale)
    report(rows, over)
    if over:
        sys.exit("\nPast 85% of the limit: " + ", ".join(over)
                 + ". Raise timeout-minutes in ci.yml (aim for 25% headroom over the max) or trim the job.")
    print("\nEvery job is within 85% of its limit.")


if __name__ == "__main__":
    main()
