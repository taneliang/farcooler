#!/usr/bin/env python3
"""Capture completed branch-run evidence without inferring success from queueing."""
import datetime
import json
from pathlib import Path
import subprocess
import sys

run_id = sys.argv[1]
if not run_id.isdigit():
    raise SystemExit("expected a numeric GitHub run ID")
here = Path(__file__).resolve().parent
destination = here / ("ci-" + run_id)
destination.mkdir(exist_ok=True)
data = json.loads(subprocess.check_output([
    "gh", "run", "view", run_id, "--json",
    "status,conclusion,jobs,createdAt,updatedAt,url,headSha,event,headBranch"], text=True))
if data["status"] != "completed":
    raise SystemExit("CI is still " + data["status"] + "; no completed result to report")
(destination / "run.json").write_text(json.dumps(data, indent=2) + "\n")
with (destination / "run.log").open("w") as log:
    subprocess.run(["gh", "run", "view", run_id, "--log"], stdout=log, check=True)
subprocess.run(["gh", "run", "download", run_id, "--name", "ov-1186-timings",
                "--dir", str(destination)], check=True)


def date(value):
    return datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))


job = data["jobs"][0]
start = min(date(s["startedAt"]) for s in job["steps"] if not s["startedAt"].startswith("0001"))
summary = {"url": data["url"], "conclusion": data["conclusion"], "head_sha": data["headSha"],
           "queue_seconds": (start - date(data["createdAt"])).total_seconds(),
           "job_seconds": (date(job["completedAt"]) - start).total_seconds(),
           "push_to_verdict_seconds": (date(job["completedAt"]) - date(data["createdAt"])).total_seconds()}
if data["conclusion"] == "success":
    reuse = json.loads((destination / "consume.json").read_text())
    cold = json.loads((destination / "cold.json").read_text())
    summary.update({"reuse_seconds": reuse["total_seconds"], "cold_seconds": cold["seconds"],
                    "ci_step_seconds_saved": round(cold["seconds"] - reuse["total_seconds"], 3)})
(destination / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps(summary, indent=2))
if data["conclusion"] != "success":
    raise SystemExit("branch experiment failed; inspect the captured log")
