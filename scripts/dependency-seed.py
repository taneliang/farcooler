#!/usr/bin/env python3
"""Build landed main's third-party crates for builders to start from (ov-1516).

The engine runs this in a private checkout of each landed commit, with `FARCOOLER_SEED` set to an
empty folder under its own root. Each Build step's `$FARCOOLER_SCRATCH` then starts as an APFS clone
of what this leaves there, so `CARGO_TARGET_DIR="$FARCOOLER_SCRATCH/target"` already holds every
third-party unit and a cold build compiles only the workspace.

Builders run cargo with no CARGO_* or RUSTFLAGS in their environment, so this does too: the units it
leaves have the same metadata hashes builders ask for. A unit built with other flags would never be
reused, only take space.

Only third-party units stay. Cargo decides whether a workspace crate is fresh by file modification
time, so a seeded workspace unit newer than a builder's checkout could stand in for changed source.
A registry crate's freshness is its version and flags, which a seed can't get wrong.
"""
from pathlib import Path
import json
import os
import re
import shutil
import subprocess
import sys

# The invocations builders run most: every test binary, and the binaries native tests launch.
# Resolver 2 unifies features over the packages a command selects, so a builder's `-p` subset can
# still need a few third-party units built with fewer features; those it compiles itself.
BUILDS = (
    ["cargo", "test", "--workspace", "--no-run", "--locked"],
    ["cargo", "build", "--workspace", "--bins", "--locked"],
)
# The unit hash cargo puts in every output name: `libserde-0123456789abcdef.rlib`, `serde-….d`,
# `.fingerprint/serde-…/`, `build/serde-…/`.
UNIT = re.compile(r"^(?P<name>.+)-(?P<hash>[0-9a-f]{16})(?:\..*)?$")
KEPT = ("deps", "build", ".fingerprint")


def cargo_environment(target: Path) -> dict:
    """The environment a builder's cargo sees, with the seed as its target."""
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("CARGO_", "RUSTFLAGS", "RUSTDOCFLAGS", "RUSTC_"))}
    environment.pop("FARCOOLER_SEED", None)
    environment["CARGO_TARGET_DIR"] = str(target)
    return environment


def workspace_packages(environment: dict) -> set:
    output = subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"],
                            env=environment, check=True, capture_output=True, text=True).stdout
    return {package["name"] for package in json.loads(output)["packages"]}


def unit(name: str):
    match = UNIT.match(name)
    return (match["name"], match["hash"]) if match else None


def strip_workspace(profile: Path, workspace: set) -> dict:
    """Leave only third-party units in `profile` (target/debug). Returns what was removed and kept."""
    fingerprints = profile / ".fingerprint"
    workspace_hashes = set()
    for entry in fingerprints.iterdir():
        parsed = unit(entry.name)
        if parsed and parsed[0] in workspace:
            workspace_hashes.add(parsed[1])
    counts = {"workspace_units": len(workspace_hashes), "removed": 0, "kept": 0}
    for entry in profile.iterdir():
        if entry.name not in KEPT:
            remove(entry)
            counts["removed"] += 1
    for folder in KEPT:
        for entry in (profile / folder).iterdir():
            parsed = unit(entry.name)
            # Anything not named for a unit is dropped: the seed holds only what it can vouch for.
            if parsed is None or parsed[1] in workspace_hashes:
                remove(entry)
                counts["removed"] += 1
            else:
                counts["kept"] += 1
    return counts


def allocated(path: Path) -> int:
    """Bytes on disk under `path`, as du counts them."""
    return sum(entry.stat(follow_symlinks=False).st_blocks * 512
               for entry in path.rglob("*")) + path.stat().st_blocks * 512


def remove(path: Path) -> None:
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
    else:
        path.unlink()


def self_test() -> int:
    """Only third-party units survive: workspace units go by their hash in every folder, as does anything
    not named for a unit."""
    import tempfile
    with tempfile.TemporaryDirectory() as scratch:
        profile = Path(scratch) / "debug"
        ours, theirs, script = "0123456789abcdef", "fedcba9876543210", "00112233445566aa"
        files = [
            f".fingerprint/farcooler-store-{ours}/lib-farcooler_store",
            f".fingerprint/farcooler-store-{script}/run-build-script",
            f".fingerprint/serde-{theirs}/lib-serde",
            f"deps/libfarcooler_store-{ours}.rlib", f"deps/farcooler_store-{ours}.d", f"deps/farcooler_store-{ours}",
            f"build/farcooler-store-{script}/output",
            f"deps/libserde-{theirs}.rlib", f"deps/libserde-{theirs}.rmeta", f"deps/serde-{theirs}.d",
            f"build/serde-{theirs}/output",
            "deps/unnamed.rlib", "incremental/farcooler_store-1/s-x", "farcooler", ".cargo-lock",
        ]
        for name in files:
            (profile / name).parent.mkdir(parents=True, exist_ok=True)
            (profile / name).write_text(name)
        counts = strip_workspace(profile, {"farcooler-store"})
        left = sorted(str(p.relative_to(profile)) for p in profile.rglob("*") if p.is_file())
        expected = sorted([f".fingerprint/serde-{theirs}/lib-serde", f"deps/libserde-{theirs}.rlib",
                           f"deps/libserde-{theirs}.rmeta", f"deps/serde-{theirs}.d", f"build/serde-{theirs}/output"])
        assert left == expected, left
        assert counts["workspace_units"] == 2 and counts["kept"] == 5, counts
    print("dependency-seed self-test passed")
    return 0


def main() -> int:
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    seed = Path(os.environ["FARCOOLER_SEED"])
    target = seed / "target"
    environment = cargo_environment(target)
    for argv in BUILDS:
        subprocess.run(argv, env=environment, check=True)
    built = allocated(target)
    counts = strip_workspace(target / "debug", workspace_packages(environment))
    counts.update(built_bytes=built, seed_bytes=allocated(target))
    for name in (target / "debug").iterdir():
        if name.name not in KEPT:
            raise SystemExit(f"unexpected seed entry left behind: {name}")
    print(json.dumps(counts))
    return 0


if __name__ == "__main__":
    sys.exit(main())
