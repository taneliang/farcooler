#!/usr/bin/env python3
"""Mac and AgentKit tests never spin a run loop (ov-606).

A synchronous test on the main actor that runs `RunLoop.main` or
`RunLoop.current` never gives the main actor back, so every other main-actor
test in the run waits behind it (ov-605 turned main red this way, Train 170).
In a Mac or AgentKit test, this fails on any call to:

  - `RunLoop.main.run(`, `RunLoop.current.run(`;
  - `CFRunLoopRun(`, `CFRunLoopRunInMode(`;

Make the test `async` and wait with `await Settle.until { condition }`, or
`try? await Task.sleep(for:)` for one turn. A site that is right on purpose
carries `// runloop-exempt: <reason>` on its line.

  ./scripts/runloop-spin-lint.py              scan the tests; exit 1 on a hit
  ./scripts/runloop-spin-lint.py --self-test  check the rules against known cases
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
# (root, fewest files a scan of it may find). A root that finds fewer is the wrong tree.
SOURCES = [("apps/macos/Tests", 30), ("apps/shared/AgentKit/Tests", 5)]

SPINS = re.compile(r"\bRunLoop\.(main|current)\.run\b|\bCFRunLoopRun(InMode)?\s*\(")
EXEMPT = re.compile(r"//\s*runloop-exempt:\s*\S")
EMPTY_EXEMPT = re.compile(r"//\s*runloop-exempt:\s*$")
COMMENT = re.compile(r"^\s*//")


def problems_in(text: str) -> list[tuple[int, str]]:
    """(line number, what is wrong) for one file's text."""
    out = []
    for number, line in enumerate(text.splitlines(), 1):
        if EMPTY_EXEMPT.search(line):
            out.append((number, "`// runloop-exempt:` needs a reason"))
        elif EXEMPT.search(line) or COMMENT.match(line):
            continue
        elif SPINS.search(line):
            out.append((number, "a test spins a run loop, which holds the main actor; "
                                "make it `async` and use `await Settle.until { }`"))
    return out


def files() -> list[pathlib.Path]:
    return sorted(p for root, _ in SOURCES for p in (ROOT / root).rglob("*.swift"))


def scan() -> int:
    seen = files()
    for root, fewest in SOURCES:
        found = sum(1 for p in seen if str(p.relative_to(ROOT)).startswith(root + "/"))
        if found < fewest:
            print(f"runloop-spin-lint: {root} has only {found} files; the root is wrong")
            return 1
    bad = 0
    for p in seen:
        for number, why in problems_in(p.read_text(encoding="utf-8")):
            print(f"{p.relative_to(ROOT)}:{number}: {why}")
            bad += 1
    if bad:
        print(f"\n{bad} problem(s). A run-loop spin starves every other main-actor test; see SettleHelpers.swift.")
        return 1
    print(f"runloop-spin-lint: ok ({len(seen)} test files scanned)")
    return 0


def self_test() -> int:
    red = [
        "RunLoop.current.run(until: Date().addingTimeInterval(0.004))",
        "        RunLoop.main.run(until: .distantFuture)",
        "RunLoop.main.run(mode: .default, before: Date())",
        "CFRunLoopRun()",
        "CFRunLoopRunInMode(.defaultMode, 0.1, false)",
        "RunLoop.current.run(until: d) // runloop-exempt:",
    ]
    green = [
        "await Settle.until { ready }",
        "RunLoop.main.perform(inModes: [.common]) { }",  # schedules, does not spin
        "try? await Task.sleep(for: .milliseconds(4))",
        "// the old loop was RunLoop.current.run(until: d)",
        "/// CFRunLoopRun() never returned the main actor",
        "RunLoop.current.run(until: d)  // runloop-exempt: a nonisolated helper on its own thread",
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
    for root, fewest in SOURCES:
        if sum(1 for p in files() if str(p.relative_to(ROOT)).startswith(root + "/")) < fewest:
            print(f"self-test: the scan sees fewer than {fewest} files under {root}")
            failed += 1
    print("runloop-spin-lint self-test: " + ("ok" if not failed else f"{failed} failed"))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(self_test() if "--self-test" in sys.argv[1:] else scan())
