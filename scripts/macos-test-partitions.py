#!/usr/bin/env python3
"""Opt-in separate-process Mac test experiment (ov-1119), after test.sh builds.

Run under scripts/heavy. Builds are deliberately outside the timed region:
  apps/macos/test.sh
  scripts/heavy -- scripts/macos-test-partitions.py --out /tmp/mac-partitions

CI uses this after one build via test.sh (ov-1183); local use stays opt-in.
Suites remain intact: AppKit and NativeAgents globals never cross processes.
All test IDs, including disabled captures and parameterized methods, come from
SwiftPM's built inventory; no hand-maintained skip list. The outer file leak
check sees the union once (per-process system-temp snapshots would accuse a
sibling process); each shard has its own tmux namespace and completion guard.
"""
import argparse
import copy
import concurrent.futures
import contextlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time
from unittest import mock

ROOT = Path(__file__).resolve().parent.parent
PACKAGE = ROOT / "apps/macos"
FLOOR = PACKAGE / "test-floor.txt"
ISOLATED = "CeremonyTests.NativeSelectionTests/TranscriptScrollerTests"


def discover_runtime():
    def output(command):
        return subprocess.check_output(command, cwd=PACKAGE, text=True).strip()
    swift = Path(output(["xcrun", "--find", "swift"]))
    platform = Path(output(["xcrun", "--show-sdk-platform-path"])) / "Developer"
    return dict(helper=str(swift.parent.parent / "libexec/swift/pm/swiftpm-testing-helper"),
                bin_dir=output(["swift", "build", "--show-bin-path"]),
                frameworks=str(platform / "Library/Frameworks"), libraries=str(platform / "usr/lib"))


def execute_shard(args):
    """Run built bundles without SwiftPM's command-long workspace lock."""
    runtime = json.loads(args.runtime.read_text())
    ids = json.loads(args.run_shard.read_text())
    env = {**os.environ, "DYLD_FRAMEWORK_PATH": runtime["frameworks"],
           "DYLD_LIBRARY_PATH": runtime["libraries"]}
    for module in sorted({test.split(".", 1)[0] for test in ids}):
        bundle = Path(runtime["bin_dir"]) / f"{module}.xctest/Contents/MacOS/{module}"
        if not bundle.is_file():
            raise ValueError(f"build the tests first; missing {bundle}")
        command = [runtime["helper"], "--test-bundle-path", str(bundle), str(bundle),
                   "--testing-library", "swift-testing"]
        if args.test_filter:
            command += ["--skip" if args.exclude else "--filter", args.test_filter]
        rc = subprocess.run(command, env=env).returncode
        if rc:
            return rc
    return 0


def partition(inventory, count):
    """Partition by the measured critical tail, preserving the exact union."""
    if not inventory or len(inventory) != len(set(inventory)):
        raise ValueError("empty or duplicate Swift test inventory")
    for test in inventory:
        if not re.fullmatch(r"[\w.]+/(?:\w+/)*[^/\s]+", test):
            raise ValueError(f"unrecognized Swift test ID: {test}")
    # A measured 124-second serialized tail, with private RealSession windows,
    # models and image directories. Keep every OTHER suite together: they can
    # share UserDefaults.standard keys or the system pasteboard even across
    # process boundaries. AppKit/NativeAgents objects themselves are process
    # local. Do not generalize to arbitrary concurrent UI partitions yet.
    if count == 1:
        return [sorted(inventory)]
    if count != 2:
        raise ValueError("only the measured two-process isolation is supported")
    isolated = [test for test in inventory if test.rsplit("/", 1)[0] == ISOLATED]
    other = [test for test in inventory if test.rsplit("/", 1)[0] != ISOLATED]
    if not isolated or not other:
        raise ValueError("isolated suite missing; review the partition against this inventory")
    shards = [sorted(isolated), sorted(other)]
    union = [test for shard in shards for test in shard]
    if len(union) != len(inventory) or set(union) != set(inventory):
        raise ValueError("partition lost or duplicated tests")
    return shards


