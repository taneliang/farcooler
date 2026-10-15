#!/usr/bin/env python3
"""Fail when the installed claude's hook schema lacks a rewake field (ov-571).

Claude mailbox delivery (crates/daemon/src/claude_mailbox.rs, ov-563) writes a
Stop-hook entry that carries three fields, which Claude Code 2.1.300 marks
@internal:

    asyncRewake     the hook runs in the background and wakes the session
    rewakeMessage   what claude prints before the message
    rewakeSummary   the one-line label for the wake

If an update drops one, the hook still runs but claude falls back to reading
the message as "Stop hook blocking error...", and nothing in the app notices.
This looks for all three names in the hook schema text of the installed
bundle, and exits 1 naming every one that is missing.

    scripts/claude-rewake-check.py                  # the claude on PATH
    scripts/claude-rewake-check.py --bundle FILE    # a given bundle or binary
    scripts/claude-rewake-check.py --self-test      # a planted fake bundle, each field dropped in turn

It never runs claude. It only reads files: the one `claude` resolves to
and, when that is an npm shim, the package's own files (cli.js, or the
platform binary that holds the same schema text). Exit 2 means no bundle was
found or readable, which is loud on purpose: a layout change must never read
as "all fields present".
"""
import argparse
import os
import shutil
import sys
import tempfile
from pathlib import Path

FIELDS = ("asyncRewake", "rewakeMessage", "rewakeSummary")
SCAN_LIMIT = 400 * 1024 * 1024  # bytes; the native binary is about 200 MB
PACKAGE = "claude-code"


def package_roots(exe: Path):
    """npm package directories that hold (or sit beside) the resolved claude."""
    for parent in exe.parents:
        if parent.parent.name == "@anthropic-ai" and parent.name.startswith(PACKAGE):
            yield parent
            # The platform binary lives in a sibling package (claude-code-<os>-<cpu>).
            for sibling in parent.parent.glob(PACKAGE + "*"):
                if sibling != parent:
                    yield sibling
            return


def candidates(bundle: str | None) -> list[Path]:
    """Files that may hold the hook schema, most likely first."""
    if bundle:
        return [Path(bundle)]
    found = shutil.which("claude")
    if not found:
        return []
    exe = Path(found).resolve()
    files = [exe]
    for root in package_roots(exe):
        for name in ("cli.js", "cli.mjs", "bundle.js"):
            files += sorted(root.rglob(name))[:3]
        for sub in ("bin", "vendor"):
            files += [p for p in sorted((root / sub).rglob("*")) if p.is_file()][:20]
    seen, out = set(), []
    for f in files:
        if f.is_file() and f not in seen and f.stat().st_size <= SCAN_LIMIT:
            seen.add(f)
            out.append(f)
    return out


def fields_in(path: Path) -> set[str]:
    """Which FIELDS appear in the file, read in chunks so a big binary is fine."""
    wanted = {f: f.encode() for f in FIELDS}
    hit, tail = set(), b""
    overlap = max(len(w) for w in wanted.values()) - 1
    with path.open("rb") as fh:
        while chunk := fh.read(8 << 20):
            data = tail + chunk
            hit |= {f for f, w in wanted.items() if w in data}
            if len(hit) == len(FIELDS):
                break
            tail = data[-overlap:]
    return hit


def check(bundle: str | None) -> tuple[int, str]:
    """(exit code, message)."""
    files = candidates(bundle)
    if not files:
        return 2, "claude-rewake-check: no claude found on PATH, and no --bundle given."
    best, best_file = set(), None
    for f in files:
        try:
            got = fields_in(f)
        except OSError as e:
            return 2, f"claude-rewake-check: cannot read {f.name}: {e.strerror}"
        if best_file is None or len(got) > len(best):
            best, best_file = got, f
    missing = [f for f in FIELDS if f not in best]
    if missing:
        return 1, (
            "claude-rewake-check: the hook schema in claude's bundle ("
            + best_file.name
            + ") is missing "
            + ", ".join(missing)
            + ". Claude mailbox delivery (crates/daemon/src/claude_mailbox.rs) sets all of "
            + ", ".join(FIELDS)
            + "; without one, messages read as 'Stop hook blocking error'. Check the claude changelog "
            "for a replacement and update claude_mailbox.rs."
        )
    return 0, "claude-rewake-check: ok, " + ", ".join(FIELDS) + " all present in " + best_file.name + "."


def self_test() -> int:
    """A planted bundle, whole and with each field removed; also no bundle at all."""
    bad = 0
    with tempfile.TemporaryDirectory() as d:
        def plant(name, fields):
            p = Path(d) / name
            body = "\n".join(f'{f}:z.string().optional(),// @internal' for f in fields)
            p.write_bytes(b"\x00junk\x00" + body.encode() + b"\x00junk")
            return str(p)

        cases = [("whole", FIELDS, 0, [])]
        cases += [(f"without {f}", [x for x in FIELDS if x != f], 1, [f]) for f in FIELDS]
        cases += [("none of them", [], 1, list(FIELDS))]
        for name, fields, want, gone in cases:
            code, msg = check(plant(name.replace(" ", "-") + ".js", fields))
            named = msg.split("is missing")[-1].split(". Claude")[0] if want else ""
            ok = code == want and all(k in named for k in gone) and not any(
                k in named for k in FIELDS if k not in gone
            )
            print(f"  {'ok  ' if ok else 'FAIL'} {name}: exit {code} ({'red' if code else 'green'})")
            bad += not ok
        # A field name split across the read-chunk boundary is still found.
        p = Path(d) / "split.js"
        p.write_bytes(b"x" * ((8 << 20) - 5) + " ".join(FIELDS).encode())
        got = fields_in(p)
        print(f"  {'ok  ' if got == set(FIELDS) else 'FAIL'} names across a chunk boundary")
        bad += got != set(FIELDS)
        # An empty PATH finds no claude: exit 2, never a pass.
        old = os.environ.get("PATH")
        os.environ["PATH"] = d
        try:
            code, _ = check(None)
        finally:
            os.environ["PATH"] = old or ""
        print(f"  {'ok  ' if code == 2 else 'FAIL'} no claude on PATH: exit {code}")
        bad += code != 2
    print("self-test " + ("passed" if not bad else f"FAILED ({bad})"))
    return 1 if bad else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--bundle")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    code, msg = check(args.bundle)
    print(msg, file=sys.stderr if code else sys.stdout)
    return code


if __name__ == "__main__":
    sys.exit(main())
