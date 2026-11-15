#!/usr/bin/env python3
"""The daemon's layers only depend downward (ov-1272, step 1 of ov-1219's split).

ov-1219 proposes splitting crates/daemon into git, access, agents, engine, board,
service, watch and delivery packages under a thin composition daemon (rpc, wire,
main). A package cannot depend on the daemon that composes it, nor on a sibling
that depends back, so the split only works if the modules already point one way.
Nothing here moves code. This records which layer each module belongs to
(scripts/daemon-layers.json), which layers each may use, and checks that no
`crate::module` reference leaves that direction.

Today's references that do leave it are listed in scripts/daemon-layers-allow.txt,
one `file -> module` per line, and the list only shrinks:

  - a violation not on the list fails (a new forbidden edge);
  - a listed violation that no longer exists fails (delete the line: the edge is cleared);
  - `--allowlist-not-grown REF` fails when the list holds a line REF's does not,
    so a pull request cannot make a new violation pass by writing it down.

A module that no layer claims fails too, so a new file must pick its layer.
The scanner reads text, so it fails closed (ov-1276): a construct it cannot resolve with
certainty is itself an error, named by file and line, unless listed as `file -> unresolved:
construct`. These are an alias of the crate root or of a layer module (`use crate as x`,
`use crate::rpc as y`, `use super as z`, `extern crate self as x`), a glob import of the
root or of another layer's module, and macro-built paths (`$crate::`, `$x::`, `use $p`,
a `:path` argument, `include!`).
Comments are not references. Tests that sit beside a module (`x_tests.rs`) are
part of its layer, which is the point: a unit test that still needs the whole
`service` has to move to the integration package before its layer can split.

  ./scripts/daemon-layers.py                          check the tree; exit 1 on drift
  ./scripts/daemon-layers.py --self-test              plant each fault, see it caught
  ./scripts/daemon-layers.py --allowlist-not-grown origin/main
  ./scripts/daemon-layers.py --prune                  drop cleared lines (never adds)
  ./scripts/daemon-layers.py --write-initial          write the list once, when none exists
  ./scripts/daemon-layers.py --report                 violations per layer, for the report
"""

import collections
import json
import pathlib
import re
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
CONFIG = "scripts/daemon-layers.json"
ALLOW = "scripts/daemon-layers-allow.txt"
SRC = "crates/daemon/src"

ALLOW_HEADER = """\
# Today's forbidden daemon layer edges (ov-1272, ov-1276): `file -> module`, or
# `file -> unresolved: construct` for what the scanner refuses to guess at.
# A line here is debt: it can only be deleted, never added. The check fails when a
# listed edge no longer exists (delete the line) or an unlisted one appears.
# Which of ov-1219's steps clears each is in .claude/agent/reports/ov-1276/README.md.
"""


# ---- configuration ---------------------------------------------------------------------------

class Layers:
    def __init__(self, cfg):
        self.allowed = {layer: set(uses) for layer, uses in cfg["allowed"].items()}
        self.owner = {}
        for layer, names in cfg["modules"].items():
            for name in names:
                self.owner[name] = layer
        for name in cfg["composition"]:
            self.owner[name] = "composition"
        self.layers = set(self.owner.values())

    def of_module(self, module):
        """The layer of a top-level module, or of a `_tests` sibling by its longest owner prefix."""
        if module in self.owner:
            return self.owner[module]
        best = max((n for n in self.owner if module.startswith(n + "_")), key=len, default=None)
        return self.owner[best] if best else None

    def of_file(self, rel):
        """The layer of a file under crates/daemon/src (rel is relative to it)."""
        parts = pathlib.PurePosixPath(rel).parts
        stem = parts[0].removesuffix(".rs")
        if stem == "watch":
            second = parts[1] if len(parts) > 1 else ""
            if second.startswith(("answer_wake", "task_notice")):
                return "delivery"
        return self.of_module(stem)

    def of_segs(self, segs):
        """The layer of a module path from the crate root: a watch module named for delivery is delivery."""
        if segs[0] == "watch" and len(segs) > 1 and segs[1].startswith(DELIVERY_PREFIXES):
            return "delivery"
        return self.of_module(segs[0])

    def problems(self):
        """Faults in the table itself: an unknown layer, a cycle, a module claimed twice."""
        found = []
        known = set(self.allowed) | {"composition"}
        for layer, uses in self.allowed.items():
            for use in uses:
                if use not in self.allowed:
                    found.append(f"{layer} may use {use}, which is not a layer")
        for layer in self.layers - known:
            found.append(f"modules are assigned to {layer}, which has no `allowed` entry")
        state = {}

        def walk(layer, path):
            if state.get(layer) == 2:
                return
            if state.get(layer) == 1:
                found.append("allowed layers form a cycle: " + " -> ".join(path + [layer]))
                return
            state[layer] = 1
            for use in sorted(self.allowed.get(layer, ())):
                walk(use, path + [layer])
            state[layer] = 2

        for layer in sorted(self.allowed):
            walk(layer, [])
        return found


# ---- reading Rust ----------------------------------------------------------------------------

