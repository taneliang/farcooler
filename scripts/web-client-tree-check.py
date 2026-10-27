#!/usr/bin/env python3
"""Keep aws-lc and mio out of the browser's build of the client core (ov-768).

`farcooler-client --no-default-features --features web` is what a browser
runs: russh on ring, and the page's task queue for a runtime. Two crates must
never come back into that tree, because neither can work in a page:

  aws-lc-sys  russh's native crypto backend: C that needs libc headers
              wasm32-unknown-unknown does not have
  mio         tokio's I/O driver (`net`, `process`, `signal`): sockets and
              an event loop a browser does not have

Each arrives silently through one feature on one dependency, often the
workspace's `tokio` or `russh`'s defaults, so this reads the resolved tree
rather than any manifest:

    scripts/web-client-tree-check.py              # fail, naming the path, if either is there
    scripts/web-client-tree-check.py --tree FILE  # read a saved `cargo tree --prefix depth`
    scripts/web-client-tree-check.py --self-test

Exit 1 is a banned crate in the tree. Exit 2 is a tree this script cannot
read (cargo failed, or the tree does not start at farcooler-client), loud on
purpose: an empty tree must never read as a clean one.
"""
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BANNED = ("aws-lc-sys", "aws-lc-rs", "mio")
TREE_ARGS = [
    "tree", "--locked",
    "-p", "farcooler-client", "--no-default-features", "--features", "web",
    "--target", "wasm32-unknown-unknown",
    # Build dependencies too: aws-lc-sys is mostly its build script.
    "-e", "normal,build",
    "--prefix", "depth",
]
LINE = re.compile(r"^(\d+)(\S+) v(\S+)")


class Unreadable(Exception):
    pass


def banned_paths(text):
    """Each banned crate in a `--prefix depth` tree, with the path to it."""
    stack, found, seen_root = [], [], False
    for raw in text.splitlines():
        m = LINE.match(raw)
        if not m:
            continue
        depth, name = int(m.group(1)), m.group(2)
        if depth == 0:
            if name != "farcooler-client":
                raise Unreadable(f"the tree starts at {name}, not farcooler-client")
            seen_root = True
        del stack[depth:]
        stack.append(name)
        if name in BANNED:
            found.append(" -> ".join(stack))
    if not seen_root:
        raise Unreadable("no farcooler-client in the tree")
    return sorted(set(found))


def cargo_tree():
    # $CARGO, then cargo on PATH, then rustup's own: a Mac shell often has
    # ~/.cargo/bin off PATH.
    cargo = os.environ.get("CARGO") or shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo")
    try:
        run = subprocess.run([cargo, *TREE_ARGS], cwd=ROOT, capture_output=True, text=True)
    except OSError as e:
        raise Unreadable(f"cannot run {cargo}: {e}")
    if run.returncode != 0:
        raise Unreadable(f"cargo tree failed:\n{run.stderr.strip()}")
    return run.stdout


def check(text):
    try:
        found = banned_paths(text)
    except Unreadable as e:
        print(f"web-client-tree-check: {e}", file=sys.stderr)
        return 2
    if found:
        print("web-client-tree-check: the browser's client tree has crates a page cannot run:", file=sys.stderr)
        for path in found:
            print(f"  {path}", file=sys.stderr)
        print("Fix: keep the feature that pulls it in behind farcooler-client's `native` (crates/client/Cargo.toml).",
              file=sys.stderr)
        return 1
    print("web-client-tree-check: no aws-lc and no mio in farcooler-client's web tree")
    return 0


def self_test():
    clean = "0farcooler-client v0.1.0\n1russh v0.62.7\n2ring v0.17.14\n1tokio v1.53.1\n"
    with_mio = clean + "2mio v1.0.4\n"
    with_aws = "0farcooler-client v0.1.0\n1russh v0.62.7\n2aws-lc-rs v1.13.0\n3aws-lc-sys v0.29.0\n"
    cases = [
        ("clean", clean, 0),
        ("mio under tokio", with_mio, 1),
        ("aws-lc under russh", with_aws, 1),
        ("empty", "", 2),
        ("wrong root", "0farcooler-web v0.1.0\n", 2),
    ]
    failed = 0
    for name, text, want in cases:
        devnull = open(os.devnull, "w")
        out, err = sys.stdout, sys.stderr
        sys.stdout = sys.stderr = devnull
        try:
            got = check(text)
        finally:
            sys.stdout, sys.stderr = out, err
            devnull.close()
        if got != want:
            print(f"self-test {name}: exit {got}, want {want}", file=sys.stderr)
            failed += 1
    if banned_paths(with_aws) != ["farcooler-client -> russh -> aws-lc-rs", "farcooler-client -> russh -> aws-lc-rs -> aws-lc-sys"]:
        print("self-test: the path to a banned crate is wrong", file=sys.stderr)
        failed += 1
    print(f"web-client-tree-check self-test: {'FAILED' if failed else 'ok'}")
    return 1 if failed else 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    if len(argv) == 2 and argv[0] == "--tree":
        return check(Path(argv[1]).read_text())
    if argv:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        text = cargo_tree()
    except Unreadable as e:
        print(f"web-client-tree-check: {e}", file=sys.stderr)
        return 2
    return check(text)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
