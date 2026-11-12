#!/usr/bin/env python3
"""ov-1116: shadow Rust selection. Never supplies a gate's skip decision.

Run under scripts/heavy locally. CI supplies green main SHAs in CANDIDATES
(same NDJSON as ci-changes.py). With missing evidence the proposal is full.
The command after -- ALWAYS runs unchanged; a successful narrow proposal is
then replayed, with only a nextest package filter added, to measure warm wall
time rather than calling summed test durations a saving.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

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


def plan(repo, candidates):
    head = capture(repo, "git", "rev-parse", "HEAD").strip()
    base = green_base(repo, head, candidates)
    if not base:
        raise ValueError("no verified green ancestor (previous commit is not evidence)")
    raw = capture(repo, "git", "diff", "--no-renames", "--name-status", "-z", f"{base}..{head}").split("\0")
    changes = list(zip(raw[0:-1:2], raw[1:-1:2]))
    if capture(repo, "git", "status", "--porcelain", "--untracked-files=no").strip():
        raise ValueError("dirty tracked inputs")
    # No --filter-platform: foreign target dependencies must remain visible.
    metadata = json.loads(capture(repo, "cargo", "metadata", "--locked", "--format-version", "1", "--all-features"))
    selected, reasons, full = select(repo, metadata, changes)
    return {"base": base, "head": head, "changes": changes, "packages": selected,
            "full": full, "reasons": reasons,
            "metadata_policy": "all features, unfiltered targets; all dependency kinds",
            "external_consumers": ["Mac full suite (including real CLI tests)",
                                   "feature/target checks, web, Android and nightly unchanged"],
            "skips_enabled": False}


def accepted(full_rc, selected_rc, inventory_ok):
    # Even when the proposed selection passes, a planted failure in an omitted
    # consumer rejects it. Full failures are never overwritten by a replay.
    return full_rc == 0 and selected_rc == 0 and inventory_ok


def measure(command, repo):
    load = os.getloadavg()
    peak = max(load[:2])
    start = time.monotonic()
    process = subprocess.Popen(command, cwd=repo)
    while True:
        try:
            rc = process.wait(timeout=1)
            break
        except subprocess.TimeoutExpired:
            peak = max(peak, *os.getloadavg()[:2])
    peak = max(peak, *os.getloadavg()[:2])
    return {"exit": rc, "wall_seconds": time.monotonic() - start,
            "load_start": load, "load_end": os.getloadavg(), "peak_load_1m_5m": peak}


def run(args):
    repo = Path.cwd().resolve()
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if command[-7:] != ["cargo", "nextest", "run", "--profile", "ci", "--workspace", "--no-fail-fast"]:
        raise ValueError("expected full workspace nextest command, ending --no-fail-fast")
    try:
        candidates = [json.loads(line)["head_sha"] for line in Path(os.environ["CANDIDATES"]).read_text().splitlines() if line.strip()]
        proposal = plan(repo, candidates)
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as exc:
        proposal = {"full": True, "packages": [], "reasons": [f"full: {exc}"], "skips_enabled": False}
    (out / "plan.json").write_text(json.dumps(proposal, indent=2) + "\n")
    filters = ["-E", " or ".join(f"package(={p})" for p in proposal["packages"])] if not proposal["full"] else []
    inventory_ok = True
    for name, extra in (("full", []), ("selected", filters)):
        try:
            inventory = capture(repo, "cargo", "nextest", "list", "--profile", "ci", "--workspace", "--message-format", "json", *extra)
            json.loads(inventory)
            (out / f"{name}-inventory.json").write_text(inventory)
        except (OSError, ValueError, subprocess.CalledProcessError) as exc:
            inventory_ok = False
            (out / f"{name}-inventory-error.txt").write_text(str(exc))
    # Both measurement runs require load <20; never stop another task's work.
    import heavy_lock
    heavy_lock.wait_for_load(20, load=lambda: max(os.getloadavg()[:2]))
    full = measure(command, repo)
    selected = None
    if not proposal["full"] and full["exit"] == 0 and inventory_ok:
        heavy_lock.wait_for_load(20, load=lambda: max(os.getloadavg()[:2]))
        selected = measure([*command, *filters], repo)
    valid = accepted(full["exit"], selected["exit"] if selected else full["exit"], inventory_ok)
    under_load_limit = all(m["peak_load_1m_5m"] < 20 for m in (full, selected) if m)
    report = {"full": full, "selected": selected, "selection_accepted": valid,
              "measurement_under_load_20": under_load_limit,
              "skips_enabled": False, "scope": "default host nextest only; full other suites and doctests retained",
              "warm_wall_savings_seconds": full["wall_seconds"] - selected["wall_seconds"] if selected and valid and under_load_limit else None}
    (out / "measurement.json").write_text(json.dumps(report, indent=2) + "\n")
    savings = report["warm_wall_savings_seconds"]
    savings_text = f"{savings:.2f}s" if savings is not None else "not measured/valid"
    summary = (f"Rust shadow: {'accepted' if valid else 'REJECTED'}; skips disabled.\n"
               f"Reasons: {'; '.join(proposal['reasons'])}\n"
               f"Full wall: {full['wall_seconds']:.2f}s; "
               f"selected wall: {selected['wall_seconds'] if selected else 'not replayed (full fallback or rejected)'}; "
               f"warm wall savings: {savings_text}.\n"
               "Warm sequential replay is not an end-to-end gate speedup. See inventories and load in artifacts.\n")
    print(summary)
    (out / "summary.txt").write_text(summary)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
            stream.write(summary)
    return full["exit"] or (selected["exit"] if selected else 0) or (0 if inventory_ok else 1)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    sys.exit(run(parser.parse_args()))