_TOKEN = re.compile(
    r"//[^\n]*|/\*.*?\*/|(?<!\w)b?r(#*)\".*?\"\1|b?\"(?:\\.|[^\"\\])*\"|(?<!\w)b?'(?:\\.|[^\\'\n])'", re.S)
_START = re.compile(r"(?<![\w.$])(?<!::)(?:crate|self|super)::")
_MOD_OPEN = re.compile(r"\bmod\s+([A-Za-z_]\w*)\s*\{")
_MOD_DECL = re.compile(
    r"((?:#\[[^\]]*\]\s*)*)(?:pub(?:\([^)]*\))?\s+)?\bmod\s+([A-Za-z_]\w*)\s*;")
_IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
_AS = re.compile(r"as\s+([A-Za-z_][A-Za-z0-9_]*)")
_USE = re.compile(r"(?<![\w.$])use\s+")
_EXTERN_SELF = re.compile(r"\bextern\s+crate\s+self\s+as\s+([A-Za-z_]\w*)")
DELIVERY_PREFIXES = ("answer_wake", "task_notice")


def mask(text):
    """The text with comments, strings and chars blanked to spaces, so positions still line up.

    Prose or a string naming `crate::x` is not a reference, and a brace in a string is not a block.
    """
    return _TOKEN.sub(lambda m: re.sub(r"[^\n]", " ", m.group(0)), text)


def mod_blocks(masked):
    """(open, close, name) of every inline `mod name { ... }`, by position."""
    opens = {m.end() - 1: m.group(1) for m in _MOD_OPEN.finditer(masked)}
    stack, blocks = [], []
    for m in re.finditer(r"[{}]", masked):
        if m.group() == "{":
            stack.append((m.start(), opens.get(m.start())))
        elif stack:
            o, name = stack.pop()
            if name:
                blocks.append((o, m.start(), name))
    return sorted(blocks)


def chain_at(blocks, pos):
    """The inline modules enclosing pos, outermost first."""
    return [name for o, c, name in blocks if o < pos < c]


def skip_ws(text, i):
    while i < len(text) and text[i].isspace():
        i += 1
    return i


def parse_tree(text, i, prefix, out, aliases=None, globs=None):
    """Read a use-tree or path from text[i], adding each leaf path (a list of segments) to out.

    A leaf renamed with `as` is also added to aliases (when given) as (name, path), and the rename
    is read past so a group's later leaves are still found. A `*` leaf is also added to globs (when
    given) as (prefix, position).
    """
    i = skip_ws(text, i)
    if i < len(text) and text[i] == "{":
        i += 1
        while i < len(text):
            i = parse_tree(text, i, prefix, out, aliases, globs)
            i = skip_ws(text, i)
            if i < len(text) and text[i] == ",":
                i += 1
                continue
            if i < len(text) and text[i] == "}":
                i += 1
            break
        return i
    m = _IDENT.match(text, i)
    if not m:
        out.append(prefix)  # `*`, or a path that stops short
        if globs is not None and i < len(text) and text[i] == "*":
            globs.append((prefix, i))
        return i + 1 if i < len(text) and text[i] == "*" else i
    path = prefix + [m.group()]
    j = skip_ws(text, m.end())
    if text.startswith("::", j):
        return parse_tree(text, j + 2, path, out, aliases, globs)
    leaf = path[:-1] if m.group() == "self" and prefix else path
    out.append(leaf)
    am = _AS.match(text, j)
    if am:
        if aliases is not None and am.group(1) != "_":
            aliases.append((am.group(1), leaf))
        return am.end()
    return m.end()


def resolve(leaf, here, alias=None):
    """A leaf path as the list of segments from the crate root, or None when it leaves the crate.

    `here` is the module the path is written in: crate::a::b is a::b, `super::x` one up from here,
    `self::x` here.
    """
    if alias and leaf[0] in alias:
        return list(alias[leaf[0]]) + leaf[1:]
    if leaf[0] == "crate":
        return leaf[1:]
    if leaf[0] == "self":
        return here + leaf[1:]
    n = 0
    while n < len(leaf) and leaf[n] == "super":
        n += 1
    if n > len(here):
        return None
    return here[:len(here) - n] + leaf[n:]


def references(text, module=()):
    """Every path the code names inside the crate, resolved to segments from the crate root.

    `module` is where the file sits in the crate (its mod path); super:: and self:: are read against it,
    and against any inline `mod x { }` the reference is inside.
    """
    masked = mask(text)
    blocks = mod_blocks(masked)
    alias = aliases_of(masked, blocks, module)
    start = _START
    if alias:
        names = "|".join(sorted(map(re.escape, alias)))
        start = re.compile(r"(?<![\w.$])(?<!::)(?:crate|self|super|" + names + r")::")
    found, pos = set(), 0
    while True:
        m = start.search(masked, pos)
        if not m:
            break
        leaves = []
        end = parse_tree(masked, m.start(), [], leaves)
        here = list(module) + chain_at(blocks, m.start())
        for leaf in leaves:
            full = resolve(leaf, here, alias)
            if full:
                found.add(tuple(full))
        pos = max(end, m.end())
    return found


