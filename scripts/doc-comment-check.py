#!/usr/bin/env python3
"""Refuse a change that leaves a doc comment documenting the wrong item.

The failure it catches: an edit inserts a new item (a fn, a test, a struct)
anchored on an existing item's signature, so the new item lands between that
item and the doc comment above it. The doc comment now sits on the new item,
and the old item has lost it. Nothing else notices — it compiles, the tests
pass, and the prose reads fine until someone checks what it is describing.

Why a diff and not a look at the tree: in the source the orphaned doc and the
new item's own doc are usually one contiguous `///` run, so a long doc comment
and two stuck together look identical to any local rule. What does show is the
artifact across a change: a doc comment that was attached to item X before is,
after, attached to a different item Y, while X still exists without it.

Precisely: for each doc comment D in the old version of a file, the items whose
doc contains D (as whole lines) are counted before and after. It is a finding
when an item X lost D and a different item Y gained it, and:

  - X still exists (a rename, or a removal, takes its doc along with it);
  - X is left with no doc at all (X given a doc of its own is a deliberate move);
  - Y had a doc before, or is new (Y undocumented before is the commit that
    fixes an orphan by moving D back).

An item is its kind and name (`fn forget`, `case idle`, `val name`), so two
overloads of one name count as one item.

Rust, Swift and Kotlin: `///` runs and `/** ... */` blocks, then any
attributes, annotations, plain comments and blank lines, then the item.

    ./scripts/doc-comment-check.py                   # staged changes vs HEAD (the pre-commit hook)
    ./scripts/doc-comment-check.py --base origin/main  # the working tree vs a commit
    ./scripts/doc-comment-check.py --base A --head B   # one commit range, as CI runs it
    ./scripts/doc-comment-check.py --self-test

What it still flags that isn't a bug: a doc moved on purpose onto a new item
that now does the work, leaving the old one an undocumented forwarder. That was
2 of 59 findings over this repository's first 1,087 commits; the other 57 were
real. Commit one with `git commit --no-verify`, and say why in the message.
"""

import argparse
import collections
import re
import subprocess
import sys

EXTENSIONS = (".rs", ".swift", ".kt", ".kts")

DOC_LINE = re.compile(r"^\s*///(?!/)")
BLOCK_OPEN = re.compile(r"^\s*/\*\*(?![*/])")
RUST_ATTR = re.compile(r"^\s*#!?\[")
ANNOTATION = re.compile(r"^\s*@[A-Za-z_]")

MODIFIERS = (
    r"(?:(?:pub(?:\([^)]*\))?|async|unsafe|const|extern(?:\s+\"[^\"]*\")?|default"
    r"|(?:public|private|internal|fileprivate|open|package)(?:\(set\))?|protected|static|final|override"
    r"|mutating|nonmutating|nonisolated|lazy|weak|unowned|dynamic|convenience|required"
    r"|indirect|package|consuming|borrowing|isolated"
    r"|data|sealed|abstract|inline|suspend|operator|infix|tailrec|external|lateinit"
    r"|value|annotation|companion|enum|inner|actual|expect|noinline|crossinline"
    r"|@\w+(?:\([^)]*\))?)\s+)*"
)
DECL = re.compile(
    r"^\s*" + MODIFIERS +
    r"(fn|struct|enum|trait|type|union|mod|const|static|macro_rules!"
    r"|func|var|let|case|class|protocol|extension|actor|typealias|associatedtype|macro"
    r"|fun|val|object|interface)\s+"
    r"(?:<[^>]*>\s*)?(?:[\w.]+\.)?(`?[A-Za-z_][\w]*`?)"
)
# `impl Foo for Bar`, `init(`, `subscript(`, `deinit`, Kotlin `constructor(`.
IMPL = re.compile(r"^\s*(?:unsafe\s+)?impl\b\s*(?:<[^{]*?>)?\s*([^{]*)")
BARE = re.compile(r"^\s*" + MODIFIERS + r"(init|deinit|subscript|constructor)\b[?!]?")
# An enum variant or entry: `Idle,`, `Busy(u32),`, `Named { .. }`, `ONE(1),`.
VARIANT = re.compile(r"^\s*([A-Za-z_]\w*)\s*(?:[({=,]|$)")
# A struct field: `pub name: Type,`.
FIELD = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?([a-z_]\w*)\s*:(?!:)")


