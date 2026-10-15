#!/usr/bin/env python3
"""Fail if a test run leaves files behind in TMPDIR or ~/Library/Preferences (ov-541).

On 14 Oct the owner's machine held 99,566 UUID-named test-suite plists in
~/Library/Preferences and about 685,000 `farcooler-*` entries in $TMPDIR. A
test that makes a defaults suite and never removes it, or a scratch directory
and never deletes it, passes every time, so nothing in a run noticed. This
notices.

  TMPDIR       The command runs with a private TMPDIR that this script makes, so
               every temp file the tests create (Rust `std::env::temp_dir()`,
               Swift `NSTemporaryDirectory()`) lands there and nowhere else.
               Anything of ours left in it afterwards is a leak, and another
               process (the owner's app, another lane) cannot cause a false one.
               Anything but a build tool's own scratch (`TemporaryDirectory.*`,
               `*.lock`, `swift-*`, `clang*`) is a test's.
  Preferences  A defaults domain named "x" lives in the real
               ~/Library/Preferences/x.plist, which cannot be redirected, and
               cfprefsd keeps writing it after the suite is "removed". Swift
               tests therefore make suites with `ScratchDefaults`, which keeps
               them in a folder under TMPDIR (a suite named by a full path lives
               at that path), so a test run writes no plist at all. This compares
               the folder before and after and flags a new plist with a UUID in
               its name (every test suite appends one). On CI, where nothing else
               runs, that is a failure. On a Mac that other runs share it is only
               a note, since a new plist is as likely another run's; set
               FC_LEAK_STRICT=1 to fail there too. cfprefsd writes a moment
               late, so it waits a few seconds before it decides.

It only reports. It never deletes anything, and it never looks at a file that
was there before the run.

  ./scripts/test-leak-check.py -- cargo test -p farcooler-core
  ./scripts/test-leak-check.py -- swift test --package-path apps/shared/AgentKit
  ./scripts/test-leak-check.py --lint
  ./scripts/test-leak-check.py --self-test

`--lint` is the bypass rule read from the source, which holds on a busy Mac too:
a Swift test that calls `UserDefaults(suiteName:)` itself, instead of
`ScratchDefaults.suite`, fails.
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

GRACE_SECONDS = 8
UUID = re.compile(r"[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}")
# What a tool leaves in a TMPDIR of its own: not a test's. Each entry, and why:
#   TemporaryDirectory.*   SwiftPM's scratch folders, left by `swift test`/`swift build`.
#   swift-, clang          the Swift and clang drivers' scratch files.
#   com.apple.*, .com.apple.*, xcrun_   Apple tools' (xcrun, system services).
#   .DS_Store              Finder.
#   *.lock                 SwiftPM's package and workspace locks, named by path.
#   .psub.*                fish's `psub` (process substitution) files, from the
#                          login shell the tests' panes run when a pane is killed
#                          before its shell cleans up (observed in the Rust run).
#   .tmpXXXX holding only .local/state/gh/device-id
#                          the real `gh` CLI, run by daemon code with HOME set
#                          to a test's TempDir, writes its device id after the
#                          TempDir is gone and recreates the directory (seen in
#                          the client and daemon runs, only now and then). Any
#                          other content in a .tmp* directory is a leak.
NOISE = re.compile(r"^(TemporaryDirectory\.|swift-|clang|com\.apple\.|xcrun_|\.DS_Store$|\.com\.apple\.|\.psub\.)|\.lock$")


def preferences_dir():
    return os.environ.get("FC_LEAK_PREFERENCES") or os.path.expanduser("~/Library/Preferences")


def plists():
    """Every plist in the preferences folder (a folder that is not there holds none)."""
    try:
        return {n for n in os.listdir(preferences_dir()) if n.endswith(".plist")}
    except OSError:
        return set()


def new_plists(before):
    return sorted(n for n in plists() - before if UUID.search(n))


def only_gh_state(path):
    """Whether `path` is a directory holding nothing but gh's `device-id` file."""
    found = []
    for folder, _dirs, files in os.walk(path):
        found += [os.path.relpath(os.path.join(folder, f), path) for f in files]
    return found == [os.path.join(".local", "state", "gh", "device-id")]


def left_in(base):
    try:
        names = os.listdir(base)
    except OSError:
        return []
    return sorted(
        n for n in names
        if not NOISE.search(n) and not (n.startswith(".tmp") and only_gh_state(os.path.join(base, n)))
    )


def leaks(base, before, grace=GRACE_SECONDS):
    """(temp entries, new plists, whether the plists fail the run) after `grace` seconds."""
    deadline = time.time() + grace
    strict = bool(os.environ.get("CI") or os.environ.get("FC_LEAK_STRICT"))
    while True:
        temp, prefs = left_in(base), new_plists(before)
        if not (temp or prefs) or time.time() >= deadline:
            return temp, prefs, strict
        time.sleep(0.25)