def aliases_of(masked, blocks, module):
    """Names the file gives to a path inside the crate: `use crate as root`, `use crate::{self as root}`,
    `use crate::watch as w`, `extern crate self as root`; each as the segments it stands for.

    An alias is read for the whole file, however deep it was declared, which can only over-report.
    """
    pending, alias = [], {}
    for m in _EXTERN_SELF.finditer(masked):
        alias[m.group(1)] = ()
    for m in _USE.finditer(masked):
        found, leaves = [], []
        parse_tree(masked, m.end(), [], leaves, found)
        here = list(module) + chain_at(blocks, m.start())
        pending += [(name, leaf, here) for name, leaf in found]
    progress = True
    while progress:
        progress = False
        for name, leaf, here in pending:
            if name in alias or not (leaf[0] in ("crate", "self", "super") or leaf[0] in alias):
                continue
            full = resolve(leaf, here, alias)
            if full is not None:
                alias[name] = tuple(full)
                progress = True
    return alias


_MACRO_CRATE = re.compile(r"\$crate\s*::")
_MACRO_FRAGMENT = re.compile(r"\$(?!crate\b)[A-Za-z_]\w*\s*::")
_MACRO_USE = re.compile(r"\buse\s+\$")
_MACRO_PATH_ARG = re.compile(r"\$[A-Za-z_]\w*\s*:\s*path\b")
_INCLUDE = re.compile(r"\binclude!\s*[(\[{]")


def names_a_layer_module(resolved, layers, mine, modules):
    """A path that is the crate root, or a top-level module (or a delivery module in watch) of another layer.

    Naming one through an alias or a glob lets later paths reach it with nothing here to read them by.
    The file's own layer is no exception when the path is spelled from the crate root or an alias; only a
    relative `super`/`self` path into the file's own layer is let through (see relative_own), since
    `use super::*` in a test module is everywhere and reaches no other layer.
    """
    if resolved is None:
        return False
    if not resolved:
        return True
    if resolved[0] not in modules:
        return False
    if len(resolved) > 2 or (len(resolved) == 2 and not (
            resolved[0] == "watch" and resolved[1].startswith(DELIVERY_PREFIXES))):
        return False
    return True

def relative_own(leaf, resolved, layers, mine):
    """`super::*` or `self as x` that stays inside the file's own layer."""
    return bool(leaf) and bool(resolved) and leaf[0] in ("super", "self") and layers.of_segs(resolved) == mine


_MACRO_DEF = re.compile(r"\bmacro_rules!\s*([A-Za-z_]\w*)\s*[{(\[]")


def reaching_macros(texts, modules):
    """Names of macro_rules macros, defined in any file, whose body names a layer module or `$crate::`.

    Their expansion at a call site is invisible to the scanner, so a call is refused (see unresolved).
    """
    out = set()
    for rel, text in texts.items():
        masked = mask(text)
        for m in _MACRO_DEF.finditer(masked):
            depth, i = 1, m.end()
            while i < len(masked) and depth:
                depth += (masked[i] in "{([") - (masked[i] in "})]")
                i += 1
            body = masked[m.end():i]
            if _MACRO_CRATE.search(body) or any(r and r[0] in modules for r in references(body)):
                out.add(m.group(1))
    return out


def unresolved(text, module, layers, mine, modules, macros=()):
    """Constructs the scanner cannot follow with certainty, as (description, line) pairs.

    Each is refused outright rather than guessed at: a rename of the crate root or of a layer module,
    a glob import of a layer module, and paths a macro builds or takes from its caller.
    """
    masked = mask(text)
    blocks = mod_blocks(masked)
    alias = aliases_of(masked, blocks, module)
    found = []

    def line(pos):
        return masked.count("\n", 0, pos) + 1

    for m in _EXTERN_SELF.finditer(masked):
        found.append((f"extern crate self as {m.group(1)}", line(m.start())))
    for m in _USE.finditer(masked):
        here = list(module) + chain_at(blocks, m.start())
        leaves, renamed, globs = [], [], []
        parse_tree(masked, m.end(), [], leaves, renamed, globs)
        for name, leaf in renamed:
            if (r := resolve(leaf, here, alias)) is not None and names_a_layer_module(r, layers, mine, modules) \
                    and not relative_own(leaf, r, layers, mine):
                found.append((f"alias {'::'.join(leaf)} as {name}", line(m.start())))
        for prefix, _ in globs:
            if (r := resolve(prefix, here, alias)) is not None and names_a_layer_module(r, layers, mine, modules) \
                    and not relative_own(prefix, r, layers, mine):
                found.append((f"glob {'::'.join(prefix)}::*", line(m.start())))
    for name in sorted(macros):
        for m in re.finditer(r"(?<![\w$])" + re.escape(name) + r"!\s*[{(\[]", masked):
            found.append((f"call of {name}!, whose expansion names a layer module", line(m.start())))
    for kind, rx in (("macro path $crate::", _MACRO_CRATE), ("macro path $fragment::", _MACRO_FRAGMENT),
                     ("macro use of a $fragment", _MACRO_USE), ("macro takes a path argument", _MACRO_PATH_ARG),
                     ("include! of another file", _INCLUDE)):
        for m in rx.finditer(masked):
            found.append((kind, line(m.start())))
    return found


