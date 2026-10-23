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

  System temp  Foundation on macOS ignores $TMPDIR: `NSTemporaryDirectory()` and
               `FileManager.temporaryDirectory` ask the system for the user's
               temporary folder (`confstr(_CS_DARWIN_USER_TEMP_DIR)`, under
               /var/folders), whatever TMPDIR says. So a Swift test that leaves a
               folder leaves it there and the private TMPDIR above stays empty:
               the check passed every Swift run (ov-724; 186,000 entries by 23
               Oct). For a `swift` command this also compares that folder before
               and after and flags a new entry that is not a build tool's. Like
               the plists, on a Mac that other runs share a new entry may be
               another run's, so it is a note there and a failure on CI or with
               FC_LEAK_STRICT=1.

It only reports. It never deletes anything, and it never looks at a file that
was there before the run.

  ./scripts/test-leak-check.py -- cargo test -p farcooler-core
  ./scripts/test-leak-check.py -- swift test --package-path apps/shared/AgentKit
  ./scripts/test-leak-check.py --lint
  ./scripts/test-leak-check.py --self-test

`--lint` is the bypass rule read from the source, which holds on a busy Mac too:
a Swift test that calls `UserDefaults(suiteName:)` itself, instead of
`ScratchDefaults.suite`, fails, and so does a Rust test that names
`std::env::temp_dir()` (ov-552), since a directory made there is one more thing
a test must remember to remove: `farcooler_core::scratch::dir(_)` removes the
whole root when the process ends. A deliberate use carries a
`// leak-exempt: <reason>` comment on its line or the one above.
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
#                          The Rust tests' panes run /bin/sh now (ov-547), so this
#                          is a Swift test's pane.
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


def left_in(base):
    try:
        names = os.listdir(base)
    except OSError:
        return []
    return sorted(n for n in names if not NOISE.search(n))


def tool_scratch(base):
    """The entries of `base` that `NOISE` lets pass."""
    try:
        return sorted(n for n in os.listdir(base) if NOISE.search(n))
    except OSError:
        return []


def system_temp():
    """The user's real temporary folder on macOS, which Foundation uses whatever TMPDIR says; None elsewhere."""
    if sys.platform != "darwin":
        return None
    try:
        import ctypes
        import ctypes.util

        libc = ctypes.CDLL(ctypes.util.find_library("c"))
        buffer = ctypes.create_string_buffer(1024)
        # _CS_DARWIN_USER_TEMP_DIR, which Python's os.confstr does not name.
        if not libc.confstr(65537, buffer, len(buffer)):
            return None
        return os.path.realpath(buffer.value.decode())
    except (OSError, AttributeError, UnicodeDecodeError):
        return None


def entries(folder):
    try:
        return set(os.listdir(folder))
    except (OSError, TypeError):
        return set()


def new_system_entries(folder, before):
    """Entries that appeared in the system temp folder during the run and are not a build tool's."""
    return sorted(n for n in entries(folder) - before if not NOISE.search(n))


def is_swift(command):
    return any(os.path.basename(word) == "swift" for word in command)


def leaks(base, before, grace=GRACE_SECONDS, system=None):
    """(temp entries, new plists, whether the plists fail the run, new system temp entries) after `grace` seconds.

    `system` is (folder, names before) for a Swift run on macOS, else None.
    """
    deadline = time.time() + grace
    strict = bool(os.environ.get("CI") or os.environ.get("FC_LEAK_STRICT"))
    while True:
        temp, prefs = left_in(base), new_plists(before)
        sys_new = new_system_entries(*system) if system else []
        if not (temp or prefs or sys_new) or time.time() >= deadline:
            return temp, prefs, strict, sys_new
        time.sleep(0.25)


