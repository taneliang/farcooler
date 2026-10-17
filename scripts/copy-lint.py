#!/usr/bin/env python3
"""No count in parentheses in any app's copy (ov-101), Android copy in
sentence case, and Apple control titles in title case (ov-204).

Apple sets buttons, menu items, menus, navigation titles, section headers and
labels in title case: "Add a Repository…", "Not on This Version". Articles,
coordinating conjunctions and prepositions of four letters or fewer stay
lowercase unless first or last (APPLE_SMALL). The Swift check reads the first
string argument of Button, Menu, Label, NavigationLink, Section, Tab,
CommandMenu, CommandGroup, NSMenuItem(title:), .navigationTitle,
`label: { Text("…") }` and, in the phone app, Toggle and Picker (the Mac's checkboxes
read as sentences). A call split over lines is read. An .alert or
.confirmationDialog title passes in title case or as a sentence in sentence
case ("Camera access is off"), as the HIG has it. A string that reads as a
sentence (it ends in punctuation, or holds a comma or a period inside) is body
text and passes. A fragment written in sentence case ("Lost connection to this runner") can't be told from a sentence without a verb list, so review catches it. Not checked: bare Text("…") headings and Text in a multi-line
label closure, which can't be told from body text. A name or wire value says
`casing ok: <reason>` on its line; a bare `casing ok` doesn't exempt.

Material 3 sets every label in sentence case, buttons, menu items and titles
included: "Try again", "Needs you", "Mark all as read". Only proper nouns keep
a capital (CASING_PROPER below). In a Kotlin string literal a capitalized
word that isn't first, isn't after a sentence's end and isn't a proper noun is
a hit. A literal that is a name or a wire value says `casing ok` in a comment
on its line.

The parentheses rule, from ov-101:

The owner's rule, from ov-104: "parentheses are a plain-text habit". A count
beside a header goes in the header's trailing slot (`SectionCount` on the
Mac), tertiary, in tabular digits; a menu item's count goes in the menu's
badge; a sentence says "3 things need you". Never "Finished (14)".

This scans the user-facing sources of the Mac, the phones and the shared
Swift package for the shapes such a count takes:

  Swift   "Finished (\\(count))"        an interpolation leading the parentheses,
          "Unread (\\(n) new)"           words after it or not
  Kotlin  "Finished ($count)"           a template leading them
          "Finished (${items.size})"
  any     "Finished (%d)", "(%1$d new)" a format specifier leading them

Lines inside a multi-line \"\"\" string count as string content.

The patterns flag any parenthesized interpolation that leads its parentheses,
which also catches errors, dates and names: those take an allow comment. That
churn is the price of not guessing which expressions are numbers. String
concatenation ("(" + n + ")") isn't seen.

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

# An interpolation's expression, one level of parentheses deep.
_EXPR = r"[^()]*(?:\([^()]*\)[^()]*)*"
# A count leads the parentheses, and words may follow it: "(\(n))",
# "(\(n) new)", "(\(a) of \(b))".
SWIFT = re.compile(r"\(\\\(" + _EXPR + r"\)(?:[^()\"\\]|\\\(" + _EXPR + r"\))*\)")
KOTLIN = re.compile(r"\(\$(?:\{[^{}]*\}|[A-Za-z_][A-Za-z0-9_.]*)(?:[^()\"$]|\$\{[^{}]*\}|\$[A-Za-z_]\w*)*\)")
FORMAT = re.compile(r"\(%(?:\d+\$)?[,']?(?:l{0,2}[dui]|@|s)[^()\"%]*\)")
# A line that matches without breaking the rule says why on the line: it isn't
# a count, or it isn't drawn (a prompt written to an agent).
ALLOWS = ("not a count", "not UI copy")

# Android casing (ov-204): words that keep their capital mid-string.
CASING_PROPER = {
    "GitHub", "Codex", "Android", "Git", "Cursor", "Mac", "Keychain", "WorkOS", "Gemini",
    "Google", "Tailscale", "Tailcat", "Firebase", "Linux", "I", "Opus", "Sonnet", "Haiku", "Claude",
}
# Multi-word names match as whole phrases: "Claude Code" passes, "Review Code"
# doesn't, and neither word passes alone.
CASING_PHRASES = ("Far Cooler", "Claude Code")
CASING_ALLOW = "casing ok"
# The exemption names its reason: `casing ok: brand name`. A bare `casing ok` doesn't exempt.
_CASING_OK = re.compile(r"casing ok:\s*\S")

# Apple title case (ov-204). A particle Apple capitalizes in a phrasal verb
# ("Sign Out", "Clean Up", "Turn Off") is right capitalized anywhere; the
# lowercase form ("Sign out") is not exempt and fails.
APPLE_SMALL = set("a an the and but or nor for yet so as at by in of on to from with into onto per via".split())
APPLE_PARTICLES = {"Up", "Off", "Out", "Over"}
APPLE_TECH = {"tmux", "git", "ssh", "npm", "npx"}
_STR = r'"((?:[^"\\\n]|\\.)*)"'
_APPLE_CALL = re.compile(
    r'(?<![\w.])(?:Button|Menu|Label|NavigationLink|Section|CommandMenu|CommandGroup|Tab)\(\s*' + _STR
    + r'|\.navigation(?:Bar)?Title\(\s*' + _STR
    + r'|(?<![\w.])NSMenuItem\(\s*title:\s*' + _STR
    + r'|\blabel:\s*\{\s*Text\(\s*' + _STR)
# An alert's or dialog's title may be a sentence ("Camera access is off") or a
# fragment ("Lost Connection to This Runner"): either casing passes.
_APPLE_ALERT = re.compile(r'\.(?:alert|confirmationDialog)\(\s*' + _STR)
_APPLE_TOGGLE = re.compile(r'(?<![\w.])(?:Toggle|Picker)\(\s*' + _STR)
_INTERPOLATION = re.compile(r'\\\((?:[^()]|\([^()]*\))*\)')
_LITERAL = re.compile(r'"((?:[^"\\\n]|\\.)*)"')
_WORD = re.compile(r"[A-Z][a-z]+(?:\u2019[a-z]+)?")


def title_case_words(text: str) -> list[str]:
    """Capitalized words in a string that sentence case wouldn't capitalize."""
    text = re.sub(r"\$\{[^{}]*\}|\$\w+", "\0", text)
    for phrase in CASING_PHRASES:
        text = re.sub(r"\b" + phrase + r"\b", "\1", text)
    words = text.split()
    out = []
    for i, word in enumerate(words):
        core = word.strip(".,;:!?()\u201c\u201d\u2018\u2019\u2026\u203a>\"'")
        if i == 0 or not _WORD.fullmatch(core) or core in CASING_PROPER:
            continue
        if words[i - 1][-1:] in ".?!:":
            continue
        out.append(core)
    return out