def experiment(args):
    args.out.mkdir(parents=True, exist_ok=False)
    listed = subprocess.run(["swift", "test", "list", "--skip-build"], cwd=PACKAGE,
                            capture_output=True, text=True, check=True)
    discovered = sorted(line.strip() for line in listed.stdout.splitlines() if "/" in line)
    ceremony = sum(test.startswith("CeremonyTests.") for test in discovered)
    floor = int(FLOOR.read_text().strip())
    if ceremony < floor:
        raise ValueError(f"CeremonyTests inventory {ceremony} below the floor {floor}")
    inventory = discovered
    if args.suite:
        if args.processes != 1:
            raise ValueError("an isolated measurement uses --processes 1")
        inventory = [test for test in inventory if test.rsplit("/", 1)[0] == args.suite]
    shards = partition(inventory, args.processes)
    (args.out / "inventory.json").write_text(json.dumps(shards, indent=2) + "\n")
    runtime = args.out / "runtime.json"
    runtime.write_text(json.dumps(discover_runtime(), indent=2) + "\n")
    # Count definitions rather than argument instances: parameterized tests run
    # every case. Disabled tests are counted by Swift Testing as skipped.
    def run(i):
        floor = args.out / f"{i}.floor"
        floor.write_text(str(len(shards[i])) + "\n")
        selection = []
        if args.suite or args.processes == 2:
            suite = args.suite or ISOLATED
            # Swift Testing appends /source-file:line:column to runtime IDs.
            # Anchor the suite PREFIX, not the method's end or a filename
            # substring (which can accidentally select captures in that file).
            pattern = "^" + re.escape(suite) + "/"
            matching = {test for test in discovered if re.search(pattern, test)}
            wanted = {test for test in discovered if test.rsplit("/", 1)[0] == suite}
            if matching != wanted:
                raise ValueError("selector includes nested suites; review the discovered inventory")
            selection = ["--test-filter", pattern]
            if not args.suite and i == 1:
                selection += ["--exclude"]
        shard_file = args.out / f"{i}.inventory.json"
        shard_file.write_text(json.dumps(shards[i]) + "\n")
        command = [str(ROOT / "scripts/swift-test-guard.sh"), str(floor), "--",
                   str(ROOT / "scripts/tmux-leak-check.py"), "--",
                   sys.executable, str(Path(__file__).resolve()), "--run-shard", str(shard_file),
                   "--runtime", str(runtime), *selection]
        # Match scripts/heavy's documented five-minute load criterion. Keep
        # the one- and fifteen-minute observations in the report as well.
        while os.getloadavg()[1] >= 20:
            print("waiting for 5-minute load below 20", flush=True)
            time.sleep(5)
        start, load = time.monotonic(), os.getloadavg()
        peak = list(load)
        with (args.out / f"{i}.log").open("w") as log:
            child = subprocess.Popen(command, cwd=PACKAGE, stdout=log, stderr=subprocess.STDOUT,
                                     env={**os.environ, "SWIFT_TEST_SUM_ALL": "1", "SWIFT_TEST_EXACT": "1"})
            while True:
                peak = [max(a, b) for a, b in zip(peak, os.getloadavg())]
                try:
                    rc = child.wait(timeout=1)
                    break
                except subprocess.TimeoutExpired:
                    pass
        # Retain ordinary CI test evidence, including failures and summaries.
        print((args.out / f"{i}.log").read_text(), flush=True)
        return {"shard": i, "rc": rc, "seconds": time.monotonic() - start,
                "started_at": start,
                "load_start": load, "load_end": os.getloadavg(), "load_peak": peak,
                "valid_measurement": peak[1] < 20, "definitions": len(shards[i])}
    start = time.monotonic()
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.processes) as pool:
        results = list(pool.map(run, range(args.processes)))
    wall = max(r["started_at"] + r["seconds"] for r in results) - min(r["started_at"] for r in results)
    for result in results:
        del result["started_at"]
    report = {"processes": args.processes, "wall_seconds": wall,
              "driver": "built Swift Testing helpers (no SwiftPM workspace lock)",
              "load_criterion": "five-minute average below 20 throughout (scripts/heavy)",
              "elapsed_including_load_wait": time.monotonic() - start,
              "runner_seconds": sum(r.get("seconds", 0) for r in results),
              "suite": args.suite, "shards": results}
    (args.out / "timing.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return int(any(r["rc"] or not r.get("valid_measurement", False) for r in results))


def self_test():
    real = [f"{ISOLATED}/short(rows:)", "CeremonyTests.NativeSelectionTests/selection()", "CeremonyTests.Other/test()"]
    assert partition(real, 2) == [[real[0]], sorted(real[1:])]
    assert sorted(sum(partition(real, 2), [])) == sorted(real)
    assert partition(real, 1) == [sorted(real)]
    for bad in ([], real + real[:1], ["bad"], ["CeremonyTests.Other/test()"]):
        try:
            partition(bad, 2)
        except ValueError:
            pass
        else:
            raise AssertionError("invalid inventory accepted")
    print("ok: intact suites; exact inventory union; invalid or changed inventory refused")
    # Exercise the real child invocations and floors. Merely checking the
    # partition function would miss SwiftPM filtering or summary wiring bugs.
    with tempfile.TemporaryDirectory(prefix="ov1119-partitions-") as scratch:
        base = Path(scratch)
        swift = base / "swift"
        swift.write_text("#!/usr/bin/env python3\n" +
            "import os, re, sys\n" +
            f"ids = {real!r}\n" +
            "if 'list' in sys.argv:\n"
            "    print('\\n'.join(ids))\n"
            "else:\n"
            "    skip = '--skip' in sys.argv\n"
            "    flag = '--skip' if skip else '--filter'\n"
            "    pattern = sys.argv[sys.argv.index(flag) + 1]\n"
            "    n = sum(bool(re.search(pattern, x + '/TranscriptScrollerTests.swift:1:1')) != skip for x in ids)\n"
            "    if os.environ.get('OMIT_TEST') and skip: n = 0\n"
            "    print(f'Test run with {n} tests passed after 0.01 seconds.')\n")
        swift.chmod(0o755)
        bundle = base / "CeremonyTests.xctest/Contents/MacOS/CeremonyTests"
        bundle.parent.mkdir(parents=True)
        bundle.touch()
        runtime = dict(helper=str(swift), bin_dir=str(base), frameworks=str(base), libraries=str(base))
        floor = base / "test-floor.txt"
        floor.write_text("3\n")
        env = {**os.environ, "PATH": str(base) + os.pathsep + os.environ["PATH"],
               "HEAVY_LOCK_HELD": "1"}
        for omit in (False, True):
            out = base / ("missing" if omit else "green")
            # Only fake Swift runs here: their correctness must not depend on
            # the host's current load. Actual measurements always sample it.
            with mock.patch.dict(os.environ, {**env, "OMIT_TEST": "1" if omit else ""}), \
                    mock.patch(__name__ + ".FLOOR", floor), \
                    mock.patch("os.getloadavg", return_value=(0., 0., 0.)), \
                    mock.patch(__name__ + ".discover_runtime", return_value=runtime), \
                    (base / "run.log").open("w") as log, contextlib.redirect_stdout(log):
                rc = experiment(argparse.Namespace(out=out, suite=None, processes=2))
            assert (rc != 0) == omit, (base / "run.log").read_text()
            if omit:
                assert "below the floor" in (out / "1.log").read_text()
    print("ok: real shard guards accept the exact union and reject an omitted test")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--out", type=Path)
    ap.add_argument("--processes", type=int, default=2)
    ap.add_argument("--repeat", type=int, default=2, help="green repetitions per configuration (default 2)")
    ap.add_argument("--suite", help="one exact suite ID to measure alone, with --processes 1")
    ap.add_argument("--inside-leak-check", action="store_true", help=argparse.SUPPRESS)
    ap.add_argument("--run-shard", type=Path, help=argparse.SUPPRESS)
    ap.add_argument("--runtime", type=Path, help=argparse.SUPPRESS)
    ap.add_argument("--test-filter", help=argparse.SUPPRESS)
    ap.add_argument("--exclude", action="store_true", help=argparse.SUPPRESS)
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not (os.environ.get("HEAVY_LOCK_HELD") or os.environ.get("CI")):
        ap.error("run through scripts/heavy to preserve resource limits")
    if args.run_shard:
        return execute_shard(args)
    if not args.out:
        ap.error("--out is required")
    if args.repeat < 1:
        ap.error("--repeat must be positive")
    args.out = args.out.resolve()
    if not args.inside_leak_check:
        return subprocess.run([str(ROOT / "scripts/test-leak-check.py"), "--", sys.executable,
                               str(Path(__file__).resolve()), *sys.argv[1:], "--inside-leak-check"]).returncode
    if args.repeat == 1:
        return experiment(args)
    args.out.mkdir(parents=True, exist_ok=False)
    reports = []
    for repetition in range(args.repeat):
        child = copy.copy(args)
        child.out = args.out / f"run-{repetition + 1}"
        rc = experiment(child)
        reports.append(json.loads((child.out / "timing.json").read_text()))
        # A compact parent status lets automation distinguish a red test from
        # a green repetition whose load invalidated its timing. Keep each
        # repetition's own inventory, exact floors, logs and timings intact.
        (args.out / "timing.json").write_text(json.dumps({
            "repetitions": len(reports), "shards": [s for r in reports for s in r["shards"]]
        }, indent=2) + "\n")
        if rc:
            return rc
    return 0


if __name__ == "__main__":
    sys.exit(main())
