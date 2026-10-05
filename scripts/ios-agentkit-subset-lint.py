#!/usr/bin/env python3
"""The iOS targets compile a hand-kept subset of AgentKit (ov-339).

`apps/macos` imports AgentKit as a module, so every file in it sees every other.
iOS has no SwiftPM project: `apps/ios/generate-project.py` names, target by
target, the AgentKit files each one compiles. A shared change that names a type
from a file a target does not compile builds on the Mac, in `swift test`, and
fails only on the phone. It happened twice on Oct 5: `PageLive.swift` named
`OneTreeFilter` (declared in `OneTree.swift`, not listed; fixed in 28829613),
and a lane added AgentKit files the iOS code used without listing them.

For each target (the phone, its Live Activity and widgets, the notification
service, the watch app, the watch complication) this reads the generator's
lists and fails when a source the target compiles, AgentKit or its own, names
a top-level type, free function or top-level constant that is declared only in
an AgentKit file the target does not compile. An `extension Foo` of such a type
names it too. Comments and string literals are ignored (interpolations are
code); a `.name` after a dot is a member, not the top-level name; a name the
referencing file declares itself, anywhere, is its own.

Not caught: a member a listed type gets from an extension in an unlisted file
(that takes type checking), and a name that is also an Apple framework's. A
name that really is Apple's goes on APPLE_NAMES below, with the reason.

  ./scripts/ios-agentkit-subset-lint.py              check the tree; exit 1 on a hit
  ./scripts/ios-agentkit-subset-lint.py --self-test  plant breaks, see each caught
"""

import ast
import pathlib
import re
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
GENERATOR = ROOT / "apps/ios/generate-project.py"
IOS = ROOT / "apps/ios"
AGENTKIT = ROOT / "apps/shared/AgentKit/Sources/AgentKit"

# Names an unlisted AgentKit file declares that a target's file legitimately gets
# from an Apple framework instead. Each needs its reason; none today.
APPLE_NAMES: dict[str, str] = {}

# (target, its own directory under apps/ios, the generator lists naming its own
# sources, and what names its AgentKit sources). A source named in any of these
# resolves to the target's directory if the file is there, else to AgentKit.
TARGETS = [
    ("phone", "FarCooler", ["SOURCES", "CEREMONY_SOURCES"], ["AGENTKIT_SOURCES"]),
    ("activity", "FarCoolerActivity", ["ACTIVITY_SOURCES"], ["activity_build_ids"]),
    ("notify", "FarCoolerNotify", ["NOTIFY_SOURCES"], ["notify_build_ids"]),
    ("watch", "FarCoolerWatch", ["WATCH_SOURCES"], ["WATCH_AGENTKIT_SOURCES"]),
    ("watch widgets", "FarCoolerWatchWidgets", ["WATCH_WIDGET_SOURCES"],
     ["WATCH_WIDGET_AGENTKIT_SOURCES"]),
]


# ---- reading the generator's lists -------------------------------------


def literal_lists(generator: pathlib.Path) -> dict[str, list[str]]:
    """Every top-level `NAME = [...]` of strings, or `NAME = {k: ... for k in A + [...]}`.

    The generator writes files when run, so it is parsed, not imported. Two
    shapes carry a target's sources: a plain list, and a dict comprehension
    whose iterable is `LIST + ["a.swift", ...]` (the build-id maps).
    """
    tree = ast.parse(generator.read_text())
    found: dict[str, list[str]] = {}

    def strings(node: ast.AST) -> list[str] | None:
        if isinstance(node, ast.List):
            if all(isinstance(e, ast.Constant) and isinstance(e.value, str) for e in node.elts):
                return [e.value for e in node.elts]
            return None
        if isinstance(node, ast.Name):
            return found.get(node.id)
        if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Add):
            left, right = strings(node.left), strings(node.right)
            return None if left is None or right is None else left + right
        return None

    for node in tree.body:
        if not isinstance(node, ast.Assign) or len(node.targets) != 1:
            continue
        target = node.targets[0]
        if not isinstance(target, ast.Name):
            continue
        value = node.value
        if isinstance(value, ast.DictComp) and value.generators:
            value = value.generators[0].iter
        got = strings(value)
        if got is not None:
            found[target.id] = got
    return found