def apple_case_words(text: str) -> list[str]:
    """Words a title-case control title gets wrong: a lowercase word that
    isn't a small one, a small one in the middle capitalized, or an edge
    word lowercase. Sentences are body text and pass."""
    text = _INTERPOLATION.sub("\0", text).rstrip("\u2026").rstrip()
    if not text or re.search(r"[.?!:;]$", text) or ". " in text or ", " in text:
        return []
    words = text.split()
    out = []
    for i, word in enumerate(words):
        core = word.strip("\u201c\u201d\u2018\u2019\"'()\u2026,")
        if not core or not core[0].isalpha() or core[0:1] == "\0":
            continue
        if any(c.isupper() for c in core[1:]) or core in APPLE_TECH:
            continue
        edge = i == 0 or i == len(words) - 1
        small = core.lower() in APPLE_SMALL
        if core in APPLE_PARTICLES:
            continue
        if core[0].islower() and (edge or not small):
            out.append(core)
        elif core[0].isupper() and small and not edge:
            out.append(core)
    return out


def apple_hits(text: str, phone: bool = False) -> list[tuple[int, str]]:
    """Calls are matched across the whole text, so a call split over lines is
    read; the exemption and the comment test use the line the string is on."""
    lines = text.splitlines()
    patterns = [(_APPLE_CALL, False), (_APPLE_ALERT, True)] + ([(_APPLE_TOGGLE, False)] if phone else [])
    out = {}
    for pattern, alert in patterns:
        for match in pattern.finditer(text):
            literal = next(g for g in match.groups() if g is not None)
            number = text.count("\n", 0, match.start(match.lastindex)) + 1
            line = lines[number - 1]
            if _CASING_OK.search(line) or not code(line):
                continue
            if alert and literal[:1].islower():
                out[number] = line.strip()
                continue
            if not apple_case_words(literal):
                continue
            if alert and not title_case_words(literal):
                continue
            out[number] = line.strip()
    return sorted(out.items())


