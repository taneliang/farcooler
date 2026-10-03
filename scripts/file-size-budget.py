#!/usr/bin/env python3
"""No source file grows past 1,500 lines (ov-169).

The code review (ov-102, root cause 5) found the hardest code to change living
in a handful of god files: the daemon's service.rs at 12.5k lines, watch.rs at
8.9k, the Mac's ContentView.swift at 4.7k. Nothing stopped any of them growing,
so every feature added another screen to the one file it touched.

The rule:

  - a Rust, Swift, Kotlin or TypeScript source file is at most 1,500 lines;
  - a file already over that on the day this landed is grandfathered in
    scripts/file-size-budget.txt with a ceiling: its size then plus a
    headroom of max(2%, 50 lines). A bug fix may add a few lines; a feature
    can't keep piling in. It may grow up to its ceiling, never past it;
  - when one shrinks, `--update` lowers its ceiling to the new size plus
    headroom, so the lines it gave up can't quietly come back. `--update`
    never raises a ceiling and never adds an entry: growing past it means
    splitting. A new file gets no headroom: 1,500 is its limit.

What counts: every tracked or untracked-but-not-ignored source file, except

  - tests: a file under a tests/test/Tests/androidTest/*UITests directory, or
    named *Tests.swift, *Test.kt, *.test.ts. A test file is a flat list of
    independent cases that nobody reads top to bottom, and its length tracks
    coverage, which this check shouldn't push against. A Rust file's inline
    `#[cfg(test)] mod tests` does count: it's in the file you open to change
    the code, and moving it to a sibling tests.rs is itself a split.
  - vendor/: copied in from upstream (regen-backend-types.sh), not written here.

  ./scripts/file-size-budget.py              check the tree; exit 1 on a breach
  ./scripts/file-size-budget.py --update     lower grandfathered ceilings to
                                             size plus headroom, drop the
                                             files now under budget or gone
  ./scripts/file-size-budget.py --self-test  check the rule against known cases
  ./scripts/file-size-budget.py --seed       write the first manifest (once;
                                             refuses if one exists)
"""

import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "scripts" / "file-size-budget.txt"
BUDGET = 1500

SOURCE_SUFFIXES = {".rs", ".swift", ".kt", ".kts", ".ts", ".tsx", ".mts", ".cts"}
TEST_DIRS = {"tests", "test", "Tests", "androidTest"}
TEST_SUFFIXES = ("Tests.swift", "Test.kt", ".test.ts", ".test.tsx", ".spec.ts")
SKIP_PREFIXES = ("vendor/",)

HEADER = """\
# Source files over the 1,500-line budget when it landed (ov-169), each with a
# ceiling: its size then plus max(2%, 50 lines) of headroom. A file may grow
# to its ceiling, never past it. Lower ceilings with
# `./scripts/file-size-budget.py --update` after a split; never raise one by
# hand. Format: <ceiling> <path>
"""