def load_targets(generator, ios, agentkit):
    """[(target name, [(label, path)] for every file it compiles, [AgentKit paths])]."""
    lists = literal_lists(generator)
    out = []
    for name, directory, own_lists, ak_lists in TARGETS:
        names = []
        for key in own_lists + ak_lists:
            if key not in lists:
                raise SystemExit(f"ios-agentkit-subset-lint: {generator.name} has no list "
                                 f"`{key}` (target {name}); the lint's TARGETS table is stale")
            names += [n for n in lists[key] if n not in names]
        files, ak_files = [], []
        for n in names:
            own = next(iter(sorted((ios / directory).rglob(n))), None)
            shared = agentkit / n
            if own is not None:
                files.append(own)
            elif shared.is_file():
                files.append(shared)
                ak_files.append(shared)
            else:
                raise SystemExit(f"ios-agentkit-subset-lint: target {name} lists {n}, "
                                 f"which is in neither {directory}/ nor AgentKit")
        out.append((name, files, ak_files))
    return out


# ---- reading Swift ------------------------------------------------------


def code_only(text: str) -> str:
    """The text with comments removed and string contents blanked, newlines kept.

    A string interpolation `\\( ... )` is code and stays. Handles nested block
    comments, triple-quoted strings and raw (`#`-delimited) strings.
    """
    out: list[str] = []
    i, n = 0, len(text)

    def skip_code(i: int, until_paren: bool) -> int:
        """Copy code from i; stop at the `)` closing an interpolation, if asked."""
        depth = 0
        while i < n:
            c = text[i]
            two = text[i:i + 2]
            if two == "//":
                while i < n and text[i] != "\n":
                    i += 1
            elif two == "/*":
                level, i = 1, i + 2
                while i < n and level:
                    if text[i:i + 2] == "/*":
                        level, i = level + 1, i + 2
                    elif text[i:i + 2] == "*/":
                        level, i = level - 1, i + 2
                    else:
                        if text[i] == "\n":
                            out.append("\n")
                        i += 1
            elif c == '"' or (c == "#" and re.match(r'#+"', text[i:i + 16])):
                i = skip_string(i)
            else:
                if until_paren:
                    if c == "(":
                        depth += 1
                    elif c == ")":
                        if depth == 0:
                            return i + 1
                        depth -= 1
                out.append(c)
                i += 1
        return i

    def skip_string(i: int) -> int:
        hashes = 0
        while text[i] == "#":
            hashes, i = hashes + 1, i + 1
        multi = text[i:i + 3] == '"""'
        i += 3 if multi else 1
        close = ('"""' if multi else '"') + "#" * hashes
        escape = "\\" + "#" * hashes
        out.append('""')
        while i < n:
            if text.startswith(escape, i):
                if text[i + len(escape)] == "(":
                    out.append(" ")
                    i = skip_code(i + len(escape) + 1, True)
                    out.append(" ")
                else:
                    i += len(escape) + 1
            elif text.startswith(close, i):
                return i + len(close)
            else:
                if text[i] == "\n":
                    out.append("\n")
                i += 1
        return i

    skip_code(0, False)
    return "".join(out)


TYPE_DECL = re.compile(
    r"^[ \t]*(?:@\w+(?:\([^)\n]*\))?[ \t]+|(?:public|open|internal|fileprivate|private|"
    r"final|indirect)[ \t]+)*(?:struct|class|enum|actor|protocol|typealias)[ \t]+([A-Za-z_]\w*)",
    re.M)
