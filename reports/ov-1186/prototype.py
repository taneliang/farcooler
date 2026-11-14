#!/usr/bin/env python3
"""Branch-only nextest archive experiment, never a required-check receipt.

The source fingerprint deliberately excludes this report and its experiment
workflow. Production must use ov-1117 provenance, not this research helper.
GitHub Git blobs are an immutable, disposable transport here, not a cache service.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
HERE = ROOT / "reports/ov-1186"
OUT = ROOT / "target/ov-1186"
PACKAGE = "farcooler-agent-core"
TARGET = "aarch64-apple-darwin"
NEXTEST = "0.9.148"


def output(*cmd):
    return subprocess.check_output(cmd, cwd=ROOT, text=True).strip()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def excluded(name):
    return name.startswith("reports/ov-1186/") or name == ".github/workflows/ov1186-cache.yml"


def source_key():
    # Hash working bytes, not index blobs: a dirty source must miss too.
    names = output("git", "ls-files", "-z").split("\0")
    unknown = output("git", "ls-files", "--others", "--exclude-standard", "-z")
    if any(n and not excluded(n) for n in unknown.split("\0")):
        raise ValueError("untracked input outside the experiment")
    entries = []
    for name in sorted(n for n in names if n and not excluded(n)):
        p = ROOT / name
        if p.is_symlink():
            value = ["symlink", os.readlink(p)]
        else:
            value = [p.stat().st_mode & 0o111, sha(p.read_bytes())]
        entries.append([name, value])
    return sha(json.dumps(entries, separators=(",", ":")).encode())


def timed(name, command, env=None):
    start = time.monotonic()
    log = OUT / (name + ".log")
    with log.open("w") as f:
        result = subprocess.run(command, cwd=ROOT, env=env, stdout=f, stderr=subprocess.STDOUT)
    seconds = time.monotonic() - start
    print(log.read_text(), end="", flush=True)
    if result.returncode:
        raise RuntimeError(f"{name} failed: {result.returncode}")
    return {"seconds": round(seconds, 3), "command": command, "log": log.name}


def validate(manifest, archive, system=None, machine=None, source=None):
    if manifest["schema"] != 1 or manifest["package"] != PACKAGE:
        raise ValueError("unsupported archive scope")
    if manifest["target"] != TARGET or (system or platform.system()) != "Darwin":
        raise ValueError("Mac artifacts cannot prove Linux")
    if (machine or platform.machine()) != "arm64":
        raise ValueError("architecture mismatch")
    if manifest["source_key"] != (source or source_key()):
        raise ValueError("source mismatch")
    if manifest["archive_sha256"] != sha(archive.read_bytes()):
        raise ValueError("archive corruption")
    if manifest["nextest"] != NEXTEST:
        raise ValueError("nextest mismatch")


def seed():
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        raise ValueError("seed this experiment on the owner's arm64 Mac")
    archive = OUT / "agent-core.tar.zst"
    build = OUT / "measured-build"
    if build.exists():
        raise ValueError("use a fresh measured-build directory; never call a warm build cold")
    before = source_key()
    env = {**os.environ, "MACOSX_DEPLOYMENT_TARGET": "15.0", "CARGO_INCREMENTAL": "0",
           "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_BUILD_JOBS": "3"}
    command = ["cargo", "nextest", "archive", "--locked", "--offline", "-p", PACKAGE,
               "--target", TARGET, "--target-dir", str(build), "--archive-file", str(archive)]
    timings = {"cold_archive": timed("cold-archive", command, env)}
    timings["warm_archive"] = timed("warm-archive", command, env)
    timings["local_tests"] = timed("local-tests", ["cargo", "nextest", "run", "--profile", "ci",
        "--archive-file", str(archive), "--workspace-remap", str(ROOT)])
    if before != source_key():
        raise ValueError("source changed during build")
    manifest = {"schema": 1, "package": PACKAGE, "target": TARGET, "nextest": NEXTEST,
                "source_key": before, "archive_sha256": sha(archive.read_bytes()),
                "archive_bytes": archive.stat().st_size, "deployment_target": "15.0",
                "producer_rustc": output("rustc", "-vV"),
                "producer_os": output("sw_vers"), "load_after": os.getloadavg(),
                "timings": timings}
    (HERE / "seed.json").write_text(json.dumps(manifest, indent=2) + "\n")


def upload():
    manifest = json.loads((HERE / "seed.json").read_text())
    archive = OUT / "agent-core.tar.zst"
    validate(manifest, archive)
    payload = OUT / "blob-upload.json"
    payload.write_text(json.dumps({"encoding": "base64",
                                  "content": base64.b64encode(archive.read_bytes()).decode()}))
    start = time.monotonic()
    blob = json.loads(output("gh", "api", "repos/taneliang/farcooler/git/blobs",
                             "--input", str(payload)))
    manifest["blob"] = blob["sha"]
    manifest["upload_seconds"] = round(time.monotonic() - start, 3)
    (HERE / "seed.json").write_text(json.dumps(manifest, indent=2) + "\n")


def consume():
    manifest = json.loads((HERE / "seed.json").read_text())
    start = time.monotonic()
    data = json.loads(output("gh", "api", "repos/taneliang/farcooler/git/blobs/" + manifest["blob"]))
    archive = OUT / "download.tar.zst"
    archive.write_bytes(base64.b64decode(data["content"]))
    validate(manifest, archive)
    result = {"download_verify_seconds": round(time.monotonic() - start, 3),
              "source_key": manifest["source_key"], "archive_sha256": manifest["archive_sha256"],
              "runner_os": platform.platform(), "compilation_skipped": True}
    # This runs the tests again on CI; no Rust compilation command is invoked.
    result["tests"] = timed("ci-archive-tests", ["cargo", "nextest", "run", "--profile", "ci",
        "--archive-file", str(archive), "--workspace-remap", str(ROOT)])
    result["total_seconds"] = round(time.monotonic() - start, 3)
    (OUT / "consume.json").write_text(json.dumps(result, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["seed", "upload", "consume", "key"])
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    if args.mode == "key":
        print(source_key())
    else:
        globals()[args.mode]()


if __name__ == "__main__":
    main()