def report(temp, prefs, sys_new=(), sys_folder=""):
    for title, names, where in (
        ("temp entries", temp, "TMPDIR"),
        ("temp entries", list(sys_new), f"{sys_folder} (the system temporary folder, which Swift uses instead of TMPDIR)"),
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


TEMP_DIR = re.compile(r"\btemp_dir\s*\(\s*\)")
RUST_SKIP = (".build", ".git", ".claude", "target", "node_modules", "DerivedData", "examples")
EXEMPT = "leak-exempt:"


def blank_comments(text):
    """`text` with Rust comments blanked, line numbers kept."""
    text = re.sub(r"/\*.*?\*/", lambda m: re.sub(r"[^\n]", " ", m.group(0)), text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def rust_test_start(path, text):
    """The line (1-based) where `path`'s test code starts, or None for none.

    A file under a `tests` folder, or named `tests.rs`, `*_tests.rs` or
    `*_test.rs`, is test code from line 1; any other file from its first
    `#[cfg(test)]` that opens a module.
    """
    name = os.path.basename(path)
    if "tests" in path.split(os.sep)[:-1] or name == "tests.rs" or name.endswith(("_tests.rs", "_test.rs")):
        return 1
    m = re.search(r"(?m)^#\[cfg\(test\)\]\s*(?:#\[[^\n]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\b", text)
    return text.count("\n", 0, m.start()) + 1 if m else None


def lint_rust(root):
    """Rust test code under `root` that names `temp_dir()`, as `path:line`."""
    bad = []
    for folder, dirs, files in os.walk(root):
        dirs[:] = [d for d in dirs if d not in RUST_SKIP]
        for name in files:
            if not name.endswith(".rs"):
                continue
            path = os.path.join(folder, name)
            with open(path, encoding="utf-8", errors="replace") as f:
                raw = f.read()
            start = rust_test_start(os.path.relpath(path, root), raw)
            if start is None:
                continue
            raw_lines = raw.split("\n")
            for number, line in enumerate(blank_comments(raw).split("\n"), 1):
                if number < start or not TEMP_DIR.search(line):
                    continue
                if EXEMPT in raw_lines[number - 1] or (number > 1 and EXEMPT in raw_lines[number - 2]):
                    continue
                bad.append(f"{os.path.relpath(path, root)}:{number}")
    return bad


WIRING = (
    (".github/workflows/ci.yml", re.compile(r"test-leak-check\.py -- .*cargo (test|nextest run( --profile \S+)?) --workspace"), "the Rust workspace test"),
    (".github/workflows/ci.yml", re.compile(r"test-leak-check\.py -- swift test --package-path apps/shared/AgentKit"), "the AgentKit test"),
    ("apps/macos/test.sh", re.compile(r"test-leak-check\.py -- .*swift test"), "the Mac tests"),
    # The nightly perf runs are `swift test` too (ov-724), each its own run.
    (".github/workflows/nightly.yml", re.compile(r"test-leak-check\.py -- swift test --package-path apps/shared/AgentKit --filter AgentRowBudgetTests"), "the nightly AgentKit row budgets"),
    (".github/workflows/nightly.yml", re.compile(r"test-leak-check\.py -- swift test --filter 'ConversationScrollTests"), "the nightly Mac scrolling test"),
    (".github/workflows/nightly.yml", re.compile(r"test-leak-check\.py -- swift test --filter 'TranscriptJumpTests'"), "the nightly Mac scroll jumps test"),
    (".github/workflows/nightly.yml", re.compile(r"test-leak-check\.py -- swift test --filter 'NativeStreamingStallTests'"), "the nightly Mac streaming test"),
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
        # A commented-out step is no step: YAML and shell comments go first,
        # whole-line or trailing (a `#` after whitespace).
        text = re.sub(r"(?m)(^|\s)#.*$", r"\1", text)
        if not pattern.search(text):
            missing.append(f"{path}: {what} no longer runs under scripts/test-leak-check.py")
    return missing


def run_lint(root, check_wiring=False):
    for line in wiring(root) if check_wiring else []:
        print(line + " (ov-541)", file=sys.stderr)
    bad = lint(root)
    for place in bad:
        print(f"{place}: `UserDefaults(suiteName:)` in a test; use `ScratchDefaults.suite(_:)`, which keeps it out of ~/Library/Preferences (ov-541)", file=sys.stderr)
    rust = lint_rust(root)
    for place in rust:
        print(f"{place}: `temp_dir()` in a Rust test; use `farcooler_core::scratch::dir(_)`, which is removed when the process ends, or say why with `// leak-exempt: <reason>` (ov-552)", file=sys.stderr)
    return 1 if bad or rust or (check_wiring and wiring(root)) else 0


def run(command, grace=GRACE_SECONDS):
    base = tempfile.mkdtemp(prefix="fcl-", dir="/tmp")
    before = plists()
    folder = system_temp() if is_swift(command) else None
    system = (folder, entries(folder)) if folder and folder != os.path.realpath(base) else None
    # A trailing slash, as the system's own temporary directory has: code that
    # joins with `NSTemporaryDirectory() + "name"` relies on it.
    env = dict(os.environ, TMPDIR=base + os.sep)
    try:
        rc = subprocess.call(command, env=env)
        temp, prefs, strict, sys_new = leaks(base, before, grace, system)
        if sys_new and not strict:
            print(
                f"test-leak-check: note: {len(sys_new)} new entries appeared in {folder}, which another run on this Mac "
                f"may have made (CI and FC_LEAK_STRICT=1 fail on them): {', '.join(sys_new[:3])}{' ...' if len(sys_new) > 3 else ''}",
                file=sys.stderr,
            )
            sys_new = []
        if prefs and not strict:
            print(
                f"test-leak-check: note: {len(prefs)} new plists appeared in {preferences_dir()}, "
                f"which another run on this Mac may have made (CI and FC_LEAK_STRICT=1 fail on them): "
                f"{', '.join(prefs[:3])}{' ...' if len(prefs) > 3 else ''}",
                file=sys.stderr,
            )
            prefs = []
        # What it let pass as a tool's own scratch, so the next CI log shows
        # which of the NOISE patterns match anything and the broad ones
        # (`swift-`, `clang`, `*.lock`) can be tightened to what they match.
        passed = tool_scratch(base)
        if passed:
            print(f"test-leak-check: note: ignored {len(passed)} entries as a build tool's scratch: {', '.join(passed[:40])}{' ...' if len(passed) > 40 else ''}")
        if temp or prefs or sys_new:
            report(temp, prefs, sys_new, folder)
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
    # gh's device id no longer gets a pass (ov-552): a test daemon's gh is inert
    # (`gh_inert.rs`), so a `.tmp` directory holding it is a leak like any other.
    case("gh's state in a recreated .tmp directory",
         "import os; d=os.path.join(os.environ['TMPDIR'],'.tmpAbC123','.local','state','gh'); os.makedirs(d); open(os.path.join(d,'device-id'),'w').close()", 1)
    case("another file in a .tmp directory",
         "import os; d=os.path.join(os.environ['TMPDIR'],'.tmpAbC123'); os.makedirs(d); open(os.path.join(d,'x'),'w').close()", 1)
    case("an unrelated temp file",
         "import os; open(os.path.join(os.environ['TMPDIR'],'clang-cache'),'w').close()", 0)
    # The command's own failure must come through even when it leaks nothing.
    if run([sys.executable, "-c", "raise SystemExit(3)"], grace=0.5) != 3:
        failures.append("a failing command's exit status was lost")

    # Foundation's temporary folder is not TMPDIR on macOS (ov-724): a `swift`
    # command that leaves an entry there fails, and one that removes it does not.
    folder = system_temp()
    if sys.platform == "darwin" and not folder:
        failures.append("the system temporary folder could not be found on macOS, so the Swift arm is untested")
    if folder:
        fake = os.path.join(scratch, "bin")
        os.makedirs(fake)
        leftover = os.path.join(folder, f"fc-leak-selftest-{os.getpid()}")
        for name, body, want in (
            ("a Swift run that leaves a folder in the system temp folder", f"mkdir {leftover}", 1),
            ("a Swift run that removes what it made there", f"mkdir {leftover} && rmdir {leftover}", 0),
        ):
            with open(os.path.join(fake, "swift"), "w") as f:
                f.write("#!/bin/sh\n" + body + "\n")
            os.chmod(os.path.join(fake, "swift"), 0o755)
            os.environ["FC_LEAK_STRICT"] = "1"
            rc = run([os.path.join(fake, "swift"), "test"], grace=0.5)
            del os.environ["FC_LEAK_STRICT"]
            if (rc != 0) != bool(want):
                failures.append(f"{name}: exit {rc}, wanted {'non-zero' if want else 'zero'}")
            shutil.rmtree(leftover, ignore_errors=True)
        # Off CI and not strict, a shared Mac only gets a note.
        with open(os.path.join(fake, "swift"), "w") as f:
            f.write(f"#!/bin/sh\nmkdir {leftover}\n")
        saved = {k: os.environ.pop(k, None) for k in ("CI", "FC_LEAK_STRICT")}
        rc = run([os.path.join(fake, "swift"), "test"], grace=0.5)
        for k, v in saved.items():
            if v is not None:
                os.environ[k] = v
        shutil.rmtree(leftover, ignore_errors=True)
        if rc != 0:
            failures.append("a Swift run's new system temp entry failed a shared Mac's run")

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
    # The wiring: a tree with none of the wrapped runs is red, and so is a tree
    # where one line lost the wrapper.
    if not wiring(os.path.join(scratch, "tree")):
        failures.append("the wiring check passed a tree with no wrapped runs")
    # A wrapped run that is only a comment is no run, and a profile is fine.
    wired = os.path.join(scratch, "wired")
    os.makedirs(os.path.join(wired, ".github", "workflows"))
    os.makedirs(os.path.join(wired, "apps", "macos"))
    with open(os.path.join(wired, "apps", "macos", "test.sh"), "w") as f:
        f.write("./scripts/test-leak-check.py -- swift test\n")
    with open(os.path.join(wired, ".github", "workflows", "nightly.yml"), "w") as f:
        f.write("test-leak-check.py -- swift test --package-path apps/shared/AgentKit --filter AgentRowBudgetTests\n"
                "test-leak-check.py -- swift test --filter 'ConversationScrollTests/(a|b)'\n"
                "test-leak-check.py -- swift test --filter 'TranscriptJumpTests'\n"
                "test-leak-check.py -- swift test --filter 'NativeStreamingStallTests'\n")
    for body, red in (
        ("      # - run: ./scripts/test-leak-check.py -- cargo nextest run --workspace\n", True),
        ("      - run: ./scripts/test-leak-check.py -- cargo nextest run --profile ci --workspace\n", False),
    ):
        with open(os.path.join(wired, ".github", "workflows", "ci.yml"), "w") as f:
            f.write(body + "      - run: ./scripts/test-leak-check.py -- swift test --package-path apps/shared/AgentKit\n")
        if bool(wiring(wired)) != red:
            failures.append(f"the wiring check read {body.strip()!r} wrong")

    # The Rust arm (ov-552). Test code by its place, and not other code; a
    # comment, an exemption and a name that only looks alike pass.
    rust = os.path.join(scratch, "rust")
    for rel, body, red in (
        ("crates/x/tests/t.rs", "let d = std::env::temp_dir();\n", True),
        ("crates/x/src/a_tests.rs", "let d = std::env::temp_dir ( );\n", True),
        ("crates/x/src/tests.rs", "let d = temp_dir().join(\"x\");\n", True),
        ("crates/x/src/lib.rs", "fn f() { let d = std::env::temp_dir(); }\n", False),
        ("crates/x/src/lib.rs", "#[cfg(test)]\nmod tests {\n    fn f() { let d = std::env::temp_dir(); }\n}\n", True),
        ("crates/x/src/lib.rs", "fn f() { let d = std::env::temp_dir(); }\n#[cfg(test)]\nmod tests {\n}\n", False),
        ("crates/x/src/lib.rs", "#[cfg(test)]\nmod tests {\n    // never temp_dir() here\n    /* temp_dir() */\n}\n", False),
        ("crates/x/src/lib.rs", "#[cfg(test)]\nmod tests {\n    // leak-exempt: kept on purpose\n    fn f() { let d = std::env::temp_dir(); }\n}\n", False),
        ("crates/x/src/lib.rs", "#[cfg(test)]\nmod tests {\n    fn f() { let d = std::env::temp_dir(); } // leak-exempt: why\n}\n", False),
        ("crates/x/src/lib.rs", "#[cfg(test)]\nmod tests {\n    fn f() { let d = tempfile::tempdir(); let e = my_temp_dir_name(); }\n}\n", False),
        ("crates/x/examples/e.rs", "let d = std::env::temp_dir();\n", False),
    ):
        shutil.rmtree(rust, ignore_errors=True)
        os.makedirs(os.path.dirname(os.path.join(rust, rel)))
        with open(os.path.join(rust, rel), "w") as f:
            f.write(body)
        if bool(lint_rust(rust)) != red:
            failures.append(f"the Rust lint read {rel} {body!r} wrong (wanted {'red' if red else 'green'})")

    # The wiring, mutated in the real text (ov-552): each real run that must go
    # through this script loses its wrapper in a copy of the real file, and the
    # check must go red; the copy as it is must stay green. A hand-written
    # stand-in could drift from the workflow and prove nothing.
    here = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
    for path, pattern, what in WIRING:
        try:
            with open(os.path.join(here, path), encoding="utf-8") as f:
                real = f.read()
        except OSError:
            failures.append(f"{path} is not there to mutate")
            continue
        lines = real.split("\n")
        hits = [i for i, line in enumerate(lines) if pattern.search(line) and not line.lstrip().startswith("#")]
        if not hits:
            failures.append(f"{path} has no line for {what}")
            continue
        mutated = os.path.join(scratch, "mutated")
        for how, change in (
            ("without its wrapper", lambda l: l.replace("./scripts/test-leak-check.py -- ", "")),
            ("commented out", lambda l: re.sub(r"^(\s*)", r"\1# ", l)),
            ("with only a trailing comment", lambda l: "true # " + l.strip()),
        ):
            shutil.rmtree(mutated, ignore_errors=True)
            for other, _p, _w in WIRING:
                os.makedirs(os.path.dirname(os.path.join(mutated, other)), exist_ok=True)
                shutil.copyfile(os.path.join(here, other), os.path.join(mutated, other))
            copy = lines[:]
            for i in hits:
                copy[i] = change(copy[i])
            with open(os.path.join(mutated, path), "w", encoding="utf-8") as f:
                f.write("\n".join(copy))
            if not any(what in m for m in wiring(mutated)):
                failures.append(f"the wiring check missed {what} {how}, in the real {path}")
    shutil.rmtree(os.path.join(scratch, "mutated"), ignore_errors=True)
    for other, _p, _w in WIRING:
        os.makedirs(os.path.dirname(os.path.join(scratch, "mutated", other)), exist_ok=True)
        shutil.copyfile(os.path.join(here, other), os.path.join(scratch, "mutated", other))
    if wiring(os.path.join(scratch, "mutated")):
        failures.append("the wiring check flagged the real, unmutated workflow: " + "; ".join(wiring(os.path.join(scratch, "mutated"))))

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