def report(temp, prefs):
    for title, names, where in (
        ("temp entries", temp, "TMPDIR"),
        ("preference plists", prefs, preferences_dir()),
    ):
        if not names:
            continue
        print(f"test-leak-check: the run left {len(names)} {title} in {where}:", file=sys.stderr)
        for name in names[:40]:
            print(f"  {name}", file=sys.stderr)
        if len(names) > 40:
            print(f"  ... and {len(names) - 40} more", file=sys.stderr)
    print(
        "test-leak-check: a test must remove what it makes. Swift: a defaults suite from "
        "`ScratchDefaults.suite(_:)`, a directory from `ScratchDirectory.make(_:)`. "
        "Rust: a directory from `farcooler_core::scratch::dir(_:)`.",
        file=sys.stderr,
    )


SUITE = re.compile(r"\b(NS)?UserDefaults\s*(\.\s*init)?\s*\(\s*suiteName\b")


def lint(root):
    """Swift test files under `root` that make a defaults suite without ScratchDefaults.

    The whole file is read, so a call broken over lines or written `.init(` is
    seen; comments are blanked first, so a mention in one is not. `.claude` (other
    lanes' worktrees) and build folders are skipped.
    """
    bad = []
    for folder, dirs, files in os.walk(root):
        dirs[:] = [d for d in dirs if d not in (".build", ".git", ".claude", "target", "node_modules", "DerivedData")]
        if not any(p == "Tests" or p.endswith("Tests") for p in folder.split(os.sep)):
            continue
        for name in files:
            if not name.endswith(".swift") or name == "ScratchDefaults.swift":
                continue
            path = os.path.join(folder, name)
            with open(path, encoding="utf-8", errors="replace") as f:
                text = f.read()
            text = re.sub(r"/\*.*?\*/", lambda m: re.sub(r"[^\n]", " ", m.group(0)), text, flags=re.S)
            text = re.sub(r"//[^\n]*", "", text)
            for m in SUITE.finditer(text):
                bad.append(f"{os.path.relpath(path, root)}:{text.count(chr(10), 0, m.start()) + 1}")
    return bad


WIRING = (
    (".github/workflows/ci.yml", re.compile(r"test-leak-check\.py -- .*cargo test --workspace"), "the Rust workspace test"),
    (".github/workflows/ci.yml", re.compile(r"test-leak-check\.py -- swift test --package-path apps/shared/AgentKit"), "the AgentKit test"),
    ("apps/macos/test.sh", re.compile(r"test-leak-check\.py -- .*swift test"), "the Mac tests"),
)


def wiring(root):
    """The test runs that must go through this script, and do not."""
    missing = []
    for path, pattern, what in WIRING:
        try:
            with open(os.path.join(root, path), encoding="utf-8") as f:
                text = f.read()
        except OSError:
            text = ""
        if not pattern.search(text):
            missing.append(f"{path}: {what} no longer runs under scripts/test-leak-check.py")
    return missing


def run_lint(root, check_wiring=False):
    for line in wiring(root) if check_wiring else []:
        print(line + " (ov-541)", file=sys.stderr)
    bad = lint(root)
    for place in bad:
        print(f"{place}: `UserDefaults(suiteName:)` in a test; use `ScratchDefaults.suite(_:)`, which keeps it out of ~/Library/Preferences (ov-541)", file=sys.stderr)
    return 1 if bad or (check_wiring and wiring(root)) else 0


def run(command, grace=GRACE_SECONDS):
    base = tempfile.mkdtemp(prefix="fcl-", dir="/tmp")
    before = plists()
    # A trailing slash, as the system's own temporary directory has: code that
    # joins with `NSTemporaryDirectory() + "name"` relies on it.
    env = dict(os.environ, TMPDIR=base + os.sep)
    try:
        rc = subprocess.call(command, env=env)
        temp, prefs, strict = leaks(base, before, grace)
        if prefs and not strict:
            print(
                f"test-leak-check: note: {len(prefs)} new plists appeared in {preferences_dir()}, "
                f"which another run on this Mac may have made (CI and FC_LEAK_STRICT=1 fail on them): "
                f"{', '.join(prefs[:3])}{' ...' if len(prefs) > 3 else ''}",
                file=sys.stderr,
            )
            prefs = []
        if temp or prefs:
            report(temp, prefs)
            return rc or 1
        print("test-leak-check: the run left nothing in TMPDIR or the preferences folder.")
        return rc
    finally:
        shutil.rmtree(base, ignore_errors=True)


