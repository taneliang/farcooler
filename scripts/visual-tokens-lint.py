#!/usr/bin/env python3
"""No new hand-drawn edge, fill, rule or glass in a Mac or AgentKit view (ov-216).

The Mac app grew ten corner radii, eleven grays, forty-two dividers and five
selection looks because every surface solved its own edge. The tokens now live
in apps/shared/AgentKit/Sources/AgentKit/DesignTokens.swift (and anything under
a `Style/` folder): `Radius`, `.control` / `.card` / `.floating`, `Surface`,
`Separator`, `Fill`, `Tint`. This scans apps/macos/Sources and
apps/shared/AgentKit/Sources, outside those files, for seven kinds of site:

  radius     a literal `cornerRadius:` (`RoundedRectangle(cornerRadius: 8)`,
             `.rect(cornerRadius: 20)`, `var cornerRadius: CGFloat = 20`)
  material   `.regularMaterial` and its siblings, `NSVisualEffectView`
  glass      `.glassEffect(`, `GlassEffectContainer`
  separator  `Divider()`, a one-point `.frame(height: 1)` / `.frame(width: 1)`
  stroke     `.strokeBorder(`, `.stroke(`
  shadow     `.shadow(`
  fill       `.primary.opacity(`, `.accentColor.opacity(`, `.secondary.opacity(`,
             `.black/.white/.gray.opacity(`, `Color(white:`, and
             `GlancePalette.<ink>(…).opacity(`

A site that is right on purpose carries `// style-exempt: <reason>` on its line
or the line above (a menu's `Divider()` is `// style-exempt: menu`).

Sites that predate the tokens are grandfathered by COUNT, per file and rule, in
scripts/visual-tokens-baseline.json, which this script generates. Counts, not
line numbers, so an unrelated edit doesn't churn it. It is a ratchet:

  - a file with more hits of a rule than its baseline fails (a new site);
  - a file with fewer fails too, until the baseline is lowered in the same
    commit (run --write-baseline, review the diff, and it can only have shrunk);
  - a baseline entry for a file that no longer exists fails.

A surface lane that converts a file lowers its counts; when the baseline is
empty the last lane deletes it and the scan requires zero.

  ./scripts/visual-tokens-lint.py                  scan the tree; exit 1 on a new site
  ./scripts/visual-tokens-lint.py --write-baseline regenerate the baseline from the tree
  ./scripts/visual-tokens-lint.py --self-test      check the rules against known cases
"""

import collections
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
BASELINE = ROOT / "scripts" / "visual-tokens-baseline.json"

SWIFT_ROOTS = ["apps/macos/Sources", "apps/shared/AgentKit/Sources"]
# Where the tokens are defined, so the only place a radius, a material or a gray
# may be spelled out.
TOKEN_FILES = {"DesignTokens.swift"}
TOKEN_DIRS = {"Style"}
# A scan that finds fewer files than this is looking at the wrong tree.
MIN_FILES = 100

MATERIALS = r"(?:ultraThin|thin|regular|thick|ultraThick)Material"
INK = r"(?:primary|secondary|accentColor|black|white|gray)"
RULES: dict[str, re.Pattern] = {
    "radius": re.compile(
        r"cornerRadius\s*:\s*-?\d"
        r"|cornerRadius\s*:\s*CGFloat\s*=\s*-?\d"
        r"|\bcornerRadius\s*=\s*-?\d"),
    "material": re.compile(rf"\.{MATERIALS}\b|\bNSVisualEffectView\b|\.background\(\s*\.bar\b"),
    "glass": re.compile(r"\.glassEffect\(|\bGlassEffectContainer\b"),
    "separator": re.compile(r"\bDivider\(\)|\.frame\(\s*(?:height|width)\s*:\s*(?:1|0\.5)\s*[,)]"),
    "stroke": re.compile(r"\.strokeBorder\(|\.stroke\("),
    "shadow": re.compile(r"\.shadow\("),
    "fill": re.compile(
        rf"\.{INK}\.opacity\(|\bColor\(white:|\bGlancePalette\.\w+\([^()]*\)\.opacity\("),
}
EXEMPT = re.compile(r"//\s*style-exempt\s*:\s*\S")