TOP_TYPE_DECL = re.compile(
    r"^(?:@\w+(?:\([^)\n]*\))?[ \t]+|(?:public|open|internal|fileprivate|private|"
    r"final|indirect)[ \t]+)*(?:struct|class|enum|actor|protocol|typealias)[ \t]+([A-Za-z_]\w*)",
    re.M)
TOP_FUNC = re.compile(
    r"^(?:@\w+(?:\([^)\n]*\))?[ \t]+|(?:public|open|internal|fileprivate|private)[ \t]+)*"
    r"func[ \t]+([A-Za-z_]\w*)", re.M)
TOP_VALUE = re.compile(
    r"^(?:@\w+(?:\([^)\n]*\))?[ \t]+|(?:public|open|internal|fileprivate|private)[ \t]+)*"
    r"(?:let|var)[ \t]+([A-Za-z_]\w*)", re.M)
ANY_DECL = re.compile(
    r"\b(?:struct|class|enum|actor|protocol|typealias|func|let|var|case|associatedtype)"
    r"[ \t]+([A-Za-z_]\w*)")
IDENT = re.compile(r"(?<![\w.])([A-Za-z_]\w*)")
FUNC_CALL = re.compile(r"(?<![\w.])([A-Za-z_]\w*)[ \t]*[(<]")


def top_level(code: str) -> dict[str, str]:
    """Top-level (column 0) names a file declares: name -> kind."""
    names: dict[str, str] = {}
    for pattern, kind in ((TOP_TYPE_DECL, "type"), (TOP_FUNC, "function"),
                          (TOP_VALUE, "constant")):
        for m in pattern.finditer(code):
            names.setdefault(m.group(1), kind)
    return names


def declared_anywhere(code: str) -> set[str]:
    """Every name the file declares at any depth, including parameters' `case`s."""
    names = {m.group(1) for m in ANY_DECL.finditer(code)}
    names |= {m.group(1) for m in TYPE_DECL.finditer(code)}
    # Parameter names and closure parameters: `(name: Type`, `name in`.
    names |= {m.group(1) for m in re.finditer(r"[(,]\s*(?:_\s+)?([A-Za-z_]\w*)\s*:", code)}
    names |= {m.group(1) for m in re.finditer(r"[{,]\s*([A-Za-z_]\w*)\s+in\b", code)}
    return names


def check_target(name, files, ak_files, agentkit_files):
    """[(file, line, what)] for one target."""
    compiled = {f.resolve() for f in files}
    stripped = {f: code_only(f.read_text()) for f in files}
    have: set[str] = set()
    for f in files:
        have |= set(top_level(stripped[f]))
    # What only an unlisted AgentKit file declares.
    missing: dict[str, tuple[str, str]] = {}
    for f in agentkit_files:
        if f.resolve() in compiled:
            continue
        for decl, kind in top_level(code_only(f.read_text())).items():
            if decl not in have and decl not in APPLE_NAMES:
                missing.setdefault(decl, (kind, f.name))
    hits = []
    for f in files:
        code = stripped[f]
        own = declared_anywhere(code)
        calls = {m.group(1) for m in FUNC_CALL.finditer(code)}
        for number, line in enumerate(code.splitlines(), 1):
            if line.lstrip().startswith(("import ", "@testable")):
                continue
            for m in IDENT.finditer(line):
                word = m.group(1)
                if word not in missing or word in own:
                    continue
                kind, home = missing[word]
                if kind == "function" and word not in calls:
                    continue
                hits.append((f, number, f"{word}, a {kind} declared only in {home}"))
    return hits


def scan(generator=GENERATOR, ios=IOS, agentkit=AGENTKIT, quiet=False):
    agentkit_files = sorted(agentkit.glob("*.swift"))
    merged: dict[tuple, list[str]] = {}
    for name, files, ak_files in load_targets(generator, ios, agentkit):
        for f, number, what in check_target(name, files, ak_files, agentkit_files):
            if name not in merged.setdefault((f, number, what), []):
                merged[(f, number, what)].append(name)
    problems = [(", ".join(names), f, number, what) for (f, number, what), names in merged.items()]
    if not quiet:
        for name, f, number, what in problems:
            try:
                shown = f.relative_to(ROOT)
            except ValueError:
                shown = f
            print(f"{shown}:{number}: the {name} target does not compile the file that "
                  f"declares {what}.\n    List it in apps/ios/generate-project.py, or do "
                  f"not name it here.")
    return problems


