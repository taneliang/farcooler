#!/usr/bin/env python3
"""run_batch.py BINARY FIRST LAST LOG: runs FIRST..LAST of the compose refusal
test under 8 load processes: `load.py 6 1` is six busy loops, and one shell
with the one child it forks and execs without pause. The load stops with the
batch. Every run's whole output is appended to LOG."""
import os, subprocess, sys, time
binary, first, last, path = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
here = os.path.dirname(os.path.abspath(__file__))
test = "watch::answer_wake::tests::compose_tests::a_draft_or_a_dialog_is_refused_untyped"
load = subprocess.Popen([sys.executable, os.path.join(here, "load.py"), "6", "1", "540"])
passed = 0
try:
    time.sleep(10)
    with open(path, "a") as log:
        for i in range(first, last + 1):
            began, at = time.monotonic(), os.getloadavg()[0]
            try:
                r = subprocess.run([binary, "--exact", test], cwd=os.path.join(here, "../../crates/daemon"), capture_output=True, text=True, timeout=120)
                code, text = r.returncode, r.stdout + r.stderr
            except subprocess.TimeoutExpired as e:
                code, text = "timeout", str(e)
            ok = code == 0 and "1 passed" in text
            passed += ok
            log.write(f"=== run {i}: {'PASS' if ok else 'FAIL'} exit={code} {time.monotonic()-began:.1f}s load1={at:.0f}\n{text}\n")
            log.flush()
    print(f"runs {first}-{last}: {passed}/{last - first + 1} passed, load1 now {os.getloadavg()[0]:.0f}")
finally:
    load.terminate(); load.wait()