def module_paths(src, texts):
    """The mod path of each file (rel path -> segments), read from `mod x;` and #[path] declarations.

    A file nothing declares falls back to the module its location implies.
    """
    def implied(rel):
        parts = list(pathlib.PurePosixPath(rel).parts)
        parts[-1] = parts[-1].removesuffix(".rs")
        return parts[:-1] if parts[-1] == "mod" else parts

    paths = {"lib.rs": []} if "lib.rs" in texts else {}
    queue = list(paths)
    while queue:
        rel = queue.pop()
        text = texts[rel]
        masked = mask(text)
        blocks = mod_blocks(masked)
        d = pathlib.PurePosixPath(rel)
        base = d.parent if d.stem in ("lib", "main", "mod") else d.parent / d.stem
        for m in _MOD_DECL.finditer(masked):
            name = m.group(2)
            attr = re.search(r'path\s*=\s*"([^"]+)"', text[m.start(1):m.end(1)])
            if attr:
                cands = [(d.parent / attr.group(1)).as_posix()]
            else:
                cands = [(base / f"{name}.rs").as_posix(), (base / name / "mod.rs").as_posix()]
            child = next((c for c in cands if c in texts), None)
            if child and child not in paths:
                paths[child] = paths[rel] + chain_at(blocks, m.start()) + [name]
                queue.append(child)
    return {rel: paths.get(rel, implied(rel)) for rel in texts}


# ---- the scan --------------------------------------------------------------------------------

def load(root):
    return Layers(json.loads((root / CONFIG).read_text()))


def scan(root, layers):
    """(violations, unclaimed, refused): violations are `file -> module` strings; unclaimed are files no
    layer owns; refused are `file -> unresolved: construct` strings for what the scanner cannot follow
    (see unresolved), each with the lines it is on.

    A target is a top-level module, or `watch/answer_wake` / `watch/task_notice` for a path into delivery.
    """
    src = root / SRC
    files = sorted(src.rglob("*.rs"))
    texts = {p.relative_to(src).as_posix(): p.read_text(errors="replace") for p in files}
    modules = {rel.split("/")[0].removesuffix(".rs") for rel in texts}
    watch_subs = {rel.split("/")[1].removesuffix(".rs") for rel in texts if rel.startswith("watch/")}
    mod_of = module_paths(src, texts)
    macros = reaching_macros(texts, modules)
    violations, unclaimed, refused = {}, [], {}
    for rel in texts:
        # By where the module sits, so a test that `#[path]` hangs under another module is that module's.
        mine = layers.of_segs(mod_of[rel]) if mod_of[rel] else layers.of_file(rel)
        if mine is None:
            unclaimed.append(rel)
            continue
        if mine == "composition":
            continue
        for what, ln in unresolved(texts[rel], mod_of[rel], layers, mine, modules, macros):
            key = f"{SRC}/{rel} -> unresolved: {what}"
            refused.setdefault(key, (mine, []))[1].append(ln)
        for segs in sorted(references(texts[rel], mod_of[rel])):
            top = segs[0]
            if top not in modules:
                continue  # an item defined in lib.rs (Result, a macro), not a module
            target = top
            if top == "watch" and len(segs) > 1 and segs[1] in watch_subs:
                prefix = next((x for x in DELIVERY_PREFIXES if segs[1].startswith(x)), None)
                if prefix:
                    target = f"watch/{prefix}"
            theirs = "delivery" if target.startswith("watch/") else layers.of_module(top)
            if theirs is None or theirs == mine or theirs in layers.allowed[mine]:
                continue
            violations[f"{SRC}/{rel} -> {target}"] = (mine, theirs)
    return violations, unclaimed, refused


def read_allow(text):
    return [l.strip() for l in text.splitlines() if l.strip() and not l.lstrip().startswith("#")]


def check(root):
    """Print what is wrong; return the number of faults."""
    layers = load(root)
    faults = [f"layer table: {p}" for p in layers.problems()]
    violations, unclaimed, refused = scan(root, layers)
    allow_path = root / ALLOW
    listed = read_allow(allow_path.read_text()) if allow_path.exists() else []
    dupes = sorted(k for k, n in collections.Counter(listed).items() if n > 1)
    faults += [f"listed twice in {ALLOW}: {d}" for d in dupes]
    faults += [f"no layer claims crates/daemon/src/{f}: add its module to {CONFIG}" for f in unclaimed]
    for key in sorted(set(violations) - set(listed)):
        mine, theirs = violations[key]
        faults.append(f"forbidden edge: {key}  ({mine} may not use {theirs})")
    for key in sorted(set(refused) - set(listed)):
        mine, lines = refused[key]
        file, what = key.split(" -> ", 1)
        at = ", ".join(f"{file}:{n}" for n in sorted(set(lines)))
        faults.append(f"cannot resolve, so refused: {what[len('unresolved: '):]} at {at}  ({mine} layer; "
                      f"write a plain crate::/super::/self:: path instead, or list `{key}`)")
    for key in sorted(set(listed) - set(violations) - set(refused)):
        faults.append(f"cleared, delete it from {ALLOW}: {key}")
    for line in faults:
        print(f"daemon-layers: {line}")
    if not faults:
        print(f"daemon-layers: ok ({len(violations) + len(refused)} listed edges, none new)")
    return len(faults)


