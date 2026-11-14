#!/usr/bin/env python3
"""Verify the real bundle build scripts with a tiny, fully optimized workspace."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def run(args, *, cwd, env=None):
    result = subprocess.run(args, cwd=cwd, env=env, text=True, capture_output=True)
    if result.returncode:
        raise AssertionError(f"{args}:\n{result.stdout}\n{result.stderr}")
    return result.stdout + result.stderr


def main():
    # Build artifacts stay in the worktree's target/, never /tmp.
    (ROOT / "target").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="bundle-native-", dir=ROOT / "target") as scratch:
        root = Path(scratch)
        (root / "Cargo.toml").write_text('''[workspace]
members = ["cli", "daemon"]
resolver = "3"
[profile.release]
lto = "thin"
strip = false
''')
        for package in ("cli", "daemon"):
            directory = root / package
            (directory / "src").mkdir(parents=True)
            shutil.copyfile(ROOT / f"crates/{package}/build.rs", directory / "build.rs")
        (root / "daemon/Cargo.toml").write_text('''[package]
name = "farcooler-daemon"
version = "0.1.0"
edition = "2024"
[features]
tailcat = []
[[bin]]
name = "farcoolerd"
path = "src/main.rs"
[[bin]]
name = "farcooler-lfs-filter"
path = "src/filter.rs"
''')
        (root / "cli/Cargo.toml").write_text('''[package]
name = "farcooler-cli"
version = "0.1.0"
edition = "2024"
[features]
tailcat = []
[dependencies]
farcooler-daemon = { path = "../daemon" }
[[bin]]
name = "farcooler"
path = "src/main.rs"
''')
        (root / "daemon/src/lib.rs").write_text('''#[cfg(feature = "tailcat")]
unsafe extern "C" { fn tailcat_fixture() -> i32; }
pub fn answer() -> i32 {
    #[cfg(feature = "tailcat")] { unsafe { tailcat_fixture() } }
    #[cfg(not(feature = "tailcat"))] { 0 }
}
''')
        for package in ("cli", "daemon"):
            (root / package / "src/main.rs").write_text(
                'fn main() { println!("{}", farcooler_daemon::answer()); }\n')
        (root / "daemon/src/filter.rs").write_text('fn main() { println!("filter"); }\n')
        archive_dir = root / "archive with spaces"
        archive_dir.mkdir()

        def archive(answer):
            (archive_dir / "fixture.c").write_text(f"int tailcat_fixture(void) {{ return {answer}; }}\n")
            run(["cc", "-c", "fixture.c", "-o", "fixture.o"], cwd=archive_dir)
            run(["ar", "rcs", "libtailcat.a", "fixture.o"], cwd=archive_dir)

        archive(41)
        env = dict(os.environ, FARCOOLER_BUNDLE_TAILCAT_DIR=str(archive_dir))
        # CI forces ANSI color; compiler-command assertions need plain output.
        command = ["cargo", "build", "--color", "never", "--offline", "--message-format=json", "--release", "-p", "farcooler-cli",
                   "-p", "farcooler-daemon", "--bins", "--features",
                   "farcooler-cli/tailcat,farcooler-daemon/tailcat", "-vv"]
        log = run(command, cwd=root, env=env)
        compiles = [line for line in log.splitlines() if "Running `" in line and "--crate-name farcooler_daemon " in line]
        assert len(compiles) == 1, log
        assert "-C opt-level=3" in compiles[0], log
        assert "link-arg" not in compiles[0], compiles
        products = root / "target/release"
        for binary in ("farcooler", "farcoolerd"):
            assert run([str(products / binary)], cwd=root).strip() == "41"
            assert "_tailcat_fixture" in run(["nm", "-gU", str(products / binary)], cwd=root)
        assert run([str(products / "farcooler-lfs-filter")], cwd=root).strip() == "filter"
        assert "_tailcat_fixture" not in run(["nm", "-gU", str(products / "farcooler-lfs-filter")], cwd=root)
        # Cargo's artifact names/layout vary by toolchain; use its public output.
        artifacts = [json.loads(line) for line in log.splitlines() if line.startswith("{")]
        libraries = sorted({Path(name) for event in artifacts
                            if event.get("reason") == "compiler-artifact"
                            and event["target"]["name"] == "farcooler_daemon"
                            for name in event["filenames"] if name.endswith(".rlib")})
        assert len(libraries) == 1, libraries
        assert "fixture.o" not in run(["ar", "t", str(libraries[0])], cwd=root)
        repeat = run(command, cwd=root, env=env)
        assert "--crate-name farcooler_daemon " not in repeat, repeat
        print("ok: one optimized library build; only CLI/daemon link the archive; repeat is fresh")

        archive(42)
        run(command, cwd=root, env=env)
        for binary in ("farcooler", "farcoolerd"):
            assert run([str(products / binary)], cwd=root).strip() == "42"
        print("ok: changed archive relinks both binaries")

        plain = dict(os.environ)
        plain.pop("FARCOOLER_BUNDLE_TAILCAT_DIR", None)
        run(command[:command.index("--features")], cwd=root, env=plain)
        for binary in ("farcooler", "farcoolerd"):
            assert run([str(products / binary)], cwd=root).strip() == "0"
        print("ok: plain builds retain their original linkage without the bundle environment")

        for args, overrides, message in (
            (command, {"FARCOOLER_BUNDLE_TAILCAT_DIR": str(root / "missing")}, "bundle tunnel archive is missing"),
            (command[:command.index("--features")], {}, "bundle requires tailcat"),
        ):
            failed = subprocess.run(args, cwd=root, env=dict(env, **overrides),
                                    text=True, capture_output=True)
            assert failed.returncode != 0 and message in failed.stderr, failed.stderr
        print("ok: missing archives and disabled tunnel features fail the bundle build")


if __name__ == "__main__":
    main()
