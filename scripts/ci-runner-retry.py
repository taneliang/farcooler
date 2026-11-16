#!/usr/bin/env python3
"""Retry main CI's hosted runner acquisition failures once (ov-1310)."""
import hashlib
import json
import os
import re
import subprocess

MESSAGE = "The job was not acquired by Runner of type hosted even after multiple attempts"
MAC_JOB = re.compile(r"Rust \(macos-latest\)|Rust native test archive \(macOS\)|Rust native tests \(macOS [12]/2\)|Swift \(.*\)|iOS|iOS UI \(smoke\)")


def marker(run_id, name):
    digest = hashlib.sha256(name.encode()).hexdigest()
    return f"runner-retry-{run_id}-{digest}"


def retry_job(event, api):
    run = event["workflow_run"]
    if (run["head_branch"] != "main" or run["event"] != "push"
            or run["conclusion"] not in ("failure", "cancelled")):
        return None
    # Later attempts contain only rerun jobs. Keep the latest instance of each
    # job, including siblings left behind by a job-specific rerun.
    latest = {}
    for attempt in range(1, run["run_attempt"] + 1):
        jobs = api(f"actions/runs/{run['id']}/attempts/{attempt}/jobs?per_page=100")
        latest.update((job["name"], job) for job in jobs["jobs"])
    for job in latest.values():
        if not MAC_JOB.fullmatch(job["name"]):
            continue
        if job["conclusion"] != "cancelled" or job["steps"] or job.get("runner_name"):
            continue
        annotations = api(f"check-runs/{job['id']}/annotations?per_page=100")
        if not any(a["message"] == MESSAGE for a in annotations):
            continue
        name = marker(run["id"], job["name"])
        artifacts = api(f"actions/artifacts?name={name}&per_page=100")
        if not any(a["name"] == name and not a["expired"] for a in artifacts["artifacts"]):
            return job["id"], name
    return None


def main():
    repo = os.environ["GITHUB_REPOSITORY"]
    with open(os.environ["GITHUB_EVENT_PATH"], encoding="utf-8") as f:
        event = json.load(f)

    def api(path):
        pages = json.loads(subprocess.check_output(["gh", "api", "--paginate", "--slurp", f"repos/{repo}/{path}"]))
        if path.startswith("check-runs/"):
            return [item for page in pages for item in page]
        key = "artifacts" if path.startswith("actions/artifacts?") else "jobs"
        return {key: [item for page in pages for item in page[key]]}

    selected = retry_job(event, api)
    if selected is None:
        print("No eligible runner acquisition failure")
        return
    job, name = selected
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as out:
        print(f"job={job}\nmarker={name}", file=out)
    with open("retry-marker.txt", "w", encoding="utf-8") as out:
        print(f"Automatic retry reserved for job {job}", file=out)


if __name__ == "__main__":
    main()
