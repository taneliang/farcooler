#!/usr/bin/env python3
"""Mac tests show windows only through TestWindow.show (ov-473).

A test that orders a window front itself puts it over the person's work, and
`NSApp.activate` would take the frontmost spot from their app. In a Mac or
AgentKit test, this fails on any call to:

  - `makeKeyAndOrderFront(`, `orderFront(`, `orderFrontRegardless(`;
  - `NSApp.activate(` or `.activate(ignoringOtherApps:`;

outside apps/macos/Tests/TestWindows, the one place that shows a window. Use
`TestWindow.show(window)`: behind every app's windows, still on screen, so it
keeps its display ticks. A site that is right on purpose carries
`// window-exempt: <reason>` on its line, and a closure that only names the
call to restore a default (`activateApp = { NSApp.activate() }`) is one.

  ./scripts/test-window-lint.py              scan the tests; exit 1 on a hit
  ./scripts/test-window-lint.py --self-test  check the rules against known cases
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
# (root, fewest files a scan of it may find). A root that finds fewer is the wrong tree.
SOURCES = [("apps/macos/Tests", 30), ("apps/shared/AgentKit/Tests", 5)]
HELPER = "apps/macos/Tests/TestWindows/"

SHOWS = re.compile(
    r"\.(makeKeyAndOrderFront|orderFrontRegardless|orderFront)\(|\bNSApp\.activate\(|\.activate\(ignoringOtherApps:")
EXEMPT = re.compile(r"//\s*window-exempt:\s*\S")
EMPTY_EXEMPT = re.compile(r"//\s*window-exempt:\s*$")
COMMENT = re.compile(r"^\s*//")


def problems_in(text: str) -> list[tuple[int, str]]:
    """(line number, what is wrong) for one file's text."""
    out = []
    for number, line in enumerate(text.splitlines(), 1):
        if EMPTY_EXEMPT.search(line):
            out.append((number, "`// window-exempt:` needs a reason"))
        elif EXEMPT.search(line) or COMMENT.match(line):
            continue
        elif SHOWS.search(line):
            out.append((number, "a test shows a window or activates the app itself; "
                                "use `TestWindow.show(window)`"))
    return out


def files() -> list[pathlib.Path]:
    return sorted(p for root, _ in SOURCES for p in (ROOT / root).rglob("*.swift"))


def scan() -> int:
    seen = files()
    for root, fewest in SOURCES:
        found = sum(1 for p in seen if str(p.relative_to(ROOT)).startswith(root + "/"))
        if found < fewest:
            print(f"test-window-lint: {root} has only {found} files; the root is wrong")
            return 1
    bad = 0
    for p in seen:
        if str(p.relative_to(ROOT)).startswith(HELPER):
            continue
        for number, why in problems_in(p.read_text(encoding="utf-8")):
            print(f"{p.relative_to(ROOT)}:{number}: {why}")
            bad += 1
    if bad:
        print(f"\n{bad} problem(s). Test windows stay behind the person's work; see TestWindow.swift.")
        return 1
    print(f"test-window-lint: ok ({len(seen)} test files scanned)")
    return 0


def self_test() -> int:
    red = [
        "window.makeKeyAndOrderFront(nil)",
        "drawn.window.orderFrontRegardless()",
        "shown.orderFront(nil)",
        "NSApp.activate()",
        "NSApplication.shared.activate(ignoringOtherApps: true)",
        "window.orderFront(nil) // window-exempt:",
    ]
    green = [
        "TestWindow.show(window)",
        "window.orderBack(nil)",
        "window.orderOut(nil)",
        "// tests used to call makeKeyAndOrderFront(nil)",
        "/// window.orderFrontRegardless() put it over the person's work",
        "opener.activateApp = { NSApp.activate() }  // window-exempt: restores the default; never called",
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
    print("test-window-lint self-test: " + ("ok" if not failed else f"{failed} failed"))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(self_test() if "--self-test" in sys.argv[1:] else scan())
