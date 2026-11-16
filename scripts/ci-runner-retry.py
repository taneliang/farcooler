#!/usr/bin/env python3
"""Retry a main CI job once when GitHub could not acquire a hosted runner (ov-1310)."""
import json
import os
import re
import subprocess
import sys

MESSAGE = "The job was not acquired by Runner of type hosted even after multiple attempts"


def retry_job(event, api):
    run = event["workflow_run"]
    if (run["head_branch"] != "main" or run["event"] != "push"
            or run["conclusion"] not in ("failure", "cancelled")):
        return None
    jobs = api(f"actions/runs/{run['id']}/attempts/1/jobs?per_page=100")
    # A job can be rerun only after the preceding attempt completes. Record
    # every job present in later attempts, including jobs rerun as dependents.
    attempted = set()
    for attempt in range(2, run["run_attempt"] + 1):
        later = api(f"actions/runs/{run['id']}/attempts/{attempt}/jobs?per_page=100")
        attempted.update(job["name"] for job in later["jobs"])
    for job in jobs["jobs"]:
        if job["name"] in attempted:
            continue
        if not re.fullmatch(r"Rust \(macos-latest\)|Rust native test archive \(macOS\)|Rust native tests \(macOS [12]/2\)|Swift \(.*\)|iOS|iOS UI \(smoke\)", job["name"]):
            continue
        if job["conclusion"] != "cancelled" or job["steps"] or job.get("runner_name"):
            continue
        annotations = api(f"check-runs/{job['id']}/annotations?per_page=100")
        if any(a["message"] == MESSAGE for a in annotations):
            return job["id"]
    return None


def process(event, api, rerun):
    job = retry_job(event, api)
    if job is None:
        print("No eligible runner acquisition failure")
        return
    print(f"Retrying runner acquisition failure in job {job}", flush=True)
    rerun(job)


def main():
    repo = os.environ["GITHUB_REPOSITORY"]
    with open(os.environ["GITHUB_EVENT_PATH"], encoding="utf-8") as f:
        event = json.load(f)

    def api(path):
        pages = json.loads(subprocess.check_output(["gh", "api", "--paginate", "--slurp", f"repos/{repo}/{path}"]))
        if path.startswith("check-runs/"):
            return [item for page in pages for item in page]
        return {"jobs": [job for page in pages for job in page["jobs"]]}

    def rerun(job):
        subprocess.run(["gh", "api", "--method", "POST", f"repos/{repo}/actions/jobs/{job}/rerun"], check=True)

    process(event, api, rerun)


if __name__ == "__main__":
    sys.exit(main())