# ---- the self-test ------------------------------------------------------

GEN = '''
SOURCES = ["App.swift"]
CEREMONY_SOURCES = []
AGENTKIT_SOURCES = ["PageLive.swift", "Listed.swift"]
ACTIVITY_SOURCES = ["Widget.swift"]
activity_build_ids = {n: n for n in ACTIVITY_SOURCES + ["Listed.swift"]}
NOTIFY_SOURCES = []
notify_build_ids = {n: n for n in NOTIFY_SOURCES + ["Listed.swift"]}
WATCH_SOURCES = []
WATCH_AGENTKIT_SOURCES = ["Listed.swift"]
WATCH_WIDGET_SOURCES = []
WATCH_WIDGET_AGENTKIT_SOURCES = ["Listed.swift"]
'''

# What 28829613 fixed, in the shape it had: a listed file naming a type that
# lives in an unlisted one.
PAGE_LIVE_BROKEN = '''
extension PageWorld {
    /// A card-count reference's status, as the board says it.
    static func statusName(_ word: String) -> String {
        if word == "open" { return OneTreeFilter.open.title }
        return TaskStatus(rawValue: word)?.title ?? word
    }
}
'''
PAGE_LIVE_FIXED = '''
extension PageWorld {
    static func statusName(_ word: String) -> String {
        // The phones compile this file without OneTree.swift, so it can't
        // name OneTreeFilter.
        if word == "open" { return "Not Done" }
        return TaskStatus(rawValue: word)?.title ?? word
    }
}
'''
ONE_TREE = '''
enum OneTreeFilter: String { case open; var title: String { "Not Done" } }
struct OneTreeRow {}
func oneTreeHelper(_ x: Int) -> Int { x }
let oneTreeLimit = 5
extension String { func shout() -> String { self } }
'''
LISTED = "struct TaskStatus { var title: String { \"\" }; init?(rawValue: String) {} }\n"


