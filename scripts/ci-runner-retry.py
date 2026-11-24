#!/usr/bin/env python3
"""Retry main CI test failures once and retry hosted runner acquisition."""
import html
import json
import os
import re
import subprocess
import sys
import time

MESSAGE = "The job was not acquired by Runner of type hosted even after multiple attempts"
# Count the original attempt and any manual attempts against this ceiling.
MAX_ATTEMPTS = 8
SUBMISSION_TRIES = 3
RECONCILE_DELAY = 10
REQUEST_TIMEOUT = 20


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
    return [job for job in jobs if job["conclusion"] == "failure"]


def unassigned(job):
    return (job["conclusion"] == "cancelled" and not job["steps"]
            and not job.get("runner_name"))


def retry_job(event, api):
    run = event["workflow_run"]
    if (run["head_branch"] != "main" or run["event"] != "push"
            or run["conclusion"] not in ("failure", "cancelled")
            or run["run_attempt"] >= MAX_ATTEMPTS):
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
                return job["id"]
    return None


def rerun_started(current, attempt):
    return (current["run_attempt"] > attempt
            or (current["run_attempt"] == attempt and current["status"] != "completed"))


def submit_rerun(attempt, current_run, post):
    # A timeout or error can hide an accepted POST. Reconcile with the live CI
    # attempt before every submission. Controllers for this attempt run serially.
    accepted = False
    for check in range(SUBMISSION_TRIES + 1):
        if check:
            time.sleep(RECONCILE_DELAY)
        try:
            current = current_run()
        except (subprocess.SubprocessError, ValueError) as error:
            print(f"Could not read the CI attempt: {error}", flush=True)
            continue
        if rerun_started(current, attempt):
            print("A newer CI attempt has started", flush=True)
            return
        if current["run_attempt"] != attempt or accepted or check == SUBMISSION_TRIES:
            continue
        try:
            post()
            accepted = True
        except subprocess.SubprocessError as error:
            print(f"Rerun submission {check + 1} returned an error: {error}", flush=True)
    raise RuntimeError(f"Could not confirm a CI rerun after {SUBMISSION_TRIES} submission rounds")


def source_attempt(event, get, recover):
    source = event["workflow_run"]
    if not recover:
        return source["run_attempt"], source
    # The triggering controller names only the exact CI attempt it handled.
    match = re.fullmatch(r"Retry CI ([0-9]+) attempt ([0-9]+)", source["display_title"])
    if not match:
        raise ValueError("Unexpected CI retry controller title")
    return int(match[2]), get(f"actions/runs/{match[1]}")


def main():
    repo = os.environ["GITHUB_REPOSITORY"]
    with open(os.environ["GITHUB_EVENT_PATH"], encoding="utf-8") as f:
        event = json.load(f)

    def gh(path, *flags, **kwargs):
        return subprocess.check_output(["gh", "api", *flags, f"repos/{repo}/{path}"],
                                       timeout=REQUEST_TIMEOUT, **kwargs)

    def get(path):
        return json.loads(gh(path))

    def api(path):
        pages = json.loads(gh(path, "--paginate", "--slurp"))
        if path.startswith("check-runs/"):
            return [item for page in pages for item in page]
        return {"jobs": [job for page in pages for job in page["jobs"]]}

    attempt, run = source_attempt(event, get, "--recover" in sys.argv[1:])
    if run["run_attempt"] < attempt:
        raise RuntimeError("GitHub has not published the completed CI attempt yet")
    if rerun_started(run, attempt):
        print("A newer attempt has already been requested")
        return
    if (run["name"], run["event"], run["head_branch"], run["conclusion"]) not in (
            ("CI", "push", "main", "failure"), ("CI", "push", "main", "cancelled")):
        return
    if attempt >= MAX_ATTEMPTS:
        print(f"Automatic runner acquisition retries stop at attempt {MAX_ATTEMPTS}")
        return

    def current_run():
        return get(f"actions/runs/{run['id']}")

    # Completion events can wait in the concurrency queue past a manual rerun.
    current = current_run()
    if rerun_started(current, attempt):
        print("A newer attempt has already been requested")
        return
    if current["run_attempt"] != attempt:
        raise RuntimeError("GitHub has not published the completed CI attempt yet")

    source_event = {"workflow_run": run}
    failures = failed_retry(source_event, api)
    if failures:
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
            print("## Automatic retry of failed CI jobs\n", file=summary)
            for job in failures:
                print(f"- **{html.escape(job['name'])}**", file=summary)
                try:
                    tests = failed_tests(gh(f"actions/jobs/{job['id']}/logs",
                                            stderr=subprocess.DEVNULL, text=True))
                except subprocess.SubprocessError:
                    tests = []
                if not tests:
                    print("  - No failed test name found in log (or log unavailable).", file=summary)
                for test in tests:
                    print(f"  - {html.escape(test)}", file=summary)
        path = f"actions/runs/{run['id']}/rerun-failed-jobs"
    else:
        job = retry_job(source_event, api)
        if job is None:
            print("No eligible runner acquisition failure")
            return
        path = f"actions/jobs/{job}/rerun"
    submit_rerun(attempt, current_run, lambda: gh(path, "--method", "POST"))


if __name__ == "__main__":
    main()