def code_of(line: str) -> str:
    """The line without its trailing `//` comment, and with the inside of its
    string literals blanked, so a word in a string or a comment isn't a site."""
    out, in_string, i = [], False, 0
    while i < len(line):
        c = line[i]
        if in_string:
            if c == "\\":
                i += 1
            elif c == '"':
                in_string = False
                out.append(c)
        elif c == '"':
            in_string = True
            out.append(c)
        elif line.startswith("//", i):
            break
        else:
            out.append(c)
        i += 1
    return "".join(out)


def hits(text: str) -> collections.Counter:
    """Rule -> count of unexempt sites in one file's text."""
    counts: collections.Counter = collections.Counter()
    lines = text.splitlines()
    in_block = False
    for index, raw in enumerate(lines):
        stripped = raw.strip()
        if in_block:
            in_block = "*/" not in stripped
            continue
        if stripped.startswith("/*") and "*/" not in stripped:
            in_block = True
            continue
        if stripped.startswith(("//", "*", "/*")):
            continue
        code = code_of(raw)
        exempt = EXEMPT.search(raw) or (index > 0 and EXEMPT.search(lines[index - 1])
                                        and lines[index - 1].strip().startswith("//"))
        if exempt:
            continue
        for rule, pattern in RULES.items():
            counts[rule] += len(pattern.findall(code))
    return +counts


def files():
    for root in SWIFT_ROOTS:
        for path in sorted((ROOT / root).rglob("*.swift")):
            rel = path.relative_to(ROOT)
            if path.name in TOKEN_FILES or TOKEN_DIRS & set(rel.parts):
                continue
            yield path


def tree_counts() -> tuple[dict[str, dict[str, int]], int]:
    out, scanned = {}, 0
    for path in files():
        scanned += 1
        counts = hits(path.read_text(encoding="utf-8"))
        if counts:
            out[str(path.relative_to(ROOT))] = dict(sorted(counts.items()))
    return out, scanned


def load_baseline() -> dict[str, dict[str, int]]:
    if not BASELINE.exists():
        return {}
    return json.loads(BASELINE.read_text(encoding="utf-8"))


def write_baseline(counts: dict[str, dict[str, int]]) -> None:
    # One line per file, so two lanes lowering different files never conflict.
    body = ",\n".join(f"  {json.dumps(f)}: {json.dumps(c, sort_keys=True)}"
                      for f, c in sorted(counts.items()))
    BASELINE.write_text("{\n" + body + "\n}\n" if counts else "{}\n", encoding="utf-8")


def compare(tree: dict, baseline: dict, exists=lambda f: True) -> list[str]:
    """Problems between the tree's counts and the baseline's."""
    problems = []
    for f in sorted(set(tree) | set(baseline)):
        if f in baseline and not exists(f):
            problems.append(f"{f}: in the baseline but no longer in the tree; remove it "
                            "(run --write-baseline)")
            continue
        for rule in RULES:
            have, allowed = tree.get(f, {}).get(rule, 0), baseline.get(f, {}).get(rule, 0)
            if have > allowed:
                problems.append(
                    f"{f}: {have - allowed} new {rule} site(s) ({have}, baseline {allowed}). "
                    "Use the tokens in DesignTokens.swift, or mark a deliberate one "
                    "`// style-exempt: <reason>`.")
            elif have < allowed:
                problems.append(
                    f"{f}: {rule} is down to {have} (baseline {allowed}). Lower the baseline "
                    "in this commit: run ./scripts/visual-tokens-lint.py --write-baseline.")
    return problems


def scan() -> int:
    tree, scanned = tree_counts()
    if scanned < MIN_FILES:
        print(f"visual-tokens-lint: scanned only {scanned} files; the roots are wrong")
        return 1
    problems = compare(tree, load_baseline(), lambda f: (ROOT / f).exists())
    for line in problems:
        print(line)
    if problems:
        print(f"\n{len(problems)} problem(s). Design: .claude/agent/reports/ov-216/design.md")
        return 1
    total = sum(sum(c.values()) for c in tree.values())
    print(f"visual-tokens-lint: no new sites ({total} grandfathered in {len(tree)} files, "
          f"{scanned} scanned)")
    return 0


