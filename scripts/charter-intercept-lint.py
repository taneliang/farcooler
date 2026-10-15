#!/usr/bin/env python3
"""Every Start Orchestrator on the Mac goes through the charter's screens
(ov-519 phase 2b, review R3).

`ContentView.startOrchestrator` opens the Reading and Charter screens for a
workspace with no charter; `startOrchestratorNow` starts straight away, past
them. Only the start itself (ContentView+Workspaces.swift) and the screens'
own Start (ContentView+Charter.swift) may call `startOrchestratorNow(`: a menu,
a row or an empty state that called it would start an orchestrator that
interviews from nothing, and no test of one call site would notice another.

  ./scripts/charter-intercept-lint.py              scan the Mac app; exit 1 on a hit
  ./scripts/charter-intercept-lint.py --self-test  check the rule against known cases
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SOURCES = ROOT / "apps/macos/Sources"
ALLOWED = {"ContentView+Workspaces.swift", "ContentView+Charter.swift"}
CALL = re.compile(r"\bstartOrchestratorNow\(")
COMMENT = re.compile(r"^\s*(//|///|\*)")


def hits(name: str, text: str) -> list[int]:
    """Line numbers in `name` that call `startOrchestratorNow(` where they may not."""
    if name in ALLOWED:
        return []
    return [n for n, line in enumerate(text.splitlines(), 1) if CALL.search(line) and not COMMENT.match(line)]


def self_test() -> int:
    cases = [
        ("ContentView+Toolbar.swift", "onStart: { startOrchestratorNow(ws, host: h, harness: $0, replace: false) }", [1]),
        ("ContentView+Toolbar.swift", "onStart: { startOrchestrator(ws, host: h, harness: $0, replace: false) }", []),
        ("ContentView+Toolbar.swift", "/// `startOrchestratorNow(` starts past the screens", []),
        ("ContentView+Charter.swift", "startOrchestratorNow(workspace, host: host, harness: harness, replace: false)", []),
        ("ContentView+Workspaces.swift", "func startOrchestratorNow(", []),
    ]
    bad = [(name, text, want, hits(name, text)) for name, text, want in cases if hits(name, text) != want]
    for name, text, want, got in bad:
        print(f"self-test: {name}: {text!r}: wanted {want}, got {got}")
    print("charter-intercept-lint: self-test " + ("failed" if bad else "ok"))
    return 1 if bad else 0


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()
    found = []
    for path in sorted(SOURCES.rglob("*.swift")):
        for line in hits(path.name, path.read_text()):
            found.append(f"{path.relative_to(ROOT)}:{line}")
    for place in found:
        print(f"{place}: calls startOrchestratorNow(, which skips the charter's screens; call startOrchestrator(")
    print(f"charter-intercept-lint: {'FAIL' if found else 'ok'}, {len(found)} call(s) outside the start and the screens")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
