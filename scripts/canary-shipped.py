#!/usr/bin/env python3
"""Did a Canary run ship anything? Read from its jobs, after it ended.

canary-baseline.yml runs when a Canary run completes, whatever its conclusion,
and records proto/baseline/canary.proto from the run's commit if a client
reached the field. This decides that from the run's jobs, as the Actions API
lists them (`gh api --paginate .../runs/ID/jobs?filter=all --jq '.jobs[]'`,
one JSON object per line, every attempt included):

    scripts/canary-shipped.py JOBS.ndjson
    scripts/canary-shipped.py --commit TITLE
    scripts/canary-shipped.py --self-test

It prints why, and writes `shipped=true` or `shipped=false` to $GITHUB_OUTPUT.

WHY NOT THE RUN'S CONCLUSION. Canary cancels its run when the next push lands,
so a run that already uploaded to TestFlight can end `cancelled`; and the iOS
job fails on App Store Connect's upload limit while the Mac one ships, so a
`failure` can have shipped too. The recording jobs used to live in canary.yml,
and that same cancel took them down with the run, after the ship.

WHAT COUNTS. A ship step that succeeded, in any attempt. And one that was
CANCELLED or FAILED once it had started, because an upload cut off part way
may still have landed, and altool can report an error for an upload App Store
Connect kept (the upload step says as much). The API cannot tell those from
the daily upload limit, which shipped nothing, so a failed upload counts as
shipped too: recording a proto that never shipped only makes the baseline
stricter, while missing one that did lets the next push break it unseen.
"Started" is every step before it ending in success or skipped, so a run
cancelled or failed during the build, which is most of them, records nothing.
"""

import importlib.util
import json
import os
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CANARY = ROOT / ".github" / "workflows" / "canary.yml"

# The ship job's display name (its `name:`, which is what the API reports) and
# the steps after which its build is in the field. The self-test checks each
# against canary.yml, so a rename there fails CI rather than recording nothing.
SHIP_STEPS = {
    "iOS (internal TestFlight)": ["Upload to TestFlight"],
    "macOS (signed disk image)": [
        "Sign the dmg, upload it, and publish canary's appcast",
        "Upload the disk image",
    ],
}


def shipped(jobs):
    """Why this run shipped, one sentence per ship; empty if it did not."""
    reasons = []
    for job in jobs:
        ships = SHIP_STEPS.get(job.get("name"))
        if not ships:
            continue
        where = f"{job['name']} (attempt {job.get('run_attempt', 1)})"
        steps = sorted(job.get("steps") or [], key=lambda s: s.get("number", 0))
        if not steps and job.get("conclusion") == "success":
            reasons.append(f"{where} succeeded")
            continue
        for i, step in enumerate(steps):
            if step.get("name") not in ships:
                continue
            started = all(s.get("conclusion") in ("success", "skipped") for s in steps[:i])
            if step.get("conclusion") == "success":
                reasons.append(f"{where}: {step['name']} succeeded")
            elif started and step.get("conclusion") in ("cancelled", "failure", None):
                reasons.append(f"{where}: {step['name']} ended {step.get('conclusion') or 'unfinished'} part way, so it may have landed")
    return reasons


def commit_of(title):
    """The commit a Canary run built, from its title, or None.

    canary.yml is a workflow_run workflow, so the run's own head_sha is main's
    head when it started, not the commit CI passed. Its `run-name` carries the
    built commit instead: "Canary <40 hex>".
    """
    found = re.fullmatch(r"Canary ([0-9a-f]{40})", (title or "").strip())
    return found.group(1) if found else None


def read_jobs(path):
    text = pathlib.Path(path).read_text()
    return [json.loads(line) for line in text.splitlines() if line.strip()]


def name_problems(text):
    """Each job and step SHIP_STEPS names, missing from canary.yml."""
    sys.dont_write_bytecode = True
    spec = importlib.util.spec_from_file_location("proto_lint", ROOT / "scripts" / "proto-lint.py")
    lint = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(lint)
    by_name = {keys.get("name", "").strip("'\""): keys for keys in lint.workflow_jobs(text).values()}
    problems = []
    for job, steps in SHIP_STEPS.items():
        if job not in by_name:
            problems.append(f"no job in canary.yml is named {job!r}")
            continue
        body = by_name[job].get("steps", "")
        for step in steps:
            if f"- name: {step}" not in body.splitlines():
                problems.append(f"canary.yml's {job!r} has no step named {step!r}")
    return problems


