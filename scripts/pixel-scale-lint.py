#!/usr/bin/env python3
"""Mac pixel tests hold at 1x (ov-280).

CI's Mac runner renders at 1x and every local Mac is 2x, so a test that samples
a bitmap with a hard-coded 2 passes here and goes red only on CI (the pulse dot,
NeedsYouLook and ov-235 on Oct 4). In a Mac or AgentKit test, this fails:

  - a `colorAt(` call whose arguments multiply by a literal 2 (`x * 2`, `2 * y`);
  - in a file that calls `colorAt(`, a scale constant set to a literal 2
    (`let scale = 2`, `scale: Int = 2.0`).

Derive the scale from the bitmap instead: `rep.pixelsWide / rep.size.width`.
A site that is right on purpose carries `// scale-exempt: <reason>` on its line.

  ./scripts/pixel-scale-lint.py              scan the tests; exit 1 on a hit
  ./scripts/pixel-scale-lint.py --self-test  check the rules against known cases
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
# (root, fewest files a scan of it may find). A root that finds fewer is the wrong tree.
SOURCES = [("apps/macos/Tests", 30), ("apps/shared/AgentKit/Tests", 5)]

SAMPLE = re.compile(r"colorAt\([^\n]*?(\*\s*2(?![\w.])|(?<![\w.])2\s*\*|<<\s*1(?!\d))")
SCALE = re.compile(r"\b\w*[sS]cale\w*\s*(?::\s*\w+\s*)?=\s*2(?:\.0*)?(?![\w.])")
EXEMPT = re.compile(r"//\s*scale-exempt:\s*\S")
EMPTY_EXEMPT = re.compile(r"//\s*scale-exempt:\s*$")


def problems_in(text: str) -> list[tuple[int, str]]:
    """(line number, what is wrong) for one file's text."""
    samples = "colorAt(" in text
    out = []
    for number, line in enumerate(text.splitlines(), 1):
        if EMPTY_EXEMPT.search(line):
            out.append((number, "`// scale-exempt:` needs a reason"))
        elif EXEMPT.search(line):
            continue
        elif SAMPLE.search(line):
            out.append((number, "colorAt samples at a hard-coded 2x; derive the scale from "
                                "`rep.pixelsWide / rep.size.width`"))
        elif samples and SCALE.search(line):
            out.append((number, "a pixel test hard-codes its scale to 2; derive it from "
                                "`rep.pixelsWide / rep.size.width`"))
    return out


def files() -> list[pathlib.Path]:
    return sorted(p for root, _ in SOURCES for p in (ROOT / root).rglob("*.swift"))


def scan() -> int:
    seen = files()
    for root, fewest in SOURCES:
        found = sum(1 for p in seen if str(p.relative_to(ROOT)).startswith(root + "/"))
        if found < fewest:
            print(f"pixel-scale-lint: {root} has only {found} files; the root is wrong")
            return 1
    bad = 0
    for p in seen:
        for number, why in problems_in(p.read_text(encoding="utf-8")):
            print(f"{p.relative_to(ROOT)}:{number}: {why}")
            bad += 1
    if bad:
        print(f"\n{bad} problem(s). CI renders at 1x; see ui-lane-common.md (Oct 4).")
        return 1
    print(f"pixel-scale-lint: ok ({len(seen)} test files scanned)")
    return 0


def self_test() -> int:
    red = [
        "let c = rep.colorAt(x: x * 2, y: y * 2)",
        "try #require(rep.colorAt(x: 2 * x, y: y))",
        "rep.colorAt(x: Int(p.x) * 2, y: 4)",
        "rep.colorAt(x: x << 1, y: y)",
        "let scale = 2\nlet c = rep.colorAt(x: x, y: y)",
        "let scale: CGFloat = 2.0\nrep.colorAt(x: 1, y: 1)",
        "rep.colorAt(x: x, y: y) // scale-exempt:",
    ]
    green = [
        "rep.colorAt(x: x * scale, y: y * scale)",
        "rep.colorAt(x: x * rep.pixelsWide / Int(rep.size.width), y: y)",
        "rep.colorAt(x: x, y: y * 20)",
        "rep.colorAt(x: x + 12, y: y)",
        "let scale = 2.0",  # no colorAt in the file: not a pixel test
        "let scale = max(1, rep.pixelsWide / Int(rep.size.width))\nrep.colorAt(x: 1, y: 1)",
        "rep.colorAt(x: x * 2, y: y) // scale-exempt: a fixture drawn at 2 pt per pixel",
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
    print("pixel-scale-lint self-test: " + ("ok" if not failed else f"{failed} failed"))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(self_test() if "--self-test" in sys.argv[1:] else scan())