def not_grown(root, ref):
    """The list holds no line that `ref` does not: a new violation cannot be written down."""
    shown = subprocess.run(["git", "show", f"{ref}:{ALLOW}"], cwd=root, capture_output=True, text=True)
    if shown.returncode != 0:
        print(f"daemon-layers: {ref} has no list yet; nothing to hold this one to")
        return 0
    before = set(read_allow(shown.stdout))
    now = set(read_allow((root / ALLOW).read_text()))
    grown = sorted(now - before)
    for key in grown:
        print(f"daemon-layers: {ALLOW} gained a line {ref} does not have: {key}")
    if not grown:
        print(f"daemon-layers: the list did not grow against {ref}")
    return len(grown)


# ---- the report ------------------------------------------------------------------------------

def report(root):
    layers = load(root)
    violations, _, refused = scan(root, layers)
    violations = {**violations, **{k: (m, "unresolved") for k, (m, _) in refused.items()}}
    by_layer = collections.defaultdict(list)
    for key, (mine, theirs) in violations.items():
        file, module = key.split(" -> ")
        by_layer[mine].append((file, module, theirs))
    for mine in sorted(by_layer):
        rows = by_layer[mine]
        edges = collections.Counter((m, t) for _, m, t in rows)
        print(f"{mine}: {len(rows)} edges in {len({f for f, _, _ in rows})} files")
        for (module, theirs), n in sorted(edges.items(), key=lambda kv: (kv[0][1], -kv[1], kv[0][0])):
            print(f"  {theirs:<12} {module:<28} {n}")


# ---- self-test -------------------------------------------------------------------------------

FIXTURE_CONFIG = {
    "allowed": {"git": [], "access": ["git"], "engine": ["git"], "board": ["git"], "delivery": ["git"],
                "watch": ["git", "delivery"]},
    "composition": ["rpc", "lib"],
    "modules": {"git": ["git"], "access": ["people"], "engine": ["engine"], "board": ["tasks"],
                "delivery": [], "watch": ["watch"]},
}


def fixture(files, allow=None):
    tmp = pathlib.Path(tempfile.mkdtemp(prefix="daemon-layers-"))
    (tmp / "scripts").mkdir()
    (tmp / CONFIG).write_text(json.dumps(FIXTURE_CONFIG))
    if allow is not None:
        (tmp / ALLOW).write_text(allow)
    src = tmp / SRC
    for name, text in {"lib.rs": "", "git.rs": "", "people.rs": "", "engine.rs": "", "tasks.rs": "",
                       "rpc.rs": "", "watch.rs": "", **files}.items():
        (src / name).parent.mkdir(parents=True, exist_ok=True)
        (src / name).write_text(text)
    return tmp


