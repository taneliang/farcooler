#!/usr/bin/env python3
"""Replay captured gate step timings through the production landing scheduler.

Scale is explicit: this is a scheduling replay, not a fresh toolchain benchmark.
No selected-test or warm-cache saving is assumed; Rust remains full.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import statistics
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import landing_policy as policy
spec = importlib.util.spec_from_file_location("lane", ROOT / "scripts/lane-gate.py")
lane = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lane)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scale", type=float, default=0.01)
    args = parser.parse_args()
    if args.scale <= 0:
        parser.error("scale must be positive")
    source = ROOT / "reports/ov-1110/historical-gate.txt"
    plans = {}
    durations = {}
    before = []
    for line in source.read_text().splitlines():
        cells = [c.strip().replace('\\|', '|') for c in re.split(r'(?<!\\)\|', line)[1:-1]]
        if len(cells) != 5 or not cells[3].isdigit():
            continue
        job, command, result, seconds, counts = cells
        name = "Build Far Cooler.app" if command == "apps/macos/build-app.sh" else command
        if command.startswith('name="$(scripts/version.sh app-name)"'):
            name = "Resolve this channel's app name"
        step = lane.Step(job, name, command)
        plans.setdefault(job, []).append(step)
        durations[id(step)] = float(seconds)
        before.append({"job": job, "command": command, "seconds": float(seconds)})
    original = sum(row["seconds"] for row in before)
    # This snapshot predates shadow mode. The full 665 s default nextest step
    # is retained, so no invented selection or inventory timing enters replay.
    policy.apply(plans, {"full": True, "packages": []})
    trials = []
    for trial in range(3):
        rows = []
        events = []
        start = time.monotonic()
        def run_job(job, plans, own_rows, repo, tmp, *rest):
            for step in plans[job]:
                a = time.monotonic()
                if not step.skip:
                    time.sleep(durations[id(step)] * args.scale)
                b = time.monotonic()
                events.append({"job": job, "command": step.label(), "source_seconds": durations[id(step)],
                               "replay_seconds": 0 if step.skip else durations[id(step)],
                               "start": (a - start) / args.scale, "end": (b - start) / args.scale,
                               "skip": step.skip})
                own_rows.append((job, step.label(), "skipped" if step.skip else "pass", str(durations[id(step)]), ""))
            return False
        with tempfile.TemporaryDirectory() as t:
            failed = policy.run(list(plans), plans, rows, ROOT, Path(t), {}, Path(t),
                                argparse.Namespace(workers=4), run_job)
        wall = time.monotonic() - start
        trials.append({"trial": trial + 1, "before_seconds": original,
                       "replayed_wall_seconds": wall / args.scale, "actual_seconds": wall,
                       "failed": failed, "steps": sorted(events, key=lambda e: e["start"])})
    output = {"source": str(source.relative_to(ROOT)), "scale": args.scale,
              "limitations": "Historical scheduling replay; excludes queue/load wait, new CI steps, parallel CPU/memory and Cargo lock contention. Not three fresh real gates at current load.",
              "rust_selection": "full; no test-duration or warm-target saving assumed",
              "median_before_seconds": original,
              "median_replayed_seconds": statistics.median(t["replayed_wall_seconds"] for t in trials),
              "trials": trials}
    here = Path(__file__).parent
    (here / "replay.json").write_text(json.dumps(output, indent=2) + "\n")
    lines = ["| Job | Historical command | Before seconds | Landing seconds | Reason |",
             "| --- | --- | ---: | ---: | --- |"]
    for event in trials[0]["steps"]:
        cells = [event["job"], event["command"], str(event["source_seconds"]),
                 str(event["replay_seconds"]), event["skip"] or "retained"]
        lines.append("| " + " | ".join(c.replace('|', '\\|') for c in cells) + " |")
    (here / "step-timings.md").write_text("\n".join(lines) + "\n")
    print(json.dumps({k: v for k, v in output.items() if k != "trials"}, indent=2))
    print("Trials:", [round(t["replayed_wall_seconds"], 2) for t in trials])


if __name__ == "__main__":
    main()
