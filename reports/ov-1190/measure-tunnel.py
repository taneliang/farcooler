#!/usr/bin/env python3
"""Compare the old serial Go checks with two concurrent shards on this host.

Run with warm Go caches; this measures the checks, not GitHub VM setup/queueing.
Both modes vet and execute uncached tests for BOTH build variants. Run from
any directory, with TMPDIR/GOCACHE pointing to writable scratch directories.
"""
import concurrent.futures
import json
import os
from pathlib import Path
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]


def shard(cgo):
    env = dict(os.environ, CGO_ENABLED=str(cgo))
    start = time.monotonic()
    for args in (["go", "vet", "./..."], ["go", "test", "-count=1", "./..."]):
        result = subprocess.run(args, cwd=ROOT / "crates/tailcat/go", env=env,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        if result.returncode:
            raise RuntimeError(result.stdout)
    return time.monotonic() - start


def main():
    # Warm both build variants before measuring either scheduling policy.
    for cgo in (0, 1):
        shard(cgo)
    rows = []
    for repetition in range(3):
        # Alternate ordering to avoid always favoring the second mode.
        for mode in (("serial", "parallel") if repetition % 2 == 0 else ("parallel", "serial")):
            start = time.monotonic()
            if mode == "serial":
                times = [shard(cgo) for cgo in (1, 0)]
            else:
                with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                    times = list(pool.map(shard, (1, 0)))
            rows.append({"repetition": repetition + 1, "mode": mode,
                         "wall_minutes": (time.monotonic() - start) / 60,
                         "summed_check_minutes": sum(times) / 60})
    print(json.dumps({"rows": rows, "median_wall_minutes": {
        mode: statistics.median(r["wall_minutes"] for r in rows if r["mode"] == mode)
        for mode in ("serial", "parallel")}}, indent=2))


if __name__ == "__main__":
    main()