def self_test():
    failed = 0

    def expect(name, files, allow, faults, mentions=()):
        nonlocal failed
        tmp = fixture(files, allow)
        import contextlib
        import io
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            got = check(tmp)
        shown = out.getvalue()
        ok = got == faults and all(m in shown for m in mentions)
        failed += not ok
        print(f"  {'ok  ' if ok else 'FAIL'} {name}" + ("" if ok else f"  (got {got} faults, wanted {faults}):\n{shown}"))

    print("daemon-layers self-test")
    expect("a clean tree passes", {"people.rs": "use crate::git::Repo;"}, "", 0)
    expect("a lower layer importing a higher one is refused",
           {"git.rs": "use crate::people::Key;"}, "", 1, ["forbidden edge", "git may not use access"])
    expect("an implementation importing the composition layer is refused",
           {"engine.rs": "fn f() { crate::rpc::dispatch(); }"}, "", 1, ["engine may not use composition"])
    expect("a sibling it may not use is refused",
           {"engine.rs": "use crate::tasks::Row;"}, "", 1, ["engine may not use board"])
    expect("a use-group is read, nested braces too",
           {"engine.rs": "use crate::{git::{A, B}, tasks, people::K};"}, "", 2)
    expect("a path inside the body is read", {"engine.rs": "fn f() { let _ = crate::tasks::all(); }"}, "", 1)
    expect("a comment naming a module is not a reference",
           {"engine.rs": "//! See crate::tasks.\n/* crate::rpc */\n// crate::people\nfn f() {}"}, "", 0)
    expect("an item defined in lib.rs is not a module", {"engine.rs": "use crate::Result;"}, "", 0)
    expect("a test beside a module is in that module's layer",
           {"git_tests.rs": "use crate::people::Key;"}, "", 1, ["git_tests.rs -> people"])
    expect("a nested file is in its top module's layer",
           {"git/deep.rs": "use crate::tasks::Row;"}, "", 1, ["git/deep.rs -> tasks"])
    expect("a module no layer claims is refused", {"stray.rs": ""}, "", 1, ["no layer claims"])
    expect("a watch file under answer_wake is delivery, which may not use watch",
           {"watch/answer_wake/x.rs": "use crate::watch::W;"}, "", 1, ["delivery may not use watch"])
    expect("super:: from a top-level module is read against the crate root",
           {"engine.rs": "use super::rpc::Call;"}, "", 1, ["engine.rs -> rpc"])
    expect("super::super from a nested module reaches the crate root",
           {"engine/deep.rs": "use super::super::rpc::Call;"}, "", 1, ["engine/deep.rs -> rpc"])
    expect("super:: from a nested module stays inside its own module",
           {"engine/deep.rs": "use super::rpc::Call; use super::{tasks_like, X};"}, "", 0)
    expect("a use-group after super:: is read", {"engine.rs": "use super::{rpc, git, tasks};"}, "", 2)
    expect("self:: stays inside its own module", {"engine.rs": "use self::rpc::Call;"}, "", 0)
    expect("a test hung under a module by #[path] reads super:: as that module",
           {"lib.rs": "mod engine;", "engine.rs": '#[path = "engine_tests.rs"]\nmod tests;',
            "engine_tests.rs": "use super::rpc::A; use super::super::tasks::B;"}, "", 1, ["engine_tests.rs -> tasks"])
    expect("super:: inside an inline mod is read against that file's module",
           {"engine.rs": "mod t { use super::rpc::A; use super::super::tasks::B; }"}, "", 1, ["engine.rs -> tasks"])
    expect("a brace in a string does not shift the module read",
           {"engine.rs": 'fn f() { let _ = "{ mod x {"; } mod t { use super::super::rpc::A; }'}, "", 1, ["engine.rs -> rpc"])
    expect("a path after a field colon is read", {"engine.rs": "fn f() { S { id:crate::tasks::id() } }"}, "", 1)
    expect("delivery to delivery is not an edge",
           {"watch/answer_wake/x.rs": "use crate::watch::answer_wake::Y; use super::Z; use super::super::task_notice::T;",
            "watch/task_notice.rs": ""}, "", 0)
    expect("delivery reaching watch through super:: is refused",
           {"watch/answer_wake/x.rs": "use super::super::holds::H;", "watch/holds.rs": ""}, "", 1,
           ["x.rs -> watch", "delivery may not use watch"])
    expect("a layer that may use watch may use delivery in it",
           {"watch/answer_wake.rs": "", "watch/helper.rs": "use super::answer_wake::A; use crate::watch::answer_wake::B;"},
           "", 0)
    expect("a layer that may not use delivery is refused a path into it",
           {"watch/answer_wake.rs": "", "engine.rs": "use crate::watch::answer_wake::A;"}, "", 1,
           ["engine.rs -> watch/answer_wake", "engine may not use delivery"])
    expect("a group naming watch and delivery is split by owner",
           {"watch/answer_wake.rs": "", "engine.rs": "use crate::watch::{answer_wake, holds};"}, "", 2,
           ["-> watch/answer_wake", "engine.rs -> watch  "])
    expect("a crate-root alias reaches the composition layer",
           {"engine.rs": "use crate as root; use root::rpc::Call;"}, "", 2, ["engine.rs -> rpc", "refused: alias crate as root"])
    expect("a grouped crate-root alias is read",
           {"engine.rs": "use crate::{self as root, git}; fn f() { root::rpc::go(); }"}, "", 2, ["engine.rs -> rpc", "refused: alias crate as root"])
    expect("a braced crate-root alias is read", {"engine.rs": "use {crate as r}; use r::{git, tasks};"}, "", 2,
           ["engine.rs -> tasks", "refused: alias crate as r"])
    expect("extern crate self as an alias is read", {"engine.rs": "extern crate self as me; use me::rpc::X;"}, "", 2,
           ["engine.rs -> rpc", "refused: extern crate self as me"])
    expect("an alias of an alias is followed", {"engine.rs": "use crate as a; use a as b; use b::rpc::X;"}, "", 3,
           ["engine.rs -> rpc", "refused: alias a as b"])
    expect("an allowed import through a crate-root alias is still refused for the alias",
           {"engine.rs": "use crate as root; use root::git::Repo;"}, "", 1, ["refused: alias crate as root"])
    expect("an alias of a module is followed", {"engine.rs": "use crate::tasks as t; fn f() { t::all(); }"}, "", 2,
           ["engine.rs -> tasks", "refused: alias crate::tasks as t"])
    expect("an alias that leaves the crate is not a reference",
           {"engine.rs": "use std::fmt as root; use root::rpc::X;"}, "", 0)
    expect("an alias into delivery is split by owner",
           {"watch/answer_wake.rs": "", "engine.rs": "use crate::watch as w; use w::answer_wake::A;"}, "", 3,
           ["engine.rs -> watch/answer_wake", "refused: alias crate::watch as w"])
    expect("a rename inside a group does not hide later leaves",
           {"engine.rs": "use crate::{git as g, rpc};"}, "", 2, ["engine.rs -> rpc", "refused: alias crate::git as g"])
    # Fail closed: what the scanner cannot resolve with certainty is itself an error.
    expect("scoped aliases that shadow one another are refused (ov-1272 third finding)",
           {"engine.rs": "mod a { use crate::git as root; } mod b { use crate as root; use root::rpc::Call; }"}, "", 2,
           ["refused: alias crate as root", "refused: alias crate::git as root"])
    expect("an alias of the composition layer is refused", {"engine.rs": "use crate::rpc as y;"}, "", 2,
           ["refused: alias crate::rpc as y"])
    expect("an alias of the crate root with self is refused", {"engine.rs": "use crate::{self as z};"}, "", 1,
           ["refused: alias crate as z"])
    expect("super as an alias of the crate root is refused", {"engine.rs": "use super as z;"}, "", 1,
           ["refused: alias super as z"])
    expect("super as an alias of a layer module is refused", {"engine/deep.rs": "use super::super as z;"}, "", 1,
           ["refused: alias super::super as z"])
    expect("self as an alias of a nested module's own layer root passes", {"engine/deep.rs": "use self as z;"}, "", 0)
    expect("an alias of a deeper item is no hiding place and passes", {"engine.rs": "use crate::git::Repo as R;"}, "", 0)
    expect("an alias in a nested file is refused wherever it sits",
           {"engine/deep.rs": "fn f() { use crate as root; }"}, "", 1, ["engine/deep.rs -> unresolved: alias crate as root"])
    expect("a glob of the crate root is refused", {"engine.rs": "use crate::*;"}, "", 1,
           ["refused: glob crate::*", "engine.rs:1"])
    expect("a glob of a layer module is refused, allowed layer or not", {"engine.rs": "use crate::git::*;"}, "", 1,
           ["refused: glob crate::git::*"])
    expect("a glob of the composition layer is refused", {"engine.rs": "use crate::rpc::*;"}, "", 2,
           ["refused: glob crate::rpc::*", "engine.rs -> rpc"])
    expect("a glob of a layer module inside a group is refused",
           {"engine.rs": "use crate::{git::*, tasks};"}, "", 2, ["refused: glob crate::git::*"])
    expect("a glob through super is refused where it reaches another layer",
           {"engine.rs": "mod t { use super::super::*; }"}, "", 1, ["refused: glob super::super::*"])
    expect("a glob into delivery from outside it is refused",
           {"watch/answer_wake.rs": "", "engine.rs": "use crate::watch::answer_wake::*;"}, "", 2,
           ["refused: glob crate::watch::answer_wake::*"])
    expect("a glob of the file's own module in a test module passes",
           {"engine.rs": "mod t { use super::*; }"}, "", 0)
    expect("a crate-rooted glob of the own layer's module is refused too",
           {"engine.rs": "use crate::engine::*;"}, "", 1, ["refused: glob crate::engine::*"])
    expect("a crate-rooted alias of the own layer's module is refused too",
           {"engine.rs": "use crate::engine as e; use crate::engine::*;"}, "", 2,
           ["refused: alias crate::engine as e", "refused: glob crate::engine::*"])
    expect("a macro in lib.rs that builds a forbidden edge is refused at its call",
           {"lib.rs": "macro_rules! inject { () => { use crate::rpc::Call; } } mod engine; mod rpc;",
            "engine.rs": "inject!();"}, "", 1, ["refused: call of inject!", "engine.rs:1"])
    expect("a macro that names no layer module is called freely",
           {"lib.rs": "macro_rules! plain { () => { 1 } }", "engine.rs": "fn f() { plain!(); }"}, "", 0)
    expect("a glob from a delivery file into watch is refused",
           {"watch/answer_wake/x.rs": "use super::super::*;"}, "", 2, ["refused: glob super::super::*"])
    expect("a glob of a std or other crate path is not a layer glob", {"engine.rs": "use std::collections::*;"}, "", 0)
    expect("a macro building a path from $crate is refused",
           {"engine.rs": "macro_rules! m { () => { $crate::rpc::go() }; }"}, "", 1,
           ["refused: macro path $crate::", "engine.rs:1"])
    expect("a macro building a path from a fragment is refused",
           {"engine.rs": "macro_rules! m { ($m:ident) => { $m::go() }; }"}, "", 1, ["refused: macro path $fragment::"])
    expect("a macro taking a path argument is refused",
           {"engine.rs": "macro_rules! m { ($p:path) => { fn f() { let _ = 1; } }; }"}, "", 1,
           ["refused: macro takes a path argument"])
    expect("a macro importing what its caller passes is refused",
           {"engine.rs": "macro_rules! m { ($p:tt) => { use $p; }; }"}, "", 1, ["refused: macro use of a $fragment"])
    expect("include! of another file is refused", {"engine.rs": 'include!("other.rs");'}, "", 1,
           ["refused: include! of another file"])
    expect("a macro naming a path in plain text is read as any other path",
           {"engine.rs": "macro_rules! m { () => { crate::tasks::go() }; }"}, "", 1, ["engine.rs -> tasks"])
    expect("a dollar in a string or a comment is not a macro path",
           {"engine.rs": '// $crate::x\nfn f() { let _ = "$crate::rpc $p:path include!("; }'}, "", 0)
    expect("the composition layer may alias and glob freely",
           {"rpc.rs": "use crate as root; use crate::tasks::*; macro_rules! m { () => { $crate::x }; }"}, "", 0)
    refused = "crates/daemon/src/engine.rs -> unresolved: alias crate as root\n"
    expect("a listed refused construct passes along with its edge",
           {"engine.rs": "use crate as root;"}, refused, 0)
    expect("a listed refused construct that was removed must be deleted", {"engine.rs": "fn f() {}"}, refused, 1,
           ["cleared, delete it"])
    expect("a second refused construct beside a listed one is refused",
           {"engine.rs": "use crate as root; use crate as other;"}, refused, 1, ["refused: alias crate as other"])
    expect("the composition layer may use anything", {"rpc.rs": "use crate::tasks::T; use crate::git::G;"}, "", 0)
    listed = "crates/daemon/src/engine.rs -> tasks\n"
    expect("a listed violation passes", {"engine.rs": "use crate::tasks::Row;"}, listed, 0)
    expect("a listed violation that was fixed must be deleted", {"engine.rs": "fn f() {}"}, listed, 1,
           ["cleared, delete it"])
    expect("a line for an edge that never existed is refused", {}, listed, 1, ["cleared, delete it"])
    expect("a listed line twice is refused", {"engine.rs": "use crate::tasks::Row;"}, listed + listed, 1,
           ["listed twice"])
    expect("a new edge beside a listed one is refused",
           {"engine.rs": "use crate::tasks::Row; use crate::people::K;"}, listed, 1, ["engine.rs -> people"])
    cyc = Layers({"allowed": {"a": ["b"], "b": ["a"]}, "modules": {"a": ["x"], "b": ["y"]}, "composition": []})
    ok = any("cycle" in p for p in cyc.problems())
    failed += not ok
    print(f"  {'ok  ' if ok else 'FAIL'} a cycle in the allowed layers is refused")
    # The shipped table must itself be sound.
    problems = load(ROOT).problems()
    failed += bool(problems)
    print(f"  {'ok  ' if not problems else 'FAIL'} the shipped layer table is acyclic and complete" + "".join(f"\n    {p}" for p in problems))
    # Not-grown, against a real git history.
    tmp = fixture({}, ALLOW_HEADER + "a -> b\n")
    git = lambda *a: subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *a], cwd=tmp, check=True, capture_output=True)
    git("init", "-q")
    git("add", ALLOW)
    git("commit", "-qm", "base")
    import contextlib
    import io
    with contextlib.redirect_stdout(io.StringIO()):
        same = not_grown(tmp, "HEAD")
        (tmp / ALLOW).write_text(ALLOW_HEADER + "a -> b\nc -> d\n")
        grown = not_grown(tmp, "HEAD")
        (tmp / ALLOW).write_text(ALLOW_HEADER)
        shrunk = not_grown(tmp, "HEAD")
    for name, got, want in [("an unchanged list is not growth", same, 0), ("an added line is growth", grown, 1),
                            ("a deleted line is not growth", shrunk, 0)]:
        failed += got != want
        print(f"  {'ok  ' if got == want else 'FAIL'} {name}")
    print(f"daemon-layers self-test: {'ok' if not failed else f'{failed} failed'}")
    return failed