def item_key(line):
    """The kind and name of the item a line declares, or None."""
    line = re.sub(r"^\s*(?:#!?\[[^\]]*\]\s*)+", "", line)
    m = DECL.match(line)
    if m:
        kind = m.group(1)
        if kind in ("fn", "func", "fun"):
            kind = "fn"
        return f"{kind} {m.group(2).strip('`')}"
    m = IMPL.match(line)
    if m:
        return "impl " + " ".join(m.group(1).split())
    m = BARE.match(line)
    if m:
        return m.group(1)
    m = VARIANT.match(line)
    if m:
        return f"variant {m.group(1)}"
    m = FIELD.match(line)
    if m:
        return f"field {m.group(1)}"
    return None


def _past_attributes(lines, i):
    """The line after the attributes or annotations starting at line i, or
    None when the item itself follows them on the same line (`@Test func x`,
    `#[test] fn x`)."""
    depth = 0
    while i < len(lines):
        rest = lines[i].strip()
        while True:
            if depth == 0:
                m = re.match(r"#!?\[|@[\w.:]+", rest)
                if not m:
                    break
                rest = rest[m.end():]
                if m.group().startswith("#"):
                    depth = 1
                elif rest.startswith("("):
                    depth, rest = 1, rest[1:]
                else:
                    rest = rest.lstrip()
                    continue
            # inside brackets: walk to the matching close
            j = 0
            while j < len(rest) and depth:
                if rest[j] in "([{":
                    depth += 1
                elif rest[j] in ")]}":
                    depth -= 1
                j += 1
            rest = rest[j:].lstrip()
            if depth:
                break
        if depth:
            i += 1
            continue
        if rest and not rest.startswith("//"):
            return None
        return i + 1
    return i


def _norm(text):
    return " ".join(text.split())


def parse(text):
    """Every documented item as (key, doc lines, line number), plus every key
    that appears at all.

    A line walk rather than a parser, because it only needs the line after a
    doc comment and a name from it, and a check that needs a toolchain for
    three languages is a check nobody runs.
    """
    lines = text.splitlines()
    documented = []
    keys = collections.Counter()
    i, n = 0, len(lines)
    while i < n:
        line = lines[i]
        doc = []
        start = i
        while i < n:
            line = lines[i]
            if DOC_LINE.match(line):
                doc.append(_norm(line.split("///", 1)[1]))
                i += 1
            elif BLOCK_OPEN.match(line):
                body = line.split("/**", 1)[1]
                chunk = []
                while True:
                    if "*/" in body:
                        chunk.append(body.split("*/", 1)[0])
                        break
                    chunk.append(body)
                    i += 1
                    if i >= n:
                        break
                    body = lines[i]
                i += 1
                for c in chunk:
                    c = c.strip()
                    if c.startswith("*"):
                        c = c[1:]
                    doc.append(_norm(c))
            elif doc and (not line.strip() or line.lstrip().startswith("//")):
                i += 1
            elif doc and (RUST_ATTR.match(line) or ANNOTATION.match(line)):
                after = _past_attributes(lines, i)
                if after is None:
                    break  # the item is on the attribute's own line
                i = after
            else:
                break
        if i >= n:
            break
        line = lines[i]
        key = item_key(line)
        if key:
            keys[key] += 1
        doc = [d for d in doc if d]
        if doc:
            documented.append((key or "line " + _norm(line)[:60], tuple(doc), start + 1))
        i += 1
    return documented, keys


def _contains(doc, part):
    k = len(part)
    return any(doc[j:j + k] == part for j in range(len(doc) - k + 1))


def compare(old, new):
    """Findings for one file: (old owner, new owner, doc, line in new)."""
    old_docs, old_keys = parse(old)
    new_docs, new_keys = parse(new)
    findings = []
    seen = set()
    for key, doc, _ in old_docs:
        if doc in seen:
            continue
        seen.add(doc)
        before = collections.Counter(k for k, d, _ in old_docs if _contains(d, doc))
        after = collections.Counter(k for k, d, _ in new_docs if _contains(d, doc))
        lost = before - after
        gained = after - before
        if not gained:
            continue
        for owner in lost:
            if owner not in new_keys:
                continue  # renamed or removed: the doc went with it
            if sum(1 for k, _, _ in new_docs if k == owner) >= new_keys[owner]:
                continue  # given a doc of its own: a deliberate move
            for taker in gained:
                if old_keys[taker] > sum(1 for k, _, _ in old_docs if k == taker):
                    continue  # it had no doc before: this is an orphan being fixed
                line = next(ln for k, d, ln in new_docs if k == taker and _contains(d, doc))
                findings.append((owner, taker, doc, line))
    return findings


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout


