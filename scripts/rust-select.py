#!/usr/bin/env python3
"""Rust affected-test selection (ov-1116, on by default in the gates since ov-1278).

The Cargo dependency graph (every kind of edge, every feature, every target) says which workspace
crates a change can reach; select() proposes exactly those crates' tests. Anything it cannot map
is everything: Cargo.toml and Cargo.lock, a build script, the toolchain file, a fixture, a
deleted or renamed file, protocol, an unknown path. test_binaries() names the test binaries each
choice runs and skips, for the gate's evidence. CI on main runs the whole suite on every push.
"""
import importlib.util
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def capture(repo, *cmd):
    return subprocess.run(cmd, cwd=repo, text=True, capture_output=True, check=True).stdout


def green_base(repo, head, candidates):
    spec = importlib.util.spec_from_file_location("canary_plan", ROOT / "scripts/canary-plan.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.base_commit(repo, head, candidates)


def graph(metadata):
    """Union all edges: normal, dev, build, optional and every target predicate.

    Do not filter metadata by host. Dependency declarations are used as well
    as resolve nodes so inactive platform/feature edges cannot disappear.
    """
    members = set(metadata["workspace_members"])
    packages = {p["id"]: p for p in metadata["packages"] if p["id"] in members}
    by_name = {p["name"]: p["id"] for p in packages.values()}
    reverse = {p: set() for p in packages}
    for node in metadata["resolve"]["nodes"]:
        if node["id"] in members:
            for dep in node["deps"]:
                if dep["pkg"] in members:
                    reverse[dep["pkg"]].add(node["id"])
    for p in packages.values():
        for dep in p["dependencies"]:
            if dep.get("path") and dep["name"] in by_name:
                reverse[by_name[dep["name"]]].add(p["id"])
    # The daemon spawns the CLI, which Cargo cannot express without a cycle.
    if "farcooler-cli" in by_name and "farcooler-daemon" in by_name:
        reverse[by_name["farcooler-cli"]].add(by_name["farcooler-daemon"])
    return packages, reverse


def select(repo, metadata, changes):
    packages, reverse = graph(metadata)
    all_names = sorted(p["name"] for p in packages.values())
    roots = {p: Path(v["manifest_path"]).parent.relative_to(repo).as_posix() + "/"
             for p, v in packages.items()}
    seeds = set()
    reasons = []
    for status, path in changes:
        owner = next((p for p, root in roots.items() if path.startswith(root)), None)
        # Manifest, build script, generated wire, include, fixture, config,
        # unknown, deletion and rename are deliberately full during shadowing.
        if (status not in ("A", "M") or owner is None
                or not re.fullmatch(re.escape(roots[owner]) + r"(?:src|tests|examples)/.+\.rs", path)
                or packages[owner]["name"] == "farcooler-protocol"
                or any("custom-build" in target["kind"]
                       and Path(target["src_path"]).resolve() == (repo / path).resolve()
                       for target in packages[owner].get("targets", []))):
            return all_names, [f"full: {status} {path}"], True
        seeds.add(owner)
        # Literal include!/include_str!/include_bytes! and #[path] inputs can
        # cross crate boundaries, even for an ordinary .rs file.
        for consumer, root in roots.items():
            for source in (repo / root).rglob("*.rs"):
                for match in re.finditer(r'(?:include(?:_str|_bytes)?!\s*\(\s*|#\[path\s*=\s*)"([^"]+)"', source.read_text()):
                    if (source.parent / match[1]).resolve() == (repo / path).resolve():
                        seeds.add(consumer)
    if not seeds:
        return all_names, ["full: empty change range"], True
    reached = set(seeds)
    pending = list(seeds)
    while pending:
        for consumer in reverse[pending.pop()] - reached:
            reached.add(consumer)
            pending.append(consumer)
    reasons.append("reverse closure includes dev/build/optional/all target edges and CLI -> daemon")
    return sorted(packages[p]["name"] for p in reached), reasons, reached == set(packages)


def test_binaries(metadata, selected):
    """Split the workspace's test binaries into (run, skipped) by selected package name.

    Ids follow nextest's: `pkg` for a library's unit tests, `pkg::bin/name`, `pkg::name` for an
    integration test. Read from metadata, so naming them builds nothing.
    """
    members = set(metadata["workspace_members"])
    run, skipped = [], []
    for package in metadata["packages"]:
        if package["id"] not in members:
            continue
        for target in package["targets"]:
            kinds = set(target["kind"])
            if not target.get("test", False) or "custom-build" in kinds:
                continue
            if kinds & {"lib", "rlib", "cdylib", "dylib", "staticlib", "proc-macro"}:
                binary = package["name"]
            elif "bin" in kinds:
                binary = f"{package['name']}::bin/{target['name']}"
            elif "test" in kinds:
                binary = f"{package['name']}::{target['name']}"
            else:
                continue
            (run if package["name"] in selected else skipped).append(binary)
    return sorted(run), sorted(skipped)
