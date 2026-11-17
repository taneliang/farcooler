#!/usr/bin/env python3
"""Retry main CI failures and hosted runner acquisition failures once."""
import hashlib
import html
import json
import os
import re
import subprocess

MESSAGE = "The job was not acquired by Runner of type hosted even after multiple attempts"
MAC_JOB = re.compile(r"Rust \(macos-latest\)|Rust native test archive \(macOS\)|Rust native tests \(macOS [12]/2\)|Swift \(.*\)|iOS|iOS UI \(smoke\)")


def marker(run_id, name):
    digest = hashlib.sha256(name.encode()).hexdigest()
    return f"runner-retry-{run_id}-{digest}"


def failed_tests(log):
    # nextest emits ANSI-colored summary lines; libtest emits "test ... FAILED".
    log = re.sub(r"\x1b\[[0-9;]*m", "", log)
    names = set()
    for line in log.splitlines():
        match = re.search(r"\bFAIL\s+\[\s*[^]]+\]\s+\(\d+/\d+\)\s+(\S+)\s+(\S+)", line)
        if match:
            names.add(f"{match[1]} {match[2]}")
        else:
            match = re.search(r"\btest (\S+) \.\.\. FAILED\b", line)
            if match:
                names.add(match[1])
    return sorted(names)


def failed_retry(event, api):
    run = event["workflow_run"]
    if (run["head_branch"] != "main" or run["event"] != "push"
            or run["conclusion"] != "failure" or run["run_attempt"] != 1):
        return None
    jobs = api(f"actions/runs/{run['id']}/attempts/1/jobs?per_page=100")["jobs"]
    failed = [job for job in jobs if job["conclusion"] == "failure"]
    if not failed:
        return None
    name = f"failed-retry-{run['id']}"
    artifacts = api(f"actions/artifacts?name={name}&per_page=100")
    if any(a["name"] == name and not a["expired"] for a in artifacts["artifacts"]):
        return None
    return failed, name


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

    failure = failed_retry(event, api)
    if failure:
        jobs, name = failure
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
            print("## Automatic retry of failed CI jobs\n", file=summary)
            for job in jobs:
                print(f"- **{html.escape(job['name'])}**", file=summary)
                try:
                    log = subprocess.check_output(
                        ["gh", "api", f"repos/{repo}/actions/jobs/{job['id']}/logs"],
                        stderr=subprocess.DEVNULL, text=True)
                    tests = failed_tests(log)
                except subprocess.CalledProcessError:
                    tests = []
                if not tests:
                    print("  - No failed test name found in log (or log unavailable).", file=summary)
                for test in tests:
                    print(f"  - {html.escape(test)}", file=summary)
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as out:
            print(f"failed_run={event['workflow_run']['id']}\nmarker={name}", file=out)
        with open("retry-marker.txt", "w", encoding="utf-8") as out:
            print("Automatic failed-jobs retry reserved", file=out)
        return

    # Do not fall through to cancelled siblings when a failure is present.
    run = event["workflow_run"]
    if run["conclusion"] == "failure":
        jobs = api(f"actions/runs/{run['id']}/attempts/{run['run_attempt']}/jobs?per_page=100")
        if any(job["conclusion"] == "failure" for job in jobs["jobs"]):
            print("Failed jobs already reserved or past first attempt")
            return
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