# ---- main ------------------------------------------------------------------------------------

def main(argv):
    if "--self-test" in argv:
        return 1 if self_test() else 0
    if "--allowlist-not-grown" in argv:
        return 1 if not_grown(ROOT, argv[argv.index("--allowlist-not-grown") + 1]) else 0
    if "--report" in argv:
        report(ROOT)
        return 0
    if "--write-initial" in argv:
        if (ROOT / ALLOW).exists():
            print(f"daemon-layers: {ALLOW} exists; the list only shrinks (--prune)")
            return 1
        violations, _, refused = scan(ROOT, load(ROOT))
        violations = {**violations, **refused}
        (ROOT / ALLOW).write_text(ALLOW_HEADER + "".join(f"{k}\n" for k in sorted(violations)))
        print(f"daemon-layers: wrote {len(violations)} edges to {ALLOW}")
        return 0
    if "--prune" in argv:
        violations, _, refused = scan(ROOT, load(ROOT))
        violations = {**violations, **refused}
        text = (ROOT / ALLOW).read_text()
        kept = [l for l in text.splitlines() if not l.strip() or l.lstrip().startswith("#") or l.strip() in violations]
        (ROOT / ALLOW).write_text("\n".join(kept) + "\n")
        print(f"daemon-layers: dropped {len(read_allow(text)) - len(read_allow(chr(10).join(kept)))} cleared edges")
        return 0
    return 1 if check(ROOT) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
