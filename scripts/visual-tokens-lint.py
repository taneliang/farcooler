#!/usr/bin/env python3
"""No new hand-drawn edge, fill, rule or glass in a Mac, iOS, Android or AgentKit view (ov-216, ov-256).

The Mac app grew ten corner radii, eleven grays, forty-two dividers and five
selection looks because every surface solved its own edge. The tokens now live
in apps/shared/AgentKit/Sources/AgentKit/DesignTokens.swift (and anything under
a `Style/` folder): `Radius`, `.control` / `.card` / `.floating`, `Surface`,
`Separator`, `Fill`, `Tint`. This scans apps/macos/Sources, apps/ios (Swift) and
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

Kotlin under apps/android/app/src/main is scanned for two of them, outside the
theme (Theme.kt, where the `Radius` tokens live):

  radius     `RoundedCornerShape(12.dp)` (or any literal / arithmetic first
             argument), `CornerSize(8.dp)`, `cornerRadius = 8.dp`, `radius = 4.dp`
  separator  `Divider(`, `HorizontalDivider(`, `VerticalDivider(`

A percent shape (`RoundedCornerShape(50)`) is a literal too: use a token or a
named shape.

A site that is right on purpose carries `// style-exempt: <reason>` on its line
or the line above (a menu's `Divider()` is `// style-exempt: menu`).

There is no baseline: every site fails. A file with a hit of any rule is a new
site, and an `// style-exempt:` with no reason fails too.

  ./scripts/visual-tokens-lint.py                  scan the tree; exit 1 on any site
  ./scripts/visual-tokens-lint.py --self-test      check the rules against known cases
"""

import collections
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# (root, extension, fewest files a scan of it may find). A root that finds fewer
# is the wrong tree.
SOURCES = [
    ("apps/macos/Sources", ".swift", 100),
    ("apps/shared/AgentKit/Sources", ".swift", 10),
    ("apps/ios", ".swift", 50),
    ("apps/android/app/src/main", ".kt", 100),
]
# Where the tokens are defined, so the only place a radius, a material or a gray
# may be spelled out.
TOKEN_FILES = {"DesignTokens.swift", "Theme.kt"}
TOKEN_DIRS = {"Style"}
# UI-test targets and build output are not views.
SKIP_DIRS = {"FarCoolerUITests", "build", ".build"}
MIN_FILES = sum(m for _, _, m in SOURCES)

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
KOTLIN_RULES: dict[str, re.Pattern] = {
    "radius": re.compile(
        r"\bRoundedCornerShape\(\s*[\d(]"
        r"|\b(?:Absolute)?RoundedCornerShape\(\s*\w+\s*=\s*[\d(]"
        r"|\bCornerSize\(\s*[\d(]"
        r"|\b(?:cornerRadius|radius)\s*=\s*\d[\d.]*f?\.dp\b"),
    "separator": re.compile(r"\b(?:Horizontal|Vertical)?Divider\("),
}
EXEMPT = re.compile(r"//\s*style-exempt\s*:\s*\S")
EMPTY_EXEMPT = re.compile(r"//\s*style-exempt\s*:?\s*$")

# What to do about a new site, by rule.
USE = {
    "radius": "use Radius.small/.medium/.large via .control, .card or .floating (or .concentric); on Android, the Radius tokens in Theme.kt",
    "material": "use .surface(.content/.floating, in:) (a frosted plane draws nothing)",
    "glass": "use .surface(.floating, in:), or one GlassEffectContainer of floating members",
    "separator": "use Spacing, or .separator(.grid/.split/.listEdge); a menu Divider is exempt; "
                 "on Android, the theme's separator",
    "stroke": "drop it: .surface supplies the edge; a state stroke (drop target) is exempt",
    "shadow": "drop it: .surface(.floating) draws its own",
    "fill": "use Fill.inset, Fill.hover, Fill.selection or Tint.attentionFill",
}


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


def hits(text: str, rules: dict[str, re.Pattern] = RULES) -> collections.Counter:
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
        for rule, pattern in rules.items():
            counts[rule] += len(pattern.findall(code))
    return +counts


def files():
    for root, ext, _ in SOURCES:
        for path in sorted((ROOT / root).rglob("*" + ext)):
            rel = path.relative_to(ROOT)
            if path.name in TOKEN_FILES or (TOKEN_DIRS | SKIP_DIRS) & set(rel.parts):
                continue
            yield path


def rules_for(path: pathlib.Path) -> dict[str, re.Pattern]:
    return KOTLIN_RULES if path.suffix == ".kt" else RULES


def tree_counts() -> tuple[dict[str, dict[str, int]], int]:
    out, scanned = {}, 0
    for path in files():
        scanned += 1
        counts = hits(path.read_text(encoding="utf-8"), rules_for(path))
        if counts:
            out[str(path.relative_to(ROOT))] = dict(sorted(counts.items()))
    return out, scanned


