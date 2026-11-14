#!/usr/bin/env python3
"""Measure three real landing wrappers, including admission and build contention.

Run with --out outside the checkout so evidence cannot change the gate's diff.
No sleep-based replay, partial-job option, or admission bypass is accepted.
Failed or interrupted trials remain evidence but cannot establish acceptance.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parent.parent


def interrupt(_signal, _frame):
    """Let normal cleanup end only the benchmark's own process group."""
    raise KeyboardInterrupt


def steps(text):
    """Read the printed table, never a potentially stale worktree summary file."""
    rows = []
    for line in text.splitlines():
        cells = [c.strip().replace(r"\|", "|") for c in re.split(r"(?<!\\)\|", line)[1:-1]]
        if len(cells) == 5 and cells[0] not in {"Job", "---"}:
            try:
                seconds = float(cells[3]) if cells[3] else None
            except ValueError:
                continue
            rows.append(dict(zip(("job", "command", "result", "seconds", "tests"),
                                 [*cells[:3], seconds, cells[4]])))
    return rows


def identity(repo):
    def git(*args):
        # Observation must not invalidate the warm build it measures.
        return subprocess.check_output(["git", "-c", "diff.autoRefreshIndex=false", *args], cwd=repo,
                                       env={**os.environ, "GIT_OPTIONAL_LOCKS": "0"})
    # Include untracked source too: HEAD alone doesn't identify an agent's build.
    digest = hashlib.sha256(git("diff", "--binary", "HEAD"))
    for name in sorted(git("ls-files", "--others", "--exclude-standard", "-z").split(b"\0")):
        if name:
            digest.update(name + b"\0")
            digest.update((repo / os.fsdecode(name)).read_bytes())
    return {"head": git("rev-parse", "HEAD").decode().strip(),
            "dirty_sha256": digest.hexdigest()}


def measure(command, repo, dest, deadline):
    """Run real subprocesses; timeout terminates only this trial's process group."""
    dest.mkdir()
    started = time.monotonic()
    samples = []
    timed_out = False
    interrupted = False
    with (dest / "gate.log").open("w") as log:
        child = subprocess.Popen(command, cwd=repo, stdout=log, stderr=subprocess.STDOUT,
                                 start_new_session=True)
        try:
            while True:
                samples.append({"seconds": time.monotonic() - started, "load": os.getloadavg()})
                try:
                    child.wait(timeout=min(5, max(0.01, deadline - (time.monotonic() - started))))
                    break
                except subprocess.TimeoutExpired:
                    if time.monotonic() - started >= deadline:
                        timed_out = True
                        break
        except KeyboardInterrupt:
            interrupted = True
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait()
    wall = time.monotonic() - started
    text = (dest / "gate.log").read_text(errors="replace")
    measured_steps = []
    for path in sorted((dest / "steps").glob("*-timings.jsonl")):
        for line in path.read_text().splitlines():
            try:
                measured_steps.append(json.loads(line))
            except json.JSONDecodeError:
                pass  # a killed writer may leave its last line incomplete
    measured_steps.sort(key=lambda r: r["start_monotonic"])
    result = {"command": command, "wall_seconds": wall, "exit_code": child.returncode,
              "timed_out": timed_out, "interrupted": interrupted,
              "load_samples": samples, "steps": measured_steps or steps(text),
              "complete_green": (not timed_out and not interrupted and child.returncode == 0
                                 and "lane-gate: all steps passed" in text
                                 and "landing-gate: PASS" in text and "PARTIAL" not in text)}
    (dest / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def summarize(trials, stable):
    median = statistics.median(t["wall_seconds"] for t in trials)
    return {"median_wall_seconds": median,
            "acceptance_1": (len(trials) == 3 and stable and median < 900
                             and all(t["complete_green"] for t in trials))}


def report(trials, before):
    def indexed(rows):
        occurrences = {}
        result = {}
        for row in rows:
            job = "rust" if row["job"] == "rust-lints" else row["job"]
            command = row["command"]
            if "cargo nextest run --profile ci --workspace" in command:
                command = "default-host Rust tests (including historical shadow, when present)"
            key = (job, command)
            occurrence = occurrences.get(key, 0)
            occurrences[key] = occurrence + 1
            result[(*key, occurrence)] = row
        return result
    baseline = indexed(before)
    maps = [indexed(t["steps"]) for t in trials]
    keys = list(dict.fromkeys(k for m in [baseline, *maps] for k in m))
    lines = ["| Job | Command | Historical seconds | Real 1 | Real 2 | Real 3 |",
             "| --- | --- | ---: | ---: | ---: | ---: |"]
    for key in keys:
        first = next(mapping[key] for mapping in [*maps, baseline] if key in mapping)
        job, command = first["job"], first["command"]
        values = []
        for number in range(3):
            if number >= len(maps):
                values.append("not run")
                continue
            mapping = maps[number]
            row = mapping.get(key)
            values.append("not reached" if row is None else
                          f'{row["seconds"]} ({row["result"]})' if row["seconds"] is not None else row["result"])
        previous = baseline.get(key)
        cells = [job, command, str(previous["seconds"] if previous else "not in capture"), *values]
        lines.append("| " + " | ".join(c.replace("|", r"\|") for c in cells) + " |")
    return "\n".join(lines) + "\n"


def main():
    signal.signal(signal.SIGTERM, interrupt)
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--base", default="origin/main")
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--deadline-seconds", type=float, default=7200,
                    help="per-trial observation limit; a timeout never counts as green")
    args = ap.parse_args()
    dest = args.out.resolve()
    if dest == ROOT or ROOT in dest.parents or dest.exists() or args.deadline_seconds <= 0:
        ap.error("use a new output directory outside the checkout and a positive deadline")
    # These variables bypass normal admission, invalidate selection evidence,
    # or replace lock-state reads. Refuse them rather than report a fast gate.
    bypasses = ("CI", "HEAVY_LOCK_HELD", "HEAVY_LOAD_LIMIT", "FARCOOLER_LOCK_STATE_CMD",
                "FARCOOLER_LOCK_WAIT_LIMIT_SECONDS", "CANDIDATES")
    if any(os.environ.get(k) for k in bypasses):
        ap.error("unset admission/selection overrides: " + ", ".join(k for k in bypasses if os.environ.get(k)))
    dest.mkdir(parents=True)
    initial = identity(ROOT)
    trials = []
    stable = True
    for number in range(1, 4):
        trial = dest / f"trial-{number}"
        command = ["bash", "scripts/landing-gate", "--base", args.base,
                   "--log-dir", str(trial / "steps")]
        print(f"trial {number}: real landing gate; logs {trial}", flush=True)
        result = measure(command, ROOT, trial, args.deadline_seconds)
        stable = stable and identity(ROOT) == initial
        trials.append(result)
        output = {"kind": "real landing wrappers", "started_from": initial,
                  "base": args.base, "source_stable": stable,
                  "recorded_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  **summarize(trials, stable), "trials": trials}
        (dest / "benchmark.json").write_text(json.dumps(output, indent=2) + "\n")
        before = steps((ROOT / "reports/ov-1110/historical-gate.txt").read_text())
        (dest / "step-timings.md").write_text(report(trials, before))
        print(f'trial {number}: {result["wall_seconds"]:.2f}s, green={result["complete_green"]}', flush=True)
        if result["interrupted"]:
            return 130
    print(json.dumps(summarize(trials, stable)), flush=True)
    return 0 if summarize(trials, stable)["acceptance_1"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
