#!/usr/bin/env python3
"""A Mac capture names its state before the shot (ov-642).

A capture that grabs a window's pixels with a fixed settle in front of it
shoots whatever the window happens to show after that long. New captures go
through `Capture.keep` / `Capture.wait` (CaptureState.swift), which wait for a
state and fail without writing. This fails on a direct call to:

  - `RealWindowCaptures.windowImage(`;
  - `settle(window` (`NativeAgentTests.settle(window, 600)` and its kin);

in a Mac test file that has more of them than `scripts/capture-window-allow.txt`
says (`<path> <count>` per line: today's sites). A call that is right on
purpose carries `// capture-exempt: <reason>` on its line. Lower a count when
a file is converted; the lint fails when a count is higher than the file needs,
so the list only ever shrinks.

  ./scripts/capture-window-lint.py              scan the tests; exit 1 on a hit
  ./scripts/capture-window-lint.py --self-test  check the rules against known cases
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
TESTS = "apps/macos/Tests"
ALLOW = ROOT / "scripts/capture-window-allow.txt"

DIRECT = re.compile(r"RealWindowCaptures\.windowImage\(|\bsettle\(window\b")
EXEMPT = re.compile(r"//\s*capture-exempt:\s*\S")
COMMENT = re.compile(r"^\s*//")


def count_in(text: str) -> int:
    return sum(
        1 for line in text.splitlines()
        if DIRECT.search(line) and not EXEMPT.search(line) and not COMMENT.match(line))


def counts() -> dict[str, int]:
    out = {}
    for p in sorted((ROOT / TESTS).rglob("*.swift")):
        n = count_in(p.read_text())
        if n:
            out[str(p.relative_to(ROOT))] = n
    return out


def allowed() -> dict[str, int]:
    out = {}
    for line in ALLOW.read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            path, n = line.rsplit(None, 1)
            out[path] = int(n)
    return out


def scan() -> int:
    have, ok = counts(), allowed()
    bad = 0
    for path in sorted(set(have) | set(ok)):
        h, a = have.get(path, 0), ok.get(path, 0)
        if h > a:
            print(f"capture-window-lint: {path} has {h} direct window captures or settles, {a} allowed; "
                  "use Capture.keep with a state (CaptureState.swift)")
            bad += 1
        elif h < a:
            print(f"capture-window-lint: {path} is allowed {a} but has {h}; lower it in {ALLOW.name}")
            bad += 1
    print(f"capture-window-lint: {'ok' if not bad else 'FAILED'} ({sum(have.values())} sites in {len(have)} files)")
    return 1 if bad else 0


def self_test() -> int:
    cases = [
        ("let i = RealWindowCaptures.windowImage(window)", 1),
        ("await NativeAgentTests.settle(window, 600)", 1),
        ("await T.settle(window, 300)", 1),
        ("// await settle(window, 1)", 0),
        ("settle(window, 1) // capture-exempt: a layout test", 0),
        ("await Settle.until { ready }", 0),
        ("Capture.keep(host, to: url, expecting: \"x\", until: { ok })", 0),
    ]
    bad = [(t, want, count_in(t)) for t, want in cases if count_in(t) != want]
    for t, want, got in bad:
        print(f"capture-window-lint self-test: {t!r}: wanted {want}, got {got}")
    print("capture-window-lint self-test:", "FAILED" if bad else "ok")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(self_test() if "--self-test" in sys.argv else scan())
