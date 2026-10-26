#!/usr/bin/env python3
"""Check each Swift test floor against the tests in the tree, without running them (ov-817).

A floor (apps/macos/test-floor.txt, apps/shared/AgentKit/test-floor.txt) is the
count `swift test` must reach, and a lane that adds tests raises it. Two lanes
that each raised it, folded into one train by hand, leave a number that is
neither lane's count nor the sum. Only a full Swift run notices, and that is
minutes on a Mac.

What is cheap is the count of test declarations: `@Test` and `func testXxx`
lines under the suite's folder. It is not the run's count (a parameterized
`@Test` runs once per argument), but the two move together: in the last ten
floor commits of each suite the floor sat 16 to 51 below the declarations.
So this does not compare the floor with the declarations; it compares how far
apart they are on this tree with how far apart they were at the base:

    drift = (floor - declarations) on this tree - (floor - declarations) at the base

A fold that adds N tests and raises the floor by N leaves the drift at 0. One
that keeps a single lane's floor, or a floor typed for a different count, moves
it by the size of the miss. It fails when the drift's size exceeds the
tolerance (default 2, `--tolerance N`; the Mac's drift stays within 3 across its
last twelve floor commits), and when the floor is above the
declarations (it asks for tests that do not exist). A parameterized test added
with many arguments moves the drift legitimately: raise the tolerance for that
run, or look at the number the last full run printed.

    ./scripts/test-floor-check.py                    # against origin/main
    ./scripts/test-floor-check.py --base <rev> [--tolerance N]
    ./scripts/test-floor-check.py --self-test
"""
import argparse
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SUITES = ["apps/macos", "apps/shared/AgentKit"]
DECL = r"^[[:space:]]*@Test([[:space:](]|$)|func test[A-Z_]"


def git(*args, cwd=ROOT):
    return subprocess.run(["git", "-C", str(cwd), *args], capture_output=True, text=True)


def declarations(rev, suite, cwd=ROOT):
    """Test declarations under `suite` at `rev` (None: the working tree)."""
    args = ["grep", "-h"] + ([] if rev else ["--untracked"]) + ["-E", DECL] + ([rev] if rev else []) + ["--", suite]
    r = git(*args, cwd=cwd)
    return len(r.stdout.splitlines())


def floor_at(rev, suite, cwd=ROOT):
    if rev:
        r = git("show", f"{rev}:{suite}/test-floor.txt", cwd=cwd)
        text = r.stdout if r.returncode == 0 else None
    else:
        p = pathlib.Path(cwd) / suite / "test-floor.txt"
        text = p.read_text() if p.exists() else None
    if text is None:
        return None
    t = text.strip()
    return int(t) if re.fullmatch(r"[0-9]+", t) else "bad"


def judge(name, floor, decls, base_floor, base_decls, tolerance):
    """(ok, message) for one suite, from four numbers. Pure, so the self-test can feed it cases."""
    if floor == "bad" or floor is None:
        return False, f"{name}: test-floor.txt must hold one number"
    head = f"{name}: floor {floor}, {decls} declarations"
    if floor > decls:
        return False, f"{head}: the floor asks for more tests than the tree declares"
    if base_floor is None or base_floor == "bad" or base_decls is None:
        return True, f"{head}: no base to compare drift with; only the ceiling checked"
    drift = (floor - decls) - (base_floor - base_decls)
    note = f"{head}; at base {base_floor} and {base_decls}; drift {drift:+d}"
    if abs(drift) > tolerance:
        return False, (f"{note}: more than {tolerance} off. The tests added or removed since the base "
                       f"({decls - base_decls:+d} declarations) and the floor's change ({floor - base_floor:+d}) disagree. "
                       "Rerun the full suite and set the floor to the count it prints.")
    return True, note


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default="origin/main")
    ap.add_argument("--tolerance", type=int, default=2)
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args(argv)
    if a.self_test:
        return self_test()
    base = git("merge-base", "HEAD", a.base).stdout.strip() or None
    bad = False
    for s in SUITES:
        ok, msg = judge(s, floor_at(None, s), declarations(None, s),
                        floor_at(base, s) if base else None,
                        declarations(base, s) if base else None, a.tolerance)
        print(("ok   " if ok else "FAIL ") + msg)
        bad |= not ok
    return 1 if bad else 0


def self_test():
    cases = [
        ("clean fold", (2006, 2024, 2001, 2019), True),
        ("kept one lane's floor after +8 tests", (2006, 2027, 2001, 2019), False),
        ("floor above the declarations", (2030, 2024, 2001, 2019), False),
        ("floor raised, no tests added", (2020, 2024, 2001, 2019), False),
        ("not a number", ("bad", 2024, 2001, 2019), False),
        ("no base", (2006, 2024, None, None), True),
    ]
    fails = 0
    for what, (f, d, bf, bd), want in cases:
        got, msg = judge("s", f, d, bf, bd, 2)
        if got != want:
            fails += 1
            print(f"self-test FAIL: {what}: got {got}, wanted {want}: {msg}")
    print("test-floor-check self-test: " + ("FAILED" if fails else f"{len(cases)} cases pass"))
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