def headroom(n: int) -> int:
    """Room a grandfathered file of n lines has to grow: max(2%, 50)."""
    return max(-(-n * 2 // 100), 50)


def ceiling(n: int) -> int:
    return n + headroom(n)


def counted(path: str) -> bool:
    """Whether the budget applies to this repo-relative path."""
    p = pathlib.PurePosixPath(path)
    if p.suffix not in SOURCE_SUFFIXES:
        return False
    if path.startswith(SKIP_PREFIXES):
        return False
    parts = p.parts[:-1]
    if any(part in TEST_DIRS or part.endswith("UITests") for part in parts):
        return False
    if p.name.endswith(TEST_SUFFIXES):
        return False
    return True


def tree_files() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT, check=True, capture_output=True,
    ).stdout.decode()
    return sorted({f for f in out.split("\0") if f and counted(f)})


def line_count(path: pathlib.Path) -> int:
    with open(path, "rb") as f:
        return sum(1 for _ in f)


def sizes() -> dict[str, int]:
    found = {}
    for rel in tree_files():
        full = ROOT / rel
        if full.is_file():  # a deleted-but-still-indexed file isn't there
            found[rel] = line_count(full)
    return found


def read_manifest(text: str) -> dict[str, int]:
    entries = {}
    for n, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        count, _, path = line.partition(" ")
        if not count.isdigit() or not path:
            raise SystemExit(f"{MANIFEST.name}:{n}: expected '<max lines> <path>', got {raw!r}")
        entries[path.strip()] = int(count)
    return entries


def write_manifest(entries: dict[str, int]) -> str:
    rows = sorted(entries.items(), key=lambda kv: (-kv[1], kv[0]))
    return HEADER + "".join(f"{n} {p}\n" for p, n in rows)


def breaches(found: dict[str, int], allowed: dict[str, int]) -> list[str]:
    """One message per file over its limit, each saying what to do."""
    msgs = []
    for path, n in sorted(found.items()):
        if path in allowed:
            cap = allowed[path]
            if n > cap:
                msgs.append(
                    f"{path}: {n:,} lines, past its grandfathered ceiling of {cap:,}.\n"
                    f"  This file is already over the {BUDGET:,}-line budget and has used its headroom.\n"
                    f"  Move at least {n - cap} line{'s' if n - cap != 1 else ''} out into a\n"
                    f"  new file: a type, a handler group or an inline test module that\n"
                    f"  stands on its own. Then run ./scripts/file-size-budget.py --update\n"
                    f"  if it shrinks. Don't raise the number in\n"
                    f"  {MANIFEST.relative_to(ROOT)}: that is the growth this check exists to stop."
                )
        elif n > BUDGET:
            msgs.append(
                f"{path}: {n:,} lines, over the {BUDGET:,}-line budget.\n"
                f"  Split it before it lands: move a type, a group of related functions or\n"
                f"  its inline tests into a sibling file, so each part is at most {BUDGET:,}.\n"
                f"  New files aren't grandfathered; only those over the budget when it\n"
                f"  landed are listed in {MANIFEST.relative_to(ROOT)}."
            )
    return msgs


def updated(found: dict[str, int], allowed: dict[str, int]) -> dict[str, int]:
    """Ceilings lowered to size plus headroom; entries for files now under
    budget or gone are dropped. Never raises one, never adds one."""
    out = {}
    for path, cap in allowed.items():
        if path not in found:
            continue
        if found[path] > BUDGET:
            out[path] = min(ceiling(found[path]), cap)
    return out


def check() -> int:
    allowed = read_manifest(MANIFEST.read_text())
    found = sizes()
    msgs = breaches(found, allowed)
    for m in msgs:
        print(m, end="\n\n", file=sys.stderr)
    if msgs:
        print(f"file-size-budget: {len(msgs)} file(s) over budget.", file=sys.stderr)
        return 1
    slack = [p for p, cap in allowed.items()
             if p in found and (found[p] <= BUDGET or ceiling(found[p]) < cap)]
    stale = [p for p in allowed if p not in found]
    if slack or stale:
        # Not a failure: a shrink is good. But lock it in.
        print(f"file-size-budget: ok. {len(slack) + len(stale)} grandfathered "
              f"entr{'y has' if len(slack) + len(stale) == 1 else 'ies have'} room to lower; "
              f"run ./scripts/file-size-budget.py --update and commit the manifest.")
    else:
        print(f"file-size-budget: ok, {len(found)} files, {len(allowed)} grandfathered.")
    return 0


def update() -> int:
    allowed = read_manifest(MANIFEST.read_text())
    new = updated(sizes(), allowed)
    MANIFEST.write_text(write_manifest(new))
    for path in sorted(allowed):
        if path not in new:
            print(f"dropped {path} (now under budget or gone)")
        elif new[path] < allowed[path]:
            print(f"lowered {path}: {allowed[path]} -> {new[path]}")
    return 0


def self_test() -> int:
    fails = []

    def expect(cond, what):
        if not cond:
            fails.append(what)

    # What's counted.
    for path, want in [
        ("crates/daemon/src/service.rs", True),
        ("apps/macos/Sources/FarCooler/ContentView.swift", True),
        ("apps/android/app/src/main/java/com/farcooler/net/Connection.kt", True),
        ("services/relay/src/index.ts", True),
        ("apps/android/app/build.gradle.kts", True),
        ("crates/daemon/tests/rpc_over_socket.rs", False),
        ("services/relay/test/relay.test.ts", False),
        ("apps/macos/Tests/CeremonyTests/BoardSidebarTests.swift", False),
        ("apps/ios/FarCoolerUITests/Flow.swift", False),
        ("apps/android/app/src/test/java/com/farcooler/X.kt", False),
        ("apps/android/app/src/androidTest/java/com/farcooler/X.kt", False),
        ("apps/shared/AgentKit/Sources/AgentKit/FooTests.swift", False),
        ("vendor/claude-sdk.d.ts", False),
        ("scripts/copy-lint.py", False),
        ("docs/README.md", False),
    ]:
        expect(counted(path) == want, f"counted({path!r}) should be {want}")

    # Headroom: max(2%, 50), rounded up.
    expect(headroom(1501) == 50, "headroom of a small file is 50")
    expect(headroom(12509) == 251, "headroom of 12,509 lines is 2%, rounded up")
    expect(ceiling(3000) == 3060, "ceiling of 3,000 is 3,060")
    allowed = {"big.rs": ceiling(3000)}
    # Growth within the headroom passes, to the line; one past the ceiling fails.
    expect(not breaches({"big.rs": 3000}, allowed), "grandfathered at its size should pass")
    expect(not breaches({"big.rs": 3060}, allowed), "growth to the ceiling should pass")
    expect(not breaches({"big.rs": 2000}, allowed), "grandfathered and shrunk should pass")
    grew = breaches({"big.rs": 3061}, allowed)
    expect(len(grew) == 1 and "ceiling" in grew[0], "growth past the ceiling should fail")
    expect(grew and "new file" in grew[0], "growth message should say to split")
    # A new file: at the budget passes, one over fails, and says to split.
    expect(not breaches({"new.rs": BUDGET}, {}), "a new file at the budget should pass")
    new = breaches({"new.rs": BUDGET + 1}, {})
    expect(len(new) == 1 and "Split it" in new[0], "a new file over budget should fail with a split hint")
    # --update lowers, drops, never raises, never adds.
    expect(updated({"big.rs": 2500}, allowed) == {"big.rs": 2550}, "update should lower to size plus headroom")
    expect(updated({"big.rs": 3040}, allowed) == {"big.rs": 3060}, "update must not raise, inside the headroom")
    expect(updated({"big.rs": 3500}, allowed) == {"big.rs": 3060}, "update must not raise, past the ceiling")
    expect(updated({"big.rs": 1200}, allowed) == {}, "update should drop a file under budget")
    expect(updated({}, allowed) == {}, "update should drop a file that's gone")
    expect(updated({"big.rs": 2500, "new.rs": 9000}, allowed) == {"big.rs": 2550}, "update must not add")
    # The manifest round-trips.
    expect(read_manifest(write_manifest({"a b.rs": 1600, "c.rs": 2000})) == {"a b.rs": 1600, "c.rs": 2000},
           "manifest should round-trip")

    for f in fails:
        print(f"FAIL: {f}", file=sys.stderr)
    if fails:
        return 1
    print("file-size-budget self-test: ok")
    return 0


def seed() -> int:
    """Write the first manifest from the tree as it stands. Used once, when
    the budget landed; refuses to run over an existing manifest."""
    if MANIFEST.exists():
        raise SystemExit(f"{MANIFEST.name} exists; seeding again would grandfather new growth.")
    over = {p: ceiling(n) for p, n in sizes().items() if n > BUDGET}
    MANIFEST.write_text(write_manifest(over))
    print(f"grandfathered {len(over)} files")
    return 0


def main(argv: list[str]) -> int:
    if argv == ["--self-test"]:
        return self_test()
    if argv == ["--update"]:
        return update()
    if argv == ["--seed"]:
        return seed()
    if not argv:
        return check()
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
