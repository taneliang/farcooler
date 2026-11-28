#!/usr/bin/env python3
"""run_under_load.py OLD NEW RUNS OUTDIR: the compose refusal test, the build
before the fix and the build after it in turn, RUNS times each, under
`load.py 60 16`. Every run's whole output is kept."""
import os, subprocess, sys, time
old, new, runs, out = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
here = os.path.dirname(os.path.abspath(__file__))
test = "watch::answer_wake::tests::compose_tests::a_draft_or_a_dialog_is_refused_untyped"
load = subprocess.Popen([sys.executable, os.path.join(here, "load.py"), "60", "16", "3600"])
passed = {"old": 0, "new": 0}
try:
    time.sleep(45)
    logs = {k: open(os.path.join(out, f"runs-{k}.txt"), "w") for k in passed}
    for i in range(1, runs + 1):
        for which, binary in (("old", old), ("new", new)):
            began, at = time.monotonic(), os.getloadavg()[0]
            try:
                r = subprocess.run([binary, "--exact", test], cwd="crates/daemon", capture_output=True, text=True, timeout=180)
                code, text = r.returncode, r.stdout + r.stderr
            except subprocess.TimeoutExpired as e:
                code, text = "timeout", str(e)
            ok = code == 0 and "1 passed" in text
            passed[which] += ok
            logs[which].write(f"=== run {i}/{runs}: {'PASS' if ok else 'FAIL'} exit={code} {time.monotonic()-began:.1f}s load1={at:.0f}\n{text}\n")
            logs[which].flush()
        if i % 20 == 0:
            print(f"{i}: old {passed['old']} passed, new {passed['new']} passed, load1={os.getloadavg()[0]:.0f}", flush=True)
finally:
    load.terminate(); load.wait()
summary = f"before the fix: {passed['old']}/{runs} passed\nafter the fix:  {passed['new']}/{runs} passed\n"
open(os.path.join(out, "runs-summary.txt"), "w").write(summary)
print(summary)