def self_test() -> int:
    red = [  # (rule, one line that must be caught)
        ("radius", "RoundedRectangle(cornerRadius: 7)"),
        ("radius", ".clipShape(.rect(cornerRadius: 12))"),
        ("radius", "var cornerRadius: CGFloat = 20"),
        ("material", ".background(.regularMaterial, in: .card)"),
        ("material", ".fill(.ultraThinMaterial)"),
        ("material", "let v = NSVisualEffectView()"),
        ("glass", "content.glassEffect(.regular, in: .card)"),
        ("glass", "GlassEffectContainer(spacing: 8) {"),
        ("separator", "Divider()"),
        ("separator", "Rectangle().fill(.quaternary).frame(height: 1)"),
        ("separator", ".frame(width: 1)"),
        ("stroke", ".strokeBorder(.quaternary)"),
        ("stroke", ".stroke(Color.red, lineWidth: 1)"),
        ("shadow", ".shadow(radius: 8)"),
        ("fill", ".background(Color.primary.opacity(0.07))"),
        ("fill", ".fill(.primary.opacity(0.05))"),
        ("fill", ".background(Color.accentColor.opacity(0.16))"),
        ("fill", ".fill(GlancePalette.amber(scheme).opacity(0.08))"),
    ]
    green = [  # lines that must not be caught
        ".clipShape(.card)",
        "RoundedRectangle(cornerRadius: Radius.small, style: .continuous)",
        "// RoundedRectangle(cornerRadius: 7) was the old look",
        "/// A Divider() would draw a rule here.",
        "let note = \"Divider() is a word here\" // Divider()",
        ".background(Fill.inset(contrast), in: .control)",
        ".frame(height: 12)",
        ".foregroundStyle(.primary)",
        ".fill(.separator)",
        "Divider() // style-exempt: menu",
        ".background(.thickMaterial) // style-exempt: sheet scrim",
    ]
    failed = 0
    for rule, line in red:
        if not hits(line).get(rule):
            print(f"self-test: {rule} missed: {line!r}")
            failed += 1
    for line in green:
        if hits(line):
            print(f"self-test: wrongly caught {dict(hits(line))}: {line!r}")
            failed += 1
    if hits("// style-exempt: menu\nDivider()"):
        print("self-test: an exempt comment on the line above did not exempt")
        failed += 1
    if not hits("// not an exemption\nDivider()"):
        print("self-test: a plain comment above exempted a site")
        failed += 1
    if not hits("/* old\n Divider()\n*/\nDivider()").get("separator") == 1:
        print("self-test: a block comment was scanned")
        failed += 1
    # The ratchet: more fails, fewer fails, the same passes, a gone file fails.
    base = {"A.swift": {"radius": 2}}
    for label, tree, bl, expect in [
        ("a new site", {"A.swift": {"radius": 3}}, base, 1),
        ("a new file", {"A.swift": {"radius": 2}, "B.swift": {"fill": 1}}, base, 1),
        ("a site fixed without lowering the baseline", {"A.swift": {"radius": 1}}, base, 1),
        ("the same", {"A.swift": {"radius": 2}}, base, 0),
        ("all fixed and baseline emptied", {}, {}, 0),
    ]:
        got = len(compare(tree, bl))
        if got != expect:
            print(f"self-test: ratchet, {label}: {got} problem(s), wanted {expect}")
            failed += 1
    if not compare({}, base, exists=lambda f: False):
        print("self-test: a baseline entry for a missing file passed")
        failed += 1
    # The real scan must see the real tree, or a broken root would pass green.
    if sum(1 for _ in files()) < MIN_FILES:
        print("self-test: the scan sees fewer than 100 files")
        failed += 1
    print("visual-tokens-lint self-test: " + ("ok" if not failed else f"{failed} failed"))
    return 1 if failed else 0


if __name__ == "__main__":
    if "--self-test" in sys.argv[1:]:
        sys.exit(self_test())
    if "--write-baseline" in sys.argv[1:]:
        counts, _ = tree_counts()
        write_baseline(counts)
        print(f"wrote {BASELINE.relative_to(ROOT)}: {sum(sum(c.values()) for c in counts.values())} "
              f"sites in {len(counts)} files")
        sys.exit(0)
    sys.exit(scan())