def compare(tree: dict) -> list[str]:
    """One problem per file and rule with any site."""
    problems = []
    for f in sorted(tree):
        for rule in RULES:
            have = tree[f].get(rule, 0)
            if have:
                problems.append(
                    f"{f}: {have} {rule} site(s).\n"
                    f"    Fix: {USE[rule]}.\n"
                    "    Or, if it is deliberate, put `// style-exempt: <reason>` on the line "
                    "or the line above (the reason must not be empty).")
    return problems


def empty_exempts() -> list[str]:
    out = []
    for path in files():
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if EMPTY_EXEMPT.search(line):
                out.append(f"{path.relative_to(ROOT)}:{number}: `// style-exempt:` needs a reason")
    return out


def scan() -> int:
    tree, scanned = tree_counts()
    if scanned < MIN_FILES:
        print(f"visual-tokens-lint: scanned only {scanned} files; the roots are wrong")
        return 1
    seen = [str(p.relative_to(ROOT)) for p in files()]
    for root, ext, fewest in SOURCES:
        found = sum(1 for f in seen if f.startswith(root + "/"))
        if found < fewest:
            print(f"visual-tokens-lint: {root} has only {found} {ext} files; the root is wrong")
            return 1
    problems = compare(tree)
    problems += empty_exempts()
    for line in problems:
        print(line)
    if problems:
        print(f"\n{len(problems)} problem(s). Design: .claude/agent/reports/ov-216/design.md")
        return 1
    print(f"visual-tokens-lint: no sites ({scanned} scanned)")
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
    kotlin_red = [
        ("radius", ".clip(RoundedCornerShape(12.dp))"),
        ("radius", "shape = RoundedCornerShape(50),"),
        ("radius", "RoundedCornerShape((28f * fraction).dp)"),
        ("radius", "RoundedCornerShape(topStart = 8.dp, topEnd = 8.dp)"),
        ("radius", "val s = CornerSize(4.dp)"),
        ("radius", "Canvas { drawRoundRect(cornerRadius = 6.dp) }"),
        ("separator", "HorizontalDivider()"),
        ("separator", "HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)"),
        ("separator", "VerticalDivider(Modifier.height(8.dp))"),
        ("separator", "Divider(thickness = 1.dp)"),
    ]
    kotlin_green = [
        ".clip(RoundedCornerShape(Radius.medium))",
        ".clip(CircleShape)",
        ".padding(vertical = 4.dp)",
        "// RoundedCornerShape(12.dp) was the old look",
        " * A HorizontalDivider() would draw a rule here.",
        "val note = \"HorizontalDivider() is a word here\"",
        "radius = size.ringRadius(attention).dp.toPx()",
        "HorizontalDivider() // style-exempt: sheet grabber",
        "val dividerColor = outlineVariant",
    ]
    failed = 0
    for rule, line in kotlin_red:
        if not hits(line, KOTLIN_RULES).get(rule):
            print(f"self-test: kotlin {rule} missed: {line!r}")
            failed += 1
    for line in kotlin_green:
        if hits(line, KOTLIN_RULES):
            print(f"self-test: kotlin wrongly caught {dict(hits(line, KOTLIN_RULES))}: {line!r}")
            failed += 1
    if hits("RoundedCornerShape(12.dp)", RULES):
        print("self-test: a Swift rule matched Kotlin")
        failed += 1
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
    # No baseline: any site in any file is a problem.
    if compare({}) or len(compare({"A.swift": {"radius": 1}})) != 1:
        print("self-test: a single site is not exactly one problem")
        failed += 1
    problems = compare({"A.swift": {"radius": 3}})
    if "Radius." not in problems[0] or "style-exempt" not in problems[0]:
        print("self-test: the error doesn't say which token or how to exempt")
        failed += 1
    if not EMPTY_EXEMPT.search("Divider() // style-exempt:") or EMPTY_EXEMPT.search("x // style-exempt: menu"):
        print("self-test: an empty exemption isn't detected")
        failed += 1
    # The real scan must see the real tree, or a broken root would pass green.
    seen = [str(p.relative_to(ROOT)) for p in files()]
    for root, ext, fewest in SOURCES:
        if sum(1 for f in seen if f.startswith(root + "/")) < fewest:
            print(f"self-test: the scan sees fewer than {fewest} {ext} files under {root}")
            failed += 1
    if any("UITests" in f or f.endswith("Theme.kt") for f in seen):
        print("self-test: the scan reads a UI-test target or the theme")
        failed += 1
    print("visual-tokens-lint self-test: " + ("ok" if not failed else f"{failed} failed"))
    return 1 if failed else 0


if __name__ == "__main__":
    if "--self-test" in sys.argv[1:]:
        sys.exit(self_test())
    sys.exit(scan())