def blobs(specs):
    """Each `rev:path` as text, or None where it doesn't exist, from one git
    process rather than one per file: a hook that spawns two per file costs
    seconds on a wide commit."""
    if not specs:
        return []
    r = subprocess.run(
        ["git", "cat-file", "--batch"],
        input="".join(s + "\n" for s in specs).encode(),
        capture_output=True,
        check=True,
    )
    out, at, texts = r.stdout, 0, []
    for _ in specs:
        end = out.index(b"\n", at)
        header = out[at:end]
        at = end + 1
        if header.endswith((b" missing", b" ambiguous")):
            texts.append(None)
            continue
        size = int(header.rsplit(b" ", 1)[1])
        texts.append(out[at:at + size].decode("utf-8", "replace"))
        at += size + 1
    return texts


def changed(base, head, staged):
    if staged:
        out = git("diff", "--cached", "--name-only", "--diff-filter=M", "-z", base)
    elif head:
        out = git("diff", "--name-only", "--diff-filter=M", "-z", base, head)
    else:
        out = git("diff", "--name-only", "--diff-filter=M", "-z", base)
    return [p for p in out.split("\0") if p.endswith(EXTENSIONS)]


def run(base, head, staged):
    root = git("rev-parse", "--show-toplevel").strip()
    paths = changed(base, head, staged)
    olds = blobs([f"{base}:{p}" for p in paths])
    if staged:
        news = blobs([f":{p}" for p in paths])
    elif head:
        news = blobs([f"{head}:{p}" for p in paths])
    else:
        news = []
        for p in paths:
            try:
                with open(f"{root}/{p}", encoding="utf-8", errors="replace") as f:
                    news.append(f.read())
            except FileNotFoundError:
                news.append(None)
    total = 0
    for path, old, new in zip(paths, olds, news):
        if old is None or new is None:
            continue
        for owner, taker, doc, line in compare(old, new):
            total += 1
            print(f"{path}:{line}: the doc comment on `{owner}` now sits on `{taker}`")
            print(f"    /// {doc[0]}")
    if total:
        sys.stdout.flush()
        print(
            f"\n{total} doc comment(s) moved onto an item they don't describe. This is\n"
            "what an insertion anchored on a signature does: the new item lands\n"
            "between the old one and its doc. Move the new item below the closing\n"
            "brace of the one above instead. If the move was deliberate, commit\n"
            "with --no-verify and say so in the message.",
            file=sys.stderr,
        )
        return 1
    return 0