def self_test():
    """Show it red for each kind of leak and green for none, on a scratch preferences folder."""
    scratch = tempfile.mkdtemp(prefix="fc-leak-selftest-")
    prefs = os.path.join(scratch, "prefs")
    os.mkdir(prefs)
    os.environ["FC_LEAK_PREFERENCES"] = prefs
    uuid = "0A1B2C3D-0000-4000-8000-000000000001"
    failures = []

    def case(name, script, expect_rc):
        rc = run([sys.executable, "-c", script], grace=0.5)
        if (rc != 0) != (expect_rc != 0):
            failures.append(f"{name}: exit {rc}, wanted {'non-zero' if expect_rc else 'zero'}")

    clean = "import os; d=os.path.join(os.environ['TMPDIR'],'farcooler-x'); os.mkdir(d); os.rmdir(d)"
    case("a run that cleans up", clean, 0)
    case("a run that leaves a farcooler- directory",
         "import os; os.mkdir(os.path.join(os.environ['TMPDIR'],'farcooler-route-1'))", 1)
    case("a run that leaves a file of its own",
         "import os; open(os.path.join(os.environ['TMPDIR'],'ov519-1'),'w').close()", 1)
    plist = os.path.join(prefs, "ov113-" + uuid + ".plist")
    leave = f"open({plist!r},'w').close()"
    ci = os.environ.pop("CI", None)
    strict = os.environ.pop("FC_LEAK_STRICT", None)
    case("a new UUID plist, on a Mac others share", leave, 0)
    os.remove(plist)
    os.environ["FC_LEAK_STRICT"] = "1"
    case("a new UUID plist, strict", leave, 1)
    case("a run that makes a UUID plist and removes it", "import os; " + leave + f"; os.remove({plist!r})", 0)
    case("a run that touches a plist with no UUID",
         f"open({os.path.join(prefs, 'com.example.app.plist')!r},'w').close()", 0)
    del os.environ["FC_LEAK_STRICT"]
    os.environ["CI"] = "1"
    case("a new UUID plist, on CI", leave, 1)
    os.remove(plist)
    del os.environ["CI"]
    for key, value in (("CI", ci), ("FC_LEAK_STRICT", strict)):
        if value is not None:
            os.environ[key] = value
    case("gh's state in a recreated .tmp directory",
         "import os; d=os.path.join(os.environ['TMPDIR'],'.tmpAbC123','.local','state','gh'); os.makedirs(d); open(os.path.join(d,'device-id'),'w').close()", 0)
    case("another file in a .tmp directory",
         "import os; d=os.path.join(os.environ['TMPDIR'],'.tmpAbC123'); os.makedirs(d); open(os.path.join(d,'x'),'w').close()", 1)
    case("an unrelated temp file",
         "import os; open(os.path.join(os.environ['TMPDIR'],'clang-cache'),'w').close()", 0)
    # The command's own failure must come through even when it leaks nothing.
    if run([sys.executable, "-c", "raise SystemExit(3)"], grace=0.5) != 3:
        failures.append("a failing command's exit status was lost")

    tree = os.path.join(scratch, "tree", "Tests", "XTests")
    os.makedirs(tree)
    with open(os.path.join(tree, "A.swift"), "w") as f:
        f.write('let d = ScratchDefaults.suite("a")\n')
    if run_lint(os.path.join(scratch, "tree")) != 0:
        failures.append("the lint flagged a test that uses ScratchDefaults")
    with open(os.path.join(tree, "B.swift"), "w") as f:
        f.write('let d = UserDefaults(suiteName: "b")\n')
    if run_lint(os.path.join(scratch, "tree")) == 0:
        failures.append("the lint missed a test that makes its own suite")

    for bypass in ('let d = UserDefaults.init(suiteName: "b")', 'let d = UserDefaults (\n    suiteName: "b")',
                   'let d = NSUserDefaults(suiteName: "b")'):
        with open(os.path.join(tree, "B.swift"), "w") as f:
            f.write(bypass + "\n")
        if run_lint(os.path.join(scratch, "tree")) == 0:
            failures.append(f"the lint missed {bypass!r}")
    with open(os.path.join(tree, "B.swift"), "w") as f:
        f.write('// never UserDefaults(suiteName: "x") here\nlet d = 1\n')
    if run_lint(os.path.join(scratch, "tree")) != 0:
        failures.append("the lint flagged a comment")
    # The wiring: a tree with none of the three runs is red, and so is a tree
    # where one line lost the wrapper.
    if not wiring(os.path.join(scratch, "tree")):
        failures.append("the wiring check passed a tree with no wrapped runs")
    shutil.rmtree(scratch, ignore_errors=True)
    if failures:
        print("test-leak-check self-test FAILED:\n  " + "\n  ".join(failures), file=sys.stderr)
        return 1
    print("test-leak-check self-test passed.")
    return 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    if argv == ["--lint"]:
        return run_lint(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."), check_wiring=True)
    if "--" not in argv or argv.index("--") == len(argv) - 1:
        print("usage: test-leak-check.py -- <command...> | --self-test", file=sys.stderr)
        return 2
    return run(argv[argv.index("--") + 1 :])


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
