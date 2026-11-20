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
    as resolve nodes (absent under `cargo metadata --no-deps`, which needs no
    registry crates) so inactive platform/feature edges cannot disappear.
    """
    members = set(metadata["workspace_members"])
    packages = {p["id"]: p for p in metadata["packages"] if p["id"] in members}
    by_name = {p["name"]: p["id"] for p in packages.values()}
    reverse = {p: set() for p in packages}
    for node in (metadata.get("resolve") or {}).get("nodes", []):
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


# Jobs of ci.yml that never run the Rust suite: an edit confined to them selects nothing (ov-1293).
# Anything else in the file (a Rust job, a shared job, the triggers, env, an unknown job) is everything.
NON_RUST_JOBS = {"swift", "swift-tests", "ios", "ios-ui-plan", "ios-ui", "android-build", "android-ui",
                 "android", "relay", "site", "web", "web-tests", "web-bundles"}
CI_WORKFLOW = ".github/workflows/ci.yml"


def workflow_jobs(text):
    """Map each line index of a workflow to the job it sits in ("" outside `jobs:`)."""
    owner, inside, out = "", False, []
    for line in text.split("\n"):
        if re.match(r"\S", line):
            inside = line.startswith("jobs:")
            owner = ""
        elif inside and (m := re.match(r"  ([A-Za-z0-9_-]+):\s*$", line)):
            owner = m[1]
        out.append(owner)
    return out


def workflow_touched(old, new):
    """The set of jobs a workflow edit changed; "" stands for the top level (triggers, env, concurrency)."""
    import difflib
    old_lines, new_lines = old.split("\n"), new.split("\n")
    old_jobs, new_jobs = workflow_jobs(old), workflow_jobs(new)
    touched = set()
    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, old_lines, new_lines, autojunk=False).get_opcodes():
        if tag == "equal":
            continue
        for jobs, lo, hi in ((old_jobs, i1, i2), (new_jobs, j1, j2)):
            # An empty range (a pure insertion or deletion) is judged by the line it sits beside.
            touched.update(jobs[lo:hi] or jobs[min(lo, len(jobs) - 1):][:1])
    return touched


def is_docs(path):
    """Words for people, never compiled: reports, docs, and Markdown anywhere (unless a crate includes it)."""
    return path.startswith(("reports/", "docs/")) or path.endswith(".md")


def includes(repo, roots):
    """Resolved included path -> the crates whose sources include it (include!, include_str!, include_bytes!, #[path])."""
    found = {}
    for consumer, root in roots.items():
        for source in (repo / root).rglob("*.rs"):
            for match in re.finditer(r'(?:include(?:_str|_bytes)?!\s*\(\s*|#\[path\s*=\s*)"([^"]+)"', source.read_text()):
                found.setdefault((source.parent / match[1]).resolve(), set()).add(consumer)
    return found


def select(repo, metadata, changes, workflow_texts=None):
    """Choose the packages to test; returns (names, reasons, full).

    Every change is judged and EVERY reason it goes full is listed, not the first. A docs path selects
    nothing unless a crate includes it. workflow_texts(path) -> (old, new) lets an edit to ci.yml be
    judged by the jobs it touches; without it, ci.yml is everything.
    """
    packages, reverse = graph(metadata)
    all_names = sorted(p["name"] for p in packages.values())
    roots = {p: Path(v["manifest_path"]).parent.relative_to(repo).as_posix() + "/"
             for p, v in packages.items()}
    changes = list(dict.fromkeys(changes))
    if not changes:
        return all_names, ["full: empty change range"], True
    seeds, ignored, full_reasons = set(), [], []
    consumers = None
    for status, path in changes:
        if path == CI_WORKFLOW and status == "M" and workflow_texts:
            touched = workflow_touched(*workflow_texts(path))
            rusty = sorted(touched - NON_RUST_JOBS)
            if rusty:
                full_reasons.append(f"full: {status} {path} (changes {', '.join(j or 'the workflow header' for j in rusty)})")
            else:
                ignored.append(f"{path} (only {', '.join(sorted(touched)) or 'whitespace'})")
            continue
        owner = next((p for p, root in roots.items() if path.startswith(root)), None)
        crate_source = (owner is not None and re.fullmatch(re.escape(roots[owner]) + r"(?:src|tests|examples)/.+\.rs", path)
                        and packages[owner]["name"] != "farcooler-protocol"
                        and not any("custom-build" in target["kind"]
                                    and Path(target["src_path"]).resolve() == (repo / path).resolve()
                                    for target in packages[owner].get("targets", [])))
        if is_docs(path) and not (owner and path.endswith(".rs")):
            if consumers is None:
                consumers = includes(repo, roots)
            readers = consumers.get((repo / path).resolve(), set())
            if readers:
                seeds.update(readers)
            else:
                ignored.append(path)
            continue
        # Manifest, build script, generated wire, include, fixture, config,
        # unknown, deletion and rename are deliberately full during shadowing.
        if status not in ("A", "M") or not crate_source:
            full_reasons.append(f"full: {status} {path}")
            continue
        seeds.add(owner)
        # Literal include!/include_str!/include_bytes! and #[path] inputs can
        # cross crate boundaries, even for an ordinary .rs file.
        if consumers is None:
            consumers = includes(repo, roots)
        seeds.update(consumers.get((repo / path).resolve(), set()))
    if full_reasons:
        return all_names, full_reasons, True
    notes = [f"docs and non-Rust files select no tests: {', '.join(ignored)}"] if ignored else []
    if not seeds:
        return [], notes or ["no Rust input changed"], False
    reached = set(seeds)
    pending = list(seeds)
    while pending:
        for consumer in reverse[pending.pop()] - reached:
            reached.add(consumer)
            pending.append(consumer)
    notes.append("reverse closure includes dev/build/optional/all target edges and CLI -> daemon")
    return sorted(packages[p]["name"] for p in reached), notes, reached == set(packages)


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
