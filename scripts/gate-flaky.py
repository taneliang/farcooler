#!/usr/bin/env python3
"""Rerun a gate's failed Rust tests once, alone, so one flake does not fail a landing (ov-1285).

    scripts/gate-flaky.py run -- cargo nextest run --profile gate --no-fail-fast --archive-file A
    scripts/gate-flaky.py --self-test

The local lane gate and the landing gate run the Rust suite through this wrapper; CI on main does not,
so a real flake still shows there. The wrapper runs the command with GATE_JUNIT set (scripts/quarantine.py
makes nextest write its JUnit report there), then reads which tests failed or timed out. After the main run
each of them is rerun once, one at a time, with nothing else running (`--test-threads 1`, a filter naming
that one test). The command passes if every rerun passes. Each test that needed one is printed as

    FLAKY: <binary id> <test>: first 1.2s (failed), rerun 1.0s (passed)

which the gate copies into its summary and evidence. When GATE_FLAKY_LOG names a file (the lane gate points it
into its own log directory), the same rows are appended there too. The wrapper never writes inside the tree it
gates: a gate that edits a tracked file fails the landing (ov-1295). A test that fails its rerun is printed as FAILED AGAIN and the exit
code is the original one, exactly as it was before this wrapper existed. A run that failed without naming a
failed test (a build error, a crash, a leak check) is never forgiven.
"""
import datetime as dt
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent
LOG = pathlib.Path(os.environ["GATE_FLAKY_LOG"]) if os.environ.get("GATE_FLAKY_LOG") else None  # the gate's log directory; none outside a gate
HEADER = "date\tbinary\ttest\tfirst_secs\trerun_secs\tfirst_result\n"


def failures(report):
    """(binary id, test, seconds, kind) for each case that failed or timed out and never passed on a nextest retry."""
    found = []
    for suite in ET.parse(report).getroot().iter("testsuite"):
        for case in suite.findall("testcase"):
            bad = case.find("failure") if case.find("failure") is not None else case.find("error")
            if bad is None:
                continue
            text = " ".join([bad.attrib.get("type", ""), bad.attrib.get("message", "")]).lower()
            kind = "timed out" if "timeout" in text or "timed out" in text else "failed"
            found.append((suite.attrib["name"], case.attrib["name"], float(case.attrib.get("time", 0) or 0), kind))
    return found


def exact(name):
    """A nextest filter operand matching `name` exactly."""
    return "=" + name


def record(rows, log=LOG, today=None):
    """Append one line per flaky test to the gate's own log, writing its header first when the file is new."""
    today = today or dt.datetime.now(dt.timezone.utc).date().isoformat()
    new = not log.exists() or not log.read_text()
    with open(log, "a") as f:
        if new:
            f.write(HEADER)
        for binary, test, first, rerun, kind in rows:
            f.write(f"{today}\t{binary}\t{test}\t{first:.1f}\t{rerun:.1f}\t{kind}\n")


def run(command, say=print, log=None):
    log = log or LOG
    with tempfile.TemporaryDirectory(prefix="gate-flaky-") as tmp:
        report = pathlib.Path(tmp, "junit.xml")
        rc = subprocess.call(command, env={**os.environ, "GATE_JUNIT": str(report)})
        if rc == 0:
            return 0
        if not report.exists():
            return rc
        try:
            bad = failures(report)
        except ET.ParseError:
            return rc
    cargo = next((i for i in range(len(command) - 2) if command[i:i + 3] == ["cargo", "nextest", "run"]), None)
    if not bad or cargo is None:
        return rc
    # The rerun names its one test, and a second -E would widen it (nextest ORs them), so drop the narrowing the
    # landing gate adds and any partition: the failed test already passed through both.
    tail, skip = [], 0
    for arg in command[cargo:]:
        if skip:
            skip -= 1
        elif arg in ("-E", "--filterset", "--partition"):
            skip = 1
        else:
            tail.append(arg)
    say(f"gate-flaky: {len(bad)} test(s) failed; rerunning each once, alone, now that the main run is over")
    flaky, still = [], []
    for binary, test, first, kind in bad:
        rerun = tail + ["-E", f"binary_id({exact(binary)}) & test({exact(test)})", "--test-threads", "1"]
        started = time.monotonic()
        code = subprocess.call(rerun)
        secs = time.monotonic() - started
        (flaky if code == 0 else still).append((binary, test, first, secs, kind))
    for binary, test, first, secs, kind in flaky:
        say(f"FLAKY: {binary} {test}: first {first:.1f}s ({kind}), rerun {secs:.1f}s (passed)")
    for binary, test, first, secs, kind in still:
        say(f"FAILED AGAIN: {binary} {test}: first {first:.1f}s ({kind}), rerun {secs:.1f}s (failed)")
    if flaky and log:
        record(flaky, log)
        say(f"gate-flaky: {len(flaky)} flaky test(s) appended to {log}")
    return rc if still else 0


