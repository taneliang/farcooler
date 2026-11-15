#!/usr/bin/env python3
"""Wait while a main CI run is queued or running, so Nightly yields (ov-1226).

Nightly runs on its own ref, so no concurrency group links it to main's CI.
Each Nightly Apple phase is gated by a small Linux job that runs this first;
the Apple job needs it, so no Mac is held while waiting. It gives up after
--max-minutes and lets the phase run: a yield, not a starvation.
"""
import json
import subprocess
import sys
import time

BUSY = ("queued", "in_progress", "waiting", "pending", "requested")


def main_ci_busy(runs):
    """Whether any run of ci.yml on main, or on a landing candidate's branch, is still waiting or running.
    A candidate's run is the one main's verdict comes from (ov-1269): main's own run of it does nothing."""
    return any(r.get("status") in BUSY and (r.get("headBranch") == "main" or str(r.get("headBranch")).startswith("landing/"))
               for r in runs)


def fetch():
    out = subprocess.run(
        ["gh", "run", "list", "--workflow", "ci.yml", "--event", "push", "--limit", "40",
         "--json", "status,headBranch"], capture_output=True, text=True, check=True).stdout
    return json.loads(out)


def self_test():
    ok = main_ci_busy([{"status": "queued", "headBranch": "main"}])
    ok = ok and main_ci_busy([{"status": "in_progress", "headBranch": "main"}])
    ok = ok and not main_ci_busy([{"status": "completed", "headBranch": "main"}])
    ok = ok and not main_ci_busy([{"status": "in_progress", "headBranch": "feature"}])
    ok = ok and main_ci_busy([{"status": "in_progress", "headBranch": "landing/0123456789abcdef"}])
    ok = ok and not main_ci_busy([{"status": "completed", "headBranch": "landing/0123456789abcdef"}])
    ok = ok and not main_ci_busy([{"status": "in_progress", "headBranch": "landingx"}])
    ok = ok and not main_ci_busy([])
    if not ok:
        print("yield decision self-test failed", file=sys.stderr)
    return 0 if ok else 1


def main():
    if "--self-test" in sys.argv:
        return self_test()
    cap = float(sys.argv[sys.argv.index("--max-minutes") + 1]) if "--max-minutes" in sys.argv else 90
    deadline = time.time() + cap * 60
    while True:
        try:
            busy = main_ci_busy(fetch())
        except (subprocess.CalledProcessError, ValueError, OSError) as err:
            print(f"Could not read main CI ({err}); not waiting.")
            return 0
        if not busy:
            print("No main CI run is queued or running; Nightly proceeds.")
            return 0
        if time.time() >= deadline:
            print(f"Main CI still busy after {cap:g} minutes; Nightly proceeds.")
            return 0
        print("Main CI is queued or running; Nightly yields.", flush=True)
        time.sleep(60)


if __name__ == "__main__":
    sys.exit(main())