def self_test() -> int:
    failures: list[str] = []

    def expect(label, files: dict[str, str], hits: int, mentions: str | None = None):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            (tmp / "gen.py").write_text(GEN)
            ios, kit = tmp / "ios", tmp / "kit"
            for d in ("FarCooler", "FarCoolerActivity", "FarCoolerNotify", "FarCoolerWatch",
                      "FarCoolerWatchWidgets"):
                (ios / d).mkdir(parents=True)
            kit.mkdir()
            base = {"kit/Listed.swift": LISTED, "kit/OneTree.swift": ONE_TREE,
                    "ios/FarCooler/App.swift": "struct App {}\n",
                    "ios/FarCoolerActivity/Widget.swift": "struct Widget {}\n",
                    "kit/PageLive.swift": PAGE_LIVE_FIXED}
            base.update(files)
            for rel, body in base.items():
                (tmp / rel).write_text(body)
            got = scan(tmp / "gen.py", ios, kit, quiet=True)
            text = " ".join(w for _, _, _, w in got)
            if len(got) != hits or (mentions and mentions not in text):
                failures.append(f"{label}: wanted {hits} hit(s)"
                                f"{' naming ' + mentions if mentions else ''}, got {got}")

    expect("clean tree", {}, 0)
    expect("PageLive names OneTreeFilter (28829613^)", {"kit/PageLive.swift": PAGE_LIVE_BROKEN},
           1, "OneTreeFilter")
    expect("the fix of 28829613 passes", {"kit/PageLive.swift": PAGE_LIVE_FIXED}, 0)
    expect("a listed file extends an unlisted type",
           {"kit/Listed.swift": LISTED + "extension OneTreeRow { var x: Int { 1 } }\n"},
           1, "OneTreeRow")
    expect("a listed file calls an unlisted free function",
           {"kit/Listed.swift": LISTED + "func f() -> Int { oneTreeHelper(2) }\n"},
           1, "oneTreeHelper")
    expect("a listed file reads an unlisted top-level constant",
           {"kit/Listed.swift": LISTED + "func f() -> Int { oneTreeLimit }\n"},
           1, "oneTreeLimit")
    expect("an iOS app source names an unlisted type",
           {"ios/FarCooler/App.swift": "struct App { var f: OneTreeFilter = .open }\n"},
           1, "OneTreeFilter")
    expect("a Live Activity source names an unlisted type",
           {"ios/FarCoolerActivity/Widget.swift": "struct Widget { let r: OneTreeRow }\n"},
           1, "OneTreeRow")
    expect("a new AgentKit file used by the app but not listed",
           {"kit/Fresh.swift": "struct FreshThing {}\n",
            "ios/FarCooler/App.swift": "struct App { let t = FreshThing() }\n"},
           1, "FreshThing")
    expect("the same file, listed, is fine",
           {"kit/Fresh.swift": "struct FreshThing {}\n",
            "kit/Listed.swift": LISTED,
            "gen.py": GEN.replace('["PageLive.swift", "Listed.swift"]',
                                  '["PageLive.swift", "Listed.swift", "Fresh.swift"]'),
            "ios/FarCooler/App.swift": "struct App { let t = FreshThing() }\n"},
           0)
    # Things that must NOT be hits.
    expect("a comment naming it",
           {"kit/Listed.swift": LISTED + "// OneTreeFilter, OneTreeRow\n"
            "/* oneTreeHelper(1) /* nested OneTreeFilter */ OneTreeRow */\n"
            "/// `OneTreeFilter` is the tree's\n"}, 0)
    expect("a string naming it",
           {"kit/Listed.swift": LISTED + 'let a = "OneTreeFilter"\n'
            'let b = """\nOneTreeRow and oneTreeHelper(1)\n"""\n'
            'let c = #"OneTreeFilter \\(OneTreeRow)"#\n'}, 0)
    expect("an interpolation is code",
           {"kit/Listed.swift": LISTED + 'let a = "x \\(OneTreeRow())"\n'}, 1, "OneTreeRow")
    expect("a member of the same name",
           {"kit/Listed.swift": LISTED + "func f(x: Int) -> Int { x.oneTreeHelper(); "
            "return Other.OneTreeRow }\n"}, 0)
    expect("a name the file declares itself",
           {"kit/Listed.swift": LISTED + "enum Mine { struct OneTreeRow {} }\n"
            "func g(oneTreeHelper: Int) {}\n"}, 0)
    expect("a name declared by a listed file too",
           {"kit/Listed.swift": LISTED + "struct OneTreeRow {}\n"}, 0)
    expect("an unlisted extension of String is not a name",
           {"kit/Listed.swift": LISTED + "let s = String()\n"}, 0)

    # A list the generator no longer has, and a listed file that is gone.
    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        (tmp / "gen.py").write_text("SOURCES = []\n")
        try:
            load_targets(tmp / "gen.py", tmp, tmp)
            failures.append("a generator without the lists should be refused")
        except SystemExit:
            pass

    if failures:
        print("\n".join(f"FAIL: {f}" for f in failures))
        return 1
    print("ios-agentkit-subset-lint --self-test: ok")
    return 0


def main(argv: list[str]) -> int:
    if argv == ["--self-test"]:
        return self_test()
    if argv:
        print(__doc__)
        return 2
    problems = scan()
    if problems:
        print(f"\nios-agentkit-subset-lint: {len(problems)} reference(s) the iOS targets "
              f"cannot resolve.")
        return 1
    print("ios-agentkit-subset-lint: every iOS target's AgentKit references resolve")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