FAKE_CARGO = """#!/usr/bin/env python3
import os, pathlib, sys
state = pathlib.Path(os.environ["FAKE_STATE"])
if "-E" not in sys.argv:
    pathlib.Path(os.environ["GATE_JUNIT"]).write_text(
        '<testsuites><testsuite name="pkg::bin"><testcase name="a_test" time="1.25"><failure type="Test failure"/></testcase>'
        '<testcase name="fine" time="0.1"/></testsuite></testsuites>')
    sys.exit(100)
n = int(state.read_text()) + 1 if state.exists() else 1
state.write_text(str(n))
assert "--test-threads" in sys.argv and sys.argv[sys.argv.index("--test-threads") + 1] == "1"
sys.exit(0 if n >= int(os.environ["PASS_ON"]) else 100)
"""


def self_test():
    failures_ = []
    with tempfile.TemporaryDirectory(prefix="gate-flaky-test-") as tmp:
        tmp = pathlib.Path(tmp)
        (tmp / "cargo").write_text(FAKE_CARGO)
        (tmp / "cargo").chmod(0o755)
        for pass_on, want_rc, want_flaky in ((1, 0, True), (2, 100, False)):
            state, log, out = tmp / f"state{pass_on}", tmp / f"log{pass_on}.tsv", []
            with_env = {"FAKE_STATE": str(state), "PASS_ON": str(pass_on)}
            os.environ.update(with_env)
            os.environ["PATH"] = f"{tmp}{os.pathsep}{os.environ['PATH']}"
            got = run(["cargo", "nextest", "run", "--profile", "gate"], out.append, log)
            text = "\n".join(out)
            if got != want_rc:
                failures_.append(f"flaky={want_flaky}: exit {got}, want {want_rc}")
            if ("FLAKY: pkg::bin a_test: first 1.2s (failed), rerun " in text) != want_flaky:
                failures_.append(f"FLAKY line wrong for pass_on={pass_on}: {text!r}")
            if ("FAILED AGAIN: pkg::bin a_test" in text) == want_flaky:
                failures_.append(f"FAILED AGAIN line wrong for pass_on={pass_on}")
            if log.exists() != want_flaky or (want_flaky and "pkg::bin\ta_test\t1.2\t" not in log.read_text()):
                failures_.append(f"flaky log wrong for pass_on={pass_on}")
            if state.read_text() != "1":
                failures_.append("the test was not rerun exactly once")
        if run(["python3", "-c", "import sys; sys.exit(3)"], lambda _l: None, tmp / "x.tsv") != 3:
            failures_.append("a failure with no report must keep its exit code")
    for line in failures_:
        print("gate-flaky self-test: FAIL " + line)
    print("gate-flaky self-test: " + ("FAILED" if failures_ else "ok"))
    return 1 if failures_ else 0


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    if argv[:1] == ["run"]:
        command = argv[1:]
        return run(command[1:] if command[:1] == ["--"] else command)
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
