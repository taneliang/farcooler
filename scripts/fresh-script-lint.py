#!/usr/bin/env python3
"""No Mac or AgentKit test makes a file executable to run it (ov-566).

macOS assesses a file the first time it is executed. For a script written a
moment ago that is a check shared by the whole machine, one at a time: 60
launches at once, on a machine at a load of 30, took up to 36 s each (ov-551),
and a test with a 20 s deadline failed for it. The same scripts read by
`/bin/sh` took 2 s at the worst. So a stand-in script is run as
`/bin/sh script args...` and is never marked executable.

This fails on a line in a Swift test that gives a file an executable mode:

  - `.posixPermissions: 0o755` (any mode with an execute bit), or `mode: 0o755`;
  - `chmod` or `+x`;
  - `S_IXUSR`.

Where the program is the thing under test (an `ssh` the app is handed by path,
a link whose real path the app checks), the site carries
`// fresh-script-exempt: <reason>` on its line or the line above.

  ./scripts/fresh-script-lint.py              scan the tests; exit 1 on a hit
  ./scripts/fresh-script-lint.py --self-test  check the rules against known cases
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
# (root, fewest files a scan of it may find). A root that finds fewer is the wrong tree.
SOURCES = [("apps/macos/Tests", 30), ("apps/shared/AgentKit/Tests", 5)]

MODE = re.compile(r"\b(posixPermissions|mode)\s*:\s*0o(?=[0-7]*[1357])[0-7]+\b")
OTHER = re.compile(r"\bchmod\b|\+x\b|\bS_IX(USR|GRP|OTH)\b")
EXEMPT = re.compile(r"//\s*fresh-script-exempt:\s*\S")
EMPTY_EXEMPT = re.compile(r"//\s*fresh-script-exempt:\s*$")
COMMENT = re.compile(r"^\s*//")


def problems_in(text: str) -> list[tuple[int, str]]:
    """(line number, what is wrong) for one file's text."""
    lines = text.splitlines()
    out = []
    for number, line in enumerate(lines, 1):
        if EMPTY_EXEMPT.search(line):
            out.append((number, "`// fresh-script-exempt:` needs a reason"))
            continue
        if COMMENT.match(line) or not (MODE.search(line) or OTHER.search(line.split("//")[0])):
            continue
        before = lines[number - 2] if number > 1 else ""
        if EXEMPT.search(line) or (COMMENT.match(before) and EXEMPT.search(before)):
            continue
        out.append((number, "a test makes a file executable; run the script as `/bin/sh script args` "
                            "(CLI.run's `interpreter:`), or say why with `// fresh-script-exempt: <reason>`"))
    return out


def files() -> list[pathlib.Path]:
    return sorted(p for root, _ in SOURCES for p in (ROOT / root).rglob("*.swift"))


def scan() -> int:
    seen = files()
    for root, fewest in SOURCES:
        found = sum(1 for p in seen if str(p.relative_to(ROOT)).startswith(root + "/"))
        if found < fewest:
            print(f"fresh-script-lint: {root} has only {found} files; the root is wrong")
            return 1
    bad = 0
    for p in seen:
        for number, why in problems_in(p.read_text(encoding="utf-8")):
            print(f"{p.relative_to(ROOT)}:{number}: {why}")
            bad += 1
    if bad:
        print(f"\n{bad} problem(s). A script written a moment ago stalls under load when it is executed (ov-551).")
        return 1
    print(f"fresh-script-lint: ok ({len(seen)} test files scanned)")
    return 0


def self_test() -> int:
    red = [
        "try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: path)",
        "try write(body, path, mode: 0o755)",
        "try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: p)",
        "let ran = await ProcessRunner.run(\"/bin/chmod\", [\"+x\", script])",
        "chmod(path, 0o755)",
        "mode |= S_IXUSR",
        "try x() // fresh-script-exempt:",
    ]
    green = [
        "try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path)",
        "try write(body, path, mode: 0o644)",
        "#expect(mode == 0o700)",
        "// setAttributes([.posixPermissions: 0o755]) used to make it a program",
        "try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: p) // fresh-script-exempt: ssh is handed by path",
        "let c = x + 1  // c++ and 0o755 in a comment",
    ]
    failed = 0
    for text in red:
        if not problems_in(text):
            print(f"self-test: missed: {text!r}")
            failed += 1
    for text in green:
        if problems_in(text):
            print(f"self-test: wrongly caught {problems_in(text)}: {text!r}")
            failed += 1
    above = "// fresh-script-exempt: the app checks this path\ntry setAttributes([.posixPermissions: 0o755], ofItemAtPath: p)"
    if problems_in(above):
        print("self-test: an exemption on the line above was not honored")
        failed += 1
    for root, fewest in SOURCES:
        if sum(1 for p in files() if str(p.relative_to(ROOT)).startswith(root + "/")) < fewest:
            print(f"self-test: the scan sees fewer than {fewest} files under {root}")
            failed += 1
    print("fresh-script-lint self-test: " + ("ok" if not failed else f"{failed} failed"))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(self_test() if "--self-test" in sys.argv[1:] else scan())
