#!/usr/bin/env python3
"""No count in parentheses in any app's copy (ov-101).

The owner's rule, from ov-104: "parentheses are a plain-text habit". A count
beside a header goes in the header's trailing slot (`SectionCount` on the
Mac), tertiary, in tabular digits; a menu item's count goes in the menu's
badge; a sentence says "3 things need you". Never "Finished (14)".

This scans the user-facing sources of the Mac, the phones and the shared
Swift package for the shapes such a count takes:

  Swift   "Finished (\\(count))"        an interpolation alone in parentheses
  Kotlin  "Finished ($count)"           a template alone in parentheses
          "Finished (${items.size})"
  any     "Finished (%d)", "(%1$d)"     a format specifier alone in them

A line that matches and is not a count (a version's channel, a twin pane told
apart by its id) says so with `not a count` in a comment on the same line; one
that is a count but is never drawn (the review prompt written to an agent)
says `not UI copy`.

  ./scripts/copy-lint.py              scan the tree; exit 1 on any hit
  ./scripts/copy-lint.py --self-test  check the patterns against known cases
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# What's scanned: the apps' own sources, not their tests.
SWIFT_ROOTS = ["apps/macos/Sources", "apps/ios", "apps/shared/AgentKit/Sources"]
KOTLIN_ROOTS = ["apps/android/app/src/main"]
SKIP_PARTS = {"Tests", "FarCoolerUITests", "test", "androidTest", ".build", "build"}

SWIFT = re.compile(r"\(\\\([^()]*(?:\([^()]*\)[^()]*)*\)\)")
KOTLIN = re.compile(r"\(\$(?:\{[^{}]*\}|[A-Za-z_][A-Za-z0-9_.]*)\)")
FORMAT = re.compile(r"\(%(?:\d+\$)?(?:l{0,2}[dui]|@|s)\)")
# A line that matches without breaking the rule says why on the line: it isn't
# a count, or it isn't drawn (a prompt written to an agent).
ALLOWS = ("not a count", "not UI copy")


def code(line: str) -> str:
    """The line without a whole-line comment: a doc comment quoting the
    wrong way isn't the wrong way."""
    stripped = line.lstrip()
    if stripped.startswith(("//", "*", "/*", "<!--")):
        return ""
    return line


def hits(text: str, kind: str) -> list[tuple[int, str]]:
    patterns = {"swift": [SWIFT, FORMAT], "kotlin": [KOTLIN, FORMAT], "xml": [FORMAT]}[kind]
    out = []
    for number, line in enumerate(text.splitlines(), 1):
        if any(allow in line for allow in ALLOWS):
            continue
        body = code(line)
        # Only inside a string: a count is copy, and a call's parentheses
        # around an interpolation-free argument are not.
        if '"' not in body and kind != "xml":
            continue
        if any(p.search(body) for p in patterns):
            out.append((number, line.strip()))
    return out


def files():
    def walk(roots, suffixes):
        for root in roots:
            for path in sorted((ROOT / root).rglob("*")):
                if path.suffix not in suffixes or SKIP_PARTS & set(path.relative_to(ROOT).parts):
                    continue
                yield path

    for path in walk(SWIFT_ROOTS, {".swift"}):
        yield path, "swift"
    for path in walk(KOTLIN_ROOTS, {".kt"}):
        yield path, "kotlin"
    for path in walk(KOTLIN_ROOTS, {".xml"}):
        if path.parent.name.startswith("values"):
            yield path, "xml"


def scan() -> int:
    found = 0
    for path, kind in files():
        for number, line in hits(path.read_text(encoding="utf-8"), kind):
            print(f"{path.relative_to(ROOT)}:{number}: a count in parentheses: {line}")
            found += 1
    if found:
        print(
            f"\n{found} parenthesized count(s). Put a header's count in its trailing slot "
            "(SectionCount), a menu item's in its badge, or say it in words. "
            "If one isn't a count, or isn't drawn, say so in a comment on its line: "
            f"{' or '.join(f'`{a}`' for a in ALLOWS)}.")
        return 1
    print("copy-lint: no parenthesized counts")
    return 0


def self_test() -> int:
    cases = [
        # Each must be caught.
        ("swift", 'Text("Finished (\\(count))")', True),
        ("swift", 'Text("\\(title) (\\(items.count))")', True),
        ("swift", 'return "Needs You (\\(needs(you)))"', True),
        ("swift", 'String(format: "Unread (%d)", n)', True),
        ("kotlin", 'Text("Finished ($count)")', True),
        ("kotlin", 'Text("Tasks (${tasks.size})")', True),
        ("kotlin", 'stringResource(R.string.x, "(%1$d)")', True),
        ("xml", '<string name="unread">Unread (%d)</string>', True),
        # Each must pass.
        ("swift", 'Text("Finished")', False),
        ("swift", '"\\(marketing) (\\(channel))"  // not a count: the channel', False),
        ("swift", "/// never \"Finished (\\(n))\"", False),
        ("swift", "let x = f(\\(a))", False),
        ("swift", 'Text("\\(n) unchanged lines")', False),
        ("kotlin", 'Text("$count things need you")', False),
        ("kotlin", 'Text("Price (USD)")', False),
        ("xml", '<string name="x">%d things need you</string>', False),
    ]
    failed = 0
    for kind, line, caught in cases:
        if bool(hits(line, kind)) != caught:
            print(f"self-test: {'missed' if caught else 'wrongly caught'} {kind}: {line}")
            failed += 1
    print("copy-lint self-test: " + ("ok" if not failed else f"{failed} failed"))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(self_test() if "--self-test" in sys.argv[1:] else scan())