# The self-test's fixtures. Each is (name, old, new, expected findings as
# (old owner, new owner) pairs).
CASES = [
    (
        "rust: a test inserted between another test and its doc",
        """
    /// A key that starts with '-' is a key.
    #[test]
    fn dash_key_is_a_key() {}
""",
        """
    /// A key that starts with '-' is a key.
    #[test]
    fn empty_key_is_no_key() {}

    #[test]
    fn dash_key_is_a_key() {}
""",
        [("fn dash_key_is_a_key", "fn empty_key_is_no_key")],
    ),
    (
        "rust: the inserted item brought a doc of its own, so the runs touch",
        """
/// Drop everything held for a terminal whose record is gone.
pub fn forget(&mut self, id: Id) {}
""",
        """
/// Drop everything held for a terminal whose record is gone.
/// Whether the pane left agent mode.
fn left_agent_mode(&self) -> bool { true }

pub fn forget(&mut self, id: Id) {}
""",
        [("fn forget", "fn left_agent_mode")],
    ),
    (
        "rust: the fix, the new item below the old one's closing brace",
        """
/// Drop everything held for a terminal whose record is gone.
pub fn forget(&mut self, id: Id) {}
""",
        """
/// Drop everything held for a terminal whose record is gone.
pub fn forget(&mut self, id: Id) {}

/// Whether the pane left agent mode.
fn left_agent_mode(&self) -> bool { true }
""",
        [],
    ),
    (
        "rust: a rename takes its doc along",
        "/// Hello.\nfn old_name() {}\n",
        "/// Hello.\nfn new_name() {}\n",
        [],
    ),
    (
        "rust: an edited doc is not a moved one",
        "/// Hello.\nfn a() {}\n",
        "/// Hello, world.\nfn a() {}\n",
        [],
    ),
    (
        "rust: an enum variant inserted above a documented one",
        "enum E {\n    /// Still running.\n    Busy,\n}\n",
        "enum E {\n    /// Still running.\n    Idle,\n    Busy,\n}\n",
        [("variant Busy", "variant Idle")],
    ),
    (
        "rust: a doc shared by two items, one of them new, is not a move",
        "/// The id.\nfn a() {}\n",
        "/// The id.\nfn b() {}\n\n/// The id.\nfn a() {}\n",
        [],
    ),
    (
        "swift: a func inserted between a doc and its func",
        """
    /// Reads the pane's task from the workspace list.
    @MainActor
    func taskId(for pane: Pane) -> String? { nil }
""",
        """
    /// Reads the pane's task from the workspace list.
    @MainActor
    func refresh() async {}

    @MainActor
    func taskId(for pane: Pane) -> String? { nil }
""",
        [("fn taskId", "fn refresh")],
    ),
    (
        "swift: an annotation on the same line as the item",
        "/// Runs.\n@Test func runs() {}\n",
        "/// Runs.\n@Test func walks() {}\n@Test func runs() {}\n",
        [("fn runs", "fn walks")],
    ),
    (
        "swift: `@Published private(set) var` is the item, not an annotation above it",
        "/// A.\n@Published private(set) var a = 0\nlet t = 1\n",
        "/// A.\n@Published private(set) var a = 0\n/// E.\nvar e = 0\nlet t = 1\n",
        [],
    ),
    (
        "swift: an attribute and a plain comment between a doc and its func",
        "/// Run a command.\n@discardableResult\nfunc run() {}\n",
        "/// Run a command.\n@discardableResult\n// ---- changes ----\n\nfunc changesJSON() {}\n\nfunc run() {}\n",
        [("fn run", "fn changesJSON")],
    ),
    (
        "rust: a multi-line attribute, and one sharing the item's line",
        "/// S.\n#[cfg_attr(\n    test,\n    derive(Debug)\n)]\npub struct S;\n\n/// T.\n#[derive(Debug)] pub struct T;\n",
        "/// S.\n#[cfg_attr(\n    test,\n    derive(Debug)\n)]\npub struct R;\npub struct S;\n\n/// T.\n#[derive(Debug)] pub struct T;\n",
        [("struct S", "struct R")],
    ),
    (
        "kotlin: a KDoc block with a fun inserted under it",
        """
    /**
     * The card's clock, stopped in the background.
     */
    @Composable
    fun CardClock(since: Long) {}
""",
        """
    /**
     * The card's clock, stopped in the background.
     */
    @Composable
    fun StaleBorder(since: Long) {}

    @Composable
    fun CardClock(since: Long) {}
""",
        [("fn CardClock", "fn StaleBorder")],
    ),
    (
        "kotlin: a single-line KDoc on a val",
        "/** The wake time. */\nval wake = 0\n",
        "/** The wake time. */\nval sleep = 0\nval wake = 0\n",
        [("val wake", "val sleep")],
    ),
    (
        "a doc handed to a new item on purpose, the old one given its own",
        "/// SwiftUI wrapper around the terminal.\nstruct TerminalSurface: View {}\n",
        "/// SwiftUI wrapper around the terminal.\nstruct TerminalCanvas {}\n\n"
        "/// A terminal pane, and whatever floats over it.\nstruct TerminalSurface: View {}\n",
        [],
    ),
    (
        "the commit that fixes an orphan is not one",
        "/// Drop everything held.\nfn left_agent_mode() {}\n\nfn forget() {}\n",
        "fn left_agent_mode() {}\n\n/// Drop everything held.\nfn forget() {}\n",
        [],
    ),
    (
        "rust: a doc lands on a field inserted above the documented one",
        "struct S {\n    /// When it started.\n    pub started: i64,\n}\n",
        "struct S {\n    /// When it started.\n    pub ended: i64,\n    pub started: i64,\n}\n",
        [("field started", "field ended")],
    ),
    (
        "a moved block of items keeps its docs",
        "/// A.\nfn a() {}\n\n/// B.\nfn b() {}\n",
        "/// B.\nfn b() {}\n\n/// A.\nfn a() {}\n",
        [],
    ),
]


def self_test():
    failed = 0
    for name, old, new, expected in CASES:
        got = [(o, t) for o, t, _, _ in compare(old, new)]
        if got != expected:
            failed += 1
            print(f"FAIL {name}\n  expected {expected}\n  got      {got}")
        else:
            print(f"ok   {name}")
    return 1 if failed else 0


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--base", help="the commit to compare against (default: HEAD, staged changes)")
    p.add_argument("--head", help="with --base, compare against this commit instead of the working tree")
    p.add_argument("--self-test", action="store_true")
    args = p.parse_args()
    if args.self_test:
        return self_test()
    if args.head and not args.base:
        p.error("--head needs --base")
    if args.base:
        return run(args.base, args.head, staged=False)
    return run("HEAD", None, staged=True)


if __name__ == "__main__":
    sys.exit(main())