def casing_hits(text: str) -> list[tuple[int, str]]:
    out = []
    for number, line in enumerate(text.splitlines(), 1):
        if _CASING_OK.search(line) or not code(line):
            continue
        for match in _LITERAL.finditer(line):
            if title_case_words(match.group(1)):
                out.append((number, line.strip()))
                break
    return out


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
    # Inside a multi-line `"""` string, every line is string content.
    in_block = False
    for number, line in enumerate(text.splitlines(), 1):
        was_in_block = in_block
        if line.count('"""') % 2 == 1:
            in_block = not in_block
        if any(allow in line for allow in ALLOWS):
            continue
        body = line if was_in_block else code(line)
        # Only inside a string: a count is copy, and a call's parentheses
        # around an interpolation-free argument are not.
        if '"' not in body and kind != "xml" and not was_in_block:
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
    casing = 0
    for path, kind in files():
        if kind != "kotlin":
            continue
        for number, line in casing_hits(path.read_text(encoding="utf-8")):
            print(f"{path.relative_to(ROOT)}:{number}: title case in Android copy: {line}")
            casing += 1
    apple = 0
    for path, kind in files():
        if kind != "swift":
            continue
        phone = "apps/ios" in path.as_posix()
        for number, line in apple_hits(path.read_text(encoding="utf-8"), phone):
            print(f"{path.relative_to(ROOT)}:{number}: not title case in Apple copy: {line}")
            apple += 1
    if apple:
        print(
            f"\n{apple} Apple control title(s) not in title case. Buttons, menus, section headers and "
            "navigation titles capitalize every word but articles, coordinating conjunctions and "
            "prepositions of four letters or fewer (\"Not on This Version\"), and always the first and last. "
            f"A name or wire value says `{CASING_ALLOW}: <reason>` in a comment on its line.")
    if casing:
        print(
            f"\n{casing} title-case Android string(s). Material uses sentence case "
            "everywhere, buttons included: \"Try again\", never \"Try Again\". "
            "A proper noun goes in CASING_PROPER (a phrase in CASING_PHRASES); a name or wire value says "
            f"`{CASING_ALLOW}: <reason>` in a comment on its line.")
    if found:
        print(
            f"\n{found} parenthesized count(s). Put a header's count in its trailing slot "
            "(SectionCount), a menu item's in its badge, or say it in words. "
            "If one isn't a count, or isn't drawn, say so in a comment on its line: "
            f"{' or '.join(f'`{a}`' for a in ALLOWS)}.")
        return 1
    if casing or apple:
        return 1
    print("copy-lint: no parenthesized counts, no title-case Android copy, no mis-cased Apple titles")
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
        ("swift", 'Text("Unread (\\(n) new)")', True),
        ("swift", 'Text("Retry (\\(attempt) of \\(limit))")', True),
        ("swift", 'let s = """\nUnread (\\(n))\n"""', True),
        ("kotlin", 'Text("Unread ($n new)")', True),
        ("kotlin", 'Text("Retry (${a} of ${b})")', True),
        ("kotlin", 'val s = """\n    Unread ($n)\n"""', True),
        ("swift", 'String(format: "Unread (%1$d new)", n)', True),
        ("swift", 'String(format: "Lines (%1$,d)", n)', True),
        # Each must pass.
        ("swift", 'Text("Finished")', False),
        ("swift", '"\\(marketing) (\\(channel))"  // not a count: the channel', False),
        ("swift", "/// never \"Finished (\\(n))\"", False),
        ("swift", "let x = f(\\(a))", False),
        ("swift", 'Text("\\(n) unchanged lines")', False),
        ("kotlin", 'Text("$count things need you")', False),
        ("kotlin", 'Text("Price (USD)")', False),
        ("swift", 'Text("Tasks (beta)")', False),
        ("swift", 'Text("Open in Terminal (⌘T)")', False),
        ("xml", '<string name="x">%d things need you</string>', False),
    ]
    casing_cases = [
        ('Text("Try Again")', True),
        ('Text("Needs You")', True),
        ('Text("New Task\u2026")', True),
        ('"Show $hidden More"', True),
        ('"$workspace Orchestrator"', True),
        ('"Moved to In Review"', True),
        ('"Review Code"', True),
        ('"Ask Far"', True),
        ('"Open Settings"', True),
        ('"Update Cooler now"', True),
        ('"Ask Claude Code"', False),
        ('"Update Far Cooler now"', False),
        ('Text("Try again")', False),
        ('Text("Needs you")', False),
        ('Text("Open on GitHub")', False),
        ('Text("Update Far Cooler, then try again.")', False),
        ('"Sign in. Then pick a runner."', False),
        ('"Run: Claude Code"', False),
        ('"Content-Type"', False),
        ('"Moved to In Review"  // casing ok: a wire value', False),
        ('// "Try Again" was the old copy', False),
    ]
    apple_cases = [
        ('Button("Add a repository…") {', False, True),
        ('Button("Add a Repository…") {', False, False),
        ('.navigationTitle("Not On This Version")', False, True),
        ('Section("Not on This Version") {', False, False),
        ('Menu("Use as Orchestrator") {', False, False),
        ('CommandMenu("Move to layout")', False, True),
        ('Label("Continue where you stopped", systemImage: "x")', False, True),
        ('Button("Add This Device With a Code")', False, True),
        ('Button("Show \\(n) lines")', False, True),
        ('Button("Show \\(n) Lines")', False, False),
        ('Button("Sign Out Everywhere")', False, False),
        ('Button("Try again")', False, True),
        ('Button("Try Again")', False, False),
        ('Button("Allow Far Cooler to use the camera.")', False, False),
        ('Toggle("When an agent needs you", isOn: $x)', True, True),
        ('Toggle("When an Agent Needs You", isOn: $x)', True, False),
        ('Toggle("Notify when an agent needs you", isOn: $x)', False, False),
        ('Button("Add a repository…")  // casing ok: typed by the person', False, False),
        ('// Button("Add a repository…")', False, False),
        ('.alert("lost connection to this runner", isPresented: $x) {', False, True),
        ('.alert("couldn’t verify your identity", isPresented: $x) {', False, True),
        ('.confirmationDialog("remove this worktree?", isPresented: $x) {', False, True),
        ('Button("Sign out")', False, True),
        ('Button("Log out")', False, True),
        ('Button("Clean up")', False, True),
        ('Button("Clean Up")', False, False),
        ('Button("Turn Off")', False, False),
        ('Button { go() } label: { Text("add a repo") }', False, True),
        ('Button { go() } label: { Text("Add a Repo") }', False, False),
        ('.alert("Couldn’t Verify your identity", isPresented: $x) {', False, True),
        ('.alert("Couldn’t verify your identity", isPresented: $x) {', False, False),
        ('.alert("Lost Connection to This Runner", isPresented: $x) {', False, False),
        ('.confirmationDialog("remove Worktree for Real", isPresented: $x) {', False, True),
        ('.confirmationDialog("Remove this worktree?", isPresented: $x) {', False, False),
        ('NSMenuItem(title: "close tab", action: nil, keyEquivalent: "")', False, True),
        ('NSMenuItem(title: "Close Tab", action: nil, keyEquivalent: "")', False, False),
        ('Picker("pick one", selection: $x) {', True, True),
        ('Picker("Pick One", selection: $x) {', True, False),
        ('Picker("Tiling prefix", selection: $x) {', False, False),
        ('Tab("home page", systemImage: "x") {', False, True),
        ('.navigationBarTitle("not title")', False, True),
        ('Button(\n    "save changes"\n) {', False, True),
        ('Button(\n    "Save Changes"\n) {', False, False),
        ('Button("save")  // casing ok', False, True),
        ('Button("save")  // casing ok: a typed name', False, False),
    ]
    failed = 0
    for line, phone, caught in apple_cases:
        if bool(apple_hits(line, phone)) != caught:
            print(f"self-test: {'missed' if caught else 'wrongly caught'} Apple casing: {line}")
            failed += 1
    for line, caught in casing_cases:
        if bool(casing_hits(line)) != caught:
            print(f"self-test: {'missed' if caught else 'wrongly caught'} casing: {line}")
            failed += 1
    for kind, line, caught in cases:
        if bool(hits(line, kind)) != caught:
            print(f"self-test: {'missed' if caught else 'wrongly caught'} {kind}: {line}")
            failed += 1
    print("copy-lint self-test: " + ("ok" if not failed else f"{failed} failed"))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(self_test() if "--self-test" in sys.argv[1:] else scan())
