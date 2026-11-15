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
# Today's forbidden daemon layer edges (ov-1272): `file -> module`.
# A line here is debt: it can only be deleted, never added. The check fails when a
# listed edge no longer exists (delete the line) or an unlisted one appears.
# Which of ov-1219's steps clears each is in .claude/agent/reports/ov-1272/README.md.
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

_BLOCK = re.compile(r"/\*.*?\*/", re.S)
_LINE = re.compile(r"//[^\n]*")
_CRATE = re.compile(r"\bcrate::(?:\{|([A-Za-z_][A-Za-z0-9_]*))")


def code_only(text):
    """The text without comments (doc comments included), so prose naming `crate::x` is not a reference."""
    return _LINE.sub("", _BLOCK.sub("", text))


def group_heads(text, start):
    """The first path segment of each item in the `{ ... }` opening at text[start]."""
    heads, depth, item, i = [], 0, "", start
    while i < len(text):
        c = text[i]
        if c == "{":
            depth += 1
            if depth > 1:
                item += c
        elif c == "}":
            depth -= 1
            if depth == 0:
                heads.append(item)
                break
            item += c
        elif c == "," and depth == 1:
            heads.append(item)
            item = ""
        else:
            item += c
        i += 1
    out = []
    for h in heads:
        m = re.match(r"\s*([A-Za-z_][A-Za-z0-9_]*)", h)
        if m:
            out.append(m.group(1))
    return out


def references(text):
    """Every top-level name reached through `crate::`, in a path or a `use crate::{a, b::c}` group."""
    text = code_only(text)
    found = set()
    for m in _CRATE.finditer(text):
        if m.group(1):
            found.add(m.group(1))
        else:
            found.update(group_heads(text, m.end() - 1))
    return found


# ---- the scan --------------------------------------------------------------------------------

def load(root):
    return Layers(json.loads((root / CONFIG).read_text()))


def scan(root, layers):
    """(violations, unclaimed): violations are `file -> module` strings; unclaimed are files no layer owns."""
    src = root / SRC
    modules = {p.relative_to(src).parts[0].removesuffix(".rs") for p in src.rglob("*.rs")}
    violations, unclaimed = {}, []
    for path in sorted(src.rglob("*.rs")):
        rel = path.relative_to(src).as_posix()
        mine = layers.of_file(rel)
        if mine is None:
            unclaimed.append(rel)
            continue
        if mine == "composition":
            continue
        for ref in sorted(references(path.read_text(errors="replace"))):
            if ref not in modules:
                continue  # an item defined in lib.rs (Result, a macro), not a module
            theirs = layers.of_module(ref)
            if theirs is None or theirs == mine or theirs in layers.allowed[mine]:
                continue
            violations[f"{SRC}/{rel} -> {ref}"] = (mine, theirs)
    return violations, unclaimed


def read_allow(text):
    return [l.strip() for l in text.splitlines() if l.strip() and not l.lstrip().startswith("#")]


def check(root):
    """Print what is wrong; return the number of faults."""
    layers = load(root)
    faults = [f"layer table: {p}" for p in layers.problems()]
    violations, unclaimed = scan(root, layers)
    allow_path = root / ALLOW
    listed = read_allow(allow_path.read_text()) if allow_path.exists() else []
    dupes = sorted(k for k, n in collections.Counter(listed).items() if n > 1)
    faults += [f"listed twice in {ALLOW}: {d}" for d in dupes]
    faults += [f"no layer claims crates/daemon/src/{f}: add its module to {CONFIG}" for f in unclaimed]
    for key in sorted(set(violations) - set(listed)):
        mine, theirs = violations[key]
        faults.append(f"forbidden edge: {key}  ({mine} may not use {theirs})")
    for key in sorted(set(listed) - set(violations)):
        faults.append(f"cleared, delete it from {ALLOW}: {key}")
    for line in faults:
        print(f"daemon-layers: {line}")
    if not faults:
        print(f"daemon-layers: ok ({len(violations)} listed edges, none new)")
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
    violations, _ = scan(root, layers)
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
        violations, _ = scan(ROOT, load(ROOT))
        (ROOT / ALLOW).write_text(ALLOW_HEADER + "".join(f"{k}\n" for k in sorted(violations)))
        print(f"daemon-layers: wrote {len(violations)} edges to {ALLOW}")
        return 0
    if "--prune" in argv:
        violations, _ = scan(ROOT, load(ROOT))
        text = (ROOT / ALLOW).read_text()
        kept = [l for l in text.splitlines() if not l.strip() or l.lstrip().startswith("#") or l.strip() in violations]
        (ROOT / ALLOW).write_text("\n".join(kept) + "\n")
        print(f"daemon-layers: dropped {len(read_allow(text)) - len(read_allow(chr(10).join(kept)))} cleared edges")
        return 0
    return 1 if check(ROOT) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