def self_test():
    def step(n, name, conclusion):
        return {"number": n, "name": name, "conclusion": conclusion}

    def ios(conclusion, archive, upload, attempt=1):
        return {"name": "iOS (internal TestFlight)", "conclusion": conclusion, "run_attempt": attempt,
                "steps": [step(1, "Set up job", "success"), step(2, "Archive and export", archive),
                          step(3, "Upload to TestFlight", upload), step(4, "Complete job", "success")]}

    def mac(conclusion, build, sparkle, artifact):
        return {"name": "macOS (signed disk image)", "conclusion": conclusion,
                "steps": [step(1, "Set up job", "success"), step(2, "Build Far Cooler.app", build),
                          step(3, "Sign the dmg, upload it, and publish canary's appcast", sparkle),
                          step(4, "Upload the disk image", artifact)]}

    linux = {"name": "linux / Linux binaries (x86_64)", "conclusion": "success",
             "steps": [step(1, "Upload to TestFlight", "success")]}
    cases = [
        ("both shipped", [ios("success", "success", "success"), mac("success", "success", "success", "success")], True),
        ("iOS uploaded, then the run was cancelled before the Mac one",
         [ios("success", "success", "success"), mac("cancelled", "cancelled", "skipped", "skipped")], True),
        ("cancelled during the upload", [ios("cancelled", "success", "cancelled")], True),
        ("uploaded, then cancelled before the job ended", [ios("cancelled", "success", "success")], True),
        ("Sparkle off, cancelled during the disk image upload", [mac("cancelled", "success", "skipped", "cancelled")], True),
        ("cancelled during the build, later steps skipped", [ios("cancelled", "cancelled", "skipped")], False),
        ("cancelled during the build, later steps cancelled", [ios("cancelled", "cancelled", "cancelled")], False),
        ("a failed upload, which may have landed",
         [ios("failure", "success", "failure"), mac("cancelled", "cancelled", "cancelled", "cancelled")], True),
        ("a failed archive, and the Mac job cancelled in its build",
         [ios("failure", "failure", "skipped"), mac("cancelled", "cancelled", "cancelled", "cancelled")], False),
        ("the upload limit, and the Mac job shipped",
         [ios("failure", "success", "failure"), mac("success", "success", "success", "success")], True),
        ("Sparkle off, the disk image uploaded", [mac("success", "success", "skipped", "success")], True),
        ("Sparkle published, then cancelled in the artifact upload",
         [mac("cancelled", "success", "success", "cancelled")], True),
        ("iOS job skipped for want of the key", [{"name": "iOS (internal TestFlight)", "conclusion": "skipped", "steps": []}], False),
        ("a re-run attempt shipped", [ios("failure", "failure", "skipped"), ios("success", "success", "success", 2)], True),
        ("a job not in SHIP_STEPS, whatever its steps", [linux], False),
        ("a successful ship job the API listed without steps",
         [{"name": "macOS (signed disk image)", "conclusion": "success", "steps": []}], True),
        ("no jobs at all", [], False),
    ]
    failures = []
    for what, jobs, want in cases:
        got = shipped(jobs)
        if bool(got) != want:
            failures.append(f"{what}: expected {'shipped' if want else 'nothing shipped'}, got {got}")

    canary = CANARY.read_text()
    failures += [f"names: {p}" for p in name_problems(canary)]
    renamed = canary.replace("- name: Upload to TestFlight\n", "- name: Upload\n", 1)
    if renamed == canary or not name_problems(renamed):
        failures.append("names: renaming the TestFlight upload step was not caught")

    sha = "0123456789abcdef0123456789abcdef01234567"
    for title, want in [(f"Canary {sha}", sha), (f"Canary {sha}\n", sha), ("Canary", None),
                        (f"Canary {sha[:39]}", None), (f"CI {sha}", None), (f"Canary {sha} extra", None),
                        ("", None), (None, None)]:
        if commit_of(title) != want:
            failures.append(f"commit_of({title!r}): got {commit_of(title)!r}, want {want!r}")
    run_name = re.search(r"^run-name:\s*(.*)$", canary, re.M)
    if not run_name or "workflow_run.head_sha" not in run_name.group(1) or not run_name.group(1).startswith("Canary "):
        failures.append("canary.yml's run-name no longer reads `Canary <workflow_run.head_sha>`, which commit_of parses")

    for f in failures:
        print(f"FAIL: {f}", file=sys.stderr)
    count = len(cases) + 2 + 9
    print(f"{count - len(failures)} passed, {len(failures)} failed")
    return 1 if failures else 0


def main():
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    if len(sys.argv) == 3 and sys.argv[1] == "--commit":
        sha = commit_of(sys.argv[2])
        if not sha:
            print(f"::error::the Canary run's title {sys.argv[2]!r} names no commit, so nothing is recorded", file=sys.stderr)
            return 1
        print(sha)
        return 0
    if len(sys.argv) != 2:
        print("usage: canary-shipped.py JOBS.ndjson | --commit TITLE | --self-test", file=sys.stderr)
        return 2
    reasons = shipped(read_jobs(sys.argv[1]))
    for r in reasons:
        print(r)
    if not reasons:
        print("nothing from this run reached the field, so the canary baseline stays where it is")
    out = pathlib.Path(os.environ.get("GITHUB_OUTPUT", "/dev/null"))
    with out.open("a") as f:
        f.write(f"shipped={'true' if reasons else 'false'}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
