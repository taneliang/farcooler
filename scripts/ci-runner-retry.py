#!/usr/bin/env python3
"""Retry main CI test failures once and retry hosted runner acquisition."""
import html
import json
import os
import re
import subprocess

MESSAGE = "The job was not acquired by Runner of type hosted even after multiple attempts"
# Count the original attempt and any manual attempts against this ceiling.
MAX_ATTEMPTS = 8


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


def reservation(run, api):
    name = f"runner-retry-{run['id']}-attempt-{run['run_attempt']}"
    artifacts = api(f"actions/artifacts?name={name}&per_page=100")
    if any(a["name"] == name and not a["expired"] for a in artifacts["artifacts"]):
        return None
    return name


def failed_retry(event, api):
    run = event["workflow_run"]
    if (run["head_branch"] != "main" or run["event"] != "push"
            or run["conclusion"] != "failure" or run["run_attempt"] != 1):
        return None
    jobs = api(f"actions/runs/{run['id']}/attempts/1/jobs?per_page=100")["jobs"]
    failed = [job for job in jobs if job["conclusion"] == "failure"]
    if not failed:
        return None
    name = reservation(run, api)
    return (failed, name) if name else None


def unassigned(job):
    return (job["conclusion"] == "cancelled" and not job["steps"]
            and not job.get("runner_name"))


def retry_job(event, api):
    run = event["workflow_run"]
    if (run["head_branch"] != "main" or run["event"] != "push"
            or run["conclusion"] not in ("failure", "cancelled")
            or run["run_attempt"] >= MAX_ATTEMPTS):
        return None
    name = reservation(run, api)
    if name is None:
        return None

    # A job-specific rerun can copy a sibling's cancellation to a new check ID
    # without copying its annotation. Correlate by name and original completion.
    history = {}
    for attempt in range(1, run["run_attempt"] + 1):
        jobs = api(f"actions/runs/{run['id']}/attempts/{attempt}/jobs?per_page=100")
        for job in jobs["jobs"]:
            history.setdefault(job["name"], {})[job["id"]] = job
    candidates = []
    for instances in history.values():
        job = next(reversed(instances.values()))
        if unassigned(job):
            candidates.append((job, instances))
    # Give older cancellations the first chance at the remaining attempts.
    for job, instances in sorted(candidates, key=lambda pair: pair[0].get("completed_at") or ""):
        completed = job.get("completed_at")
        for previous in reversed(instances.values()):
            if previous["id"] != job["id"] and (
                    not completed or previous.get("completed_at") != completed or not unassigned(previous)):
                continue
            annotations = api(f"check-runs/{previous['id']}/annotations?per_page=100")
            if any(a["message"] == MESSAGE for a in annotations):
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

    run = event["workflow_run"]
    if (run["event"], run["head_branch"], run["conclusion"]) not in (
            ("push", "main", "failure"), ("push", "main", "cancelled")):
        return
    # Completion events can wait in the concurrency queue past a manual rerun.
    current = json.loads(subprocess.check_output(
        ["gh", "api", f"repos/{repo}/actions/runs/{run['id']}"]))
    if current["run_attempt"] != run["run_attempt"] or current["status"] != "completed":
        print("A newer attempt has already been requested")
        return

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

    selected = retry_job(event, api)
    if selected is None:
        if run["run_attempt"] >= MAX_ATTEMPTS:
            print(f"Automatic runner acquisition retries stop at attempt {MAX_ATTEMPTS}")
        else:
            print("No eligible runner acquisition failure")
        return
    job, name = selected
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as out:
        print(f"job={job}\nmarker={name}", file=out)
    with open("retry-marker.txt", "w", encoding="utf-8") as out:
        print(f"Automatic retry reserved for job {job}", file=out)


if __name__ == "__main__":
    main()
