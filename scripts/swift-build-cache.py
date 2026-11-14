#!/usr/bin/env python3
"""Reuse Swift compilation across fresh CI checkouts without caching final links.

Swift's incremental build records include input timestamps. A fresh checkout
otherwise makes every unchanged source newer than its cached objects. Restore
only timestamps whose SHA-256 still matches; changed inputs remain fresh. The
cache is scoped to the complete Xcode/Swift/SDK version by ci.yml (ov-1183).
Rust libraries are always built separately, and every executable is relinked.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
PACKAGES = ("apps/macos", "apps/shared/AgentKit")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inputs(root):
    listed = subprocess.check_output([
        "git", "ls-files", "-z", "--", *PACKAGES,
        "crates/vt/include", "crates/client/include",
    ], cwd=root)
    return [os.fsdecode(name) for name in listed.split(b"\0") if name]


def save(root, manifest, names):
    records = {}
    for name in names:
        path = root / name
        if path.is_file() and not path.is_symlink():
            records[name] = {"sha256": digest(path), "mtime_ns": path.stat().st_mtime_ns}
    manifest.parent.mkdir(parents=True, exist_ok=True)
    manifest.write_text(json.dumps(records, sort_keys=True) + "\n")
    print(f"swift-build-cache: recorded {len(records)} input timestamps")


def restore(root, manifest, names):
    if not manifest.exists():
        print("swift-build-cache: cold build; no input timestamp record")
        return
    records = json.loads(manifest.read_text())
    restored = changed = 0
    # Iterate this checkout's tracked inputs, never paths supplied by the cache.
    for name in names:
        path = root / name
        record = records.get(name)
        if not record or not path.is_file() or path.is_symlink():
            continue
        stat = path.stat()
        if digest(path) == record["sha256"]:
            os.utime(path, ns=(stat.st_atime_ns, record["mtime_ns"]))
            restored += 1
        else:
            # Even a checkout that preserved an old timestamp must rebuild a
            # changed input. It cannot inherit the cached input's identity.
            os.utime(path, ns=(stat.st_atime_ns, max(time.time_ns(), record["mtime_ns"] + 1)))
            changed += 1
    linked = 0
    for package in PACKAGES:
        build = root / package / ".build"
        # SwiftPM's actual product paths: <triple>/<configuration>/<product>.
        # Keep objects/modules, discard executable products so the current
        # Rust .a files and this commit's build stamp always reach the tests.
        products = list(build.glob("*/*/*.xctest/Contents/MacOS/*"))
        products += list(build.glob("*/*/Far Cooler"))
        # Swift 6.4's Xcode backend uses out/Products/Debug, reached through
        # these standard aliases; the llbuild backend uses <triple>/debug.
        for config in ("debug", "release"):
            directory = build / config
            products += list(directory.glob("*.xctest/Contents/MacOS/*"))
            products.append(directory / "Far Cooler")
        actual = set()
        for product in products:
            if product.is_file():
                resolved = product.resolve()
                if not resolved.is_relative_to(build.resolve()):
                    raise ValueError(f"cached product escapes its build directory: {product}")
                actual.add(resolved)
        for product in actual:
            product.unlink()
            linked += 1
    print(f"swift-build-cache: restored {restored} unchanged inputs; {changed} changed; {linked} products will relink")


def self_test():
    with tempfile.TemporaryDirectory(prefix="ov1183-swift-cache-") as scratch:
        root = Path(scratch)
        names = ["apps/macos/Sources/Unchanged.swift", "apps/macos/Sources/Changed.swift"]
        for name in names:
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("original")
            os.utime(path, ns=(1_000_000_000, 1_000_000_000))
        manifest = root / "target/cache.json"
        save(root, manifest, names)
        for name in names:
            os.utime(root / name, ns=(2_000_000_000, 2_000_000_000))
        (root / names[1]).write_text("changed")
        executable = root / "apps/macos/.build/arm64-apple-macosx/debug/CeremonyTests.xctest/Contents/MacOS/CeremonyTests"
        executable.parent.mkdir(parents=True)
        executable.touch()
        app = executable.parents[3] / "Far Cooler"
        app.touch()
        obj = executable.parents[3] / "Far Cooler.build/source.o"
        obj.parent.mkdir()
        obj.write_text("object")
        modern = root / "apps/shared/AgentKit/.build/out/Products/Debug"
        modern.mkdir(parents=True)
        (modern / "Far Cooler").touch()
        (modern.parents[2] / "debug").symlink_to("out/Products/Debug", target_is_directory=True)
        restore(root, manifest, names)
        assert not (modern / "Far Cooler").exists()
        assert (root / names[0]).stat().st_mtime_ns == 1_000_000_000
        assert (root / names[1]).read_text() == "changed"
        assert (root / names[1]).stat().st_mtime_ns > 1_000_000_000
        assert not executable.exists() and not app.exists()
        assert obj.read_text() == "object"
        restore(root, root / "missing.json", names)
    print("ok: unchanged inputs reuse timestamps, changed sources rebuild, current libraries must relink")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("mode", choices=("save", "restore", "self-test"))
    args = ap.parse_args()
    if args.mode == "self-test":
        self_test()
    else:
        manifest = ROOT / "target/swift-build-cache.json"
        (save if args.mode == "save" else restore)(ROOT, manifest, inputs(ROOT))


if __name__ == "__main__":
    main()
