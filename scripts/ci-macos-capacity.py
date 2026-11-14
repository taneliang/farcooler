#!/usr/bin/env python3
"""Check the repository's four landing / one background Mac reservation (ov-1190).

Uses the same small workflow reader as the wire lint; no third-party modules.
Enumerates dependency antichains, including matrix widths, rather than assuming
all named jobs overlap. This is a capacity proof, not a hosted queue benchmark.
"""
import importlib.util
import itertools
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("proto_lint", ROOT / "scripts/proto-lint.py")
proto = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proto)
GROUP = "group: background-macos\ncancel-in-progress: false\nqueue: max"


def peak(jobs, weights):
    """Largest weighted set whose jobs do not depend on each other."""
    ancestors = {}
    def visit(job, stack=()):
        if job in stack:
            raise ValueError(f"dependency cycle at {job}")
        if job not in ancestors:
            direct = proto.needs_of(jobs[job].get("needs"))
            ancestors[job] = direct | set().union(*(visit(n, (*stack, job)) for n in direct))
        return ancestors[job]
    for job in jobs:
        visit(job)
    names = [n for n in jobs if weights[n]]
    return max((sum(weights[n] for n in subset)
                for size in range(len(names) + 1)
                for subset in itertools.combinations(names, size)
                if all(a not in ancestors[b] and b not in ancestors[a]
                       for a, b in itertools.combinations(subset, 2))), default=0)


def widths(keys):
    """Runnable total/Mac matrix width for the checked-in workflow shapes."""
    if "uses" in keys:
        if keys["uses"] != "./.github/workflows/linux-binaries.yml":
            raise ValueError("unmodelled reusable workflow: " + keys["uses"])
        return 2, 0  # checked against the reusable's matrix below
    strategy = keys.get("strategy", "")
    axes = re.findall(r"^(?:os|shard): \[([^\]]+)\]$", strategy, re.M)
    if "matrix:" in strategy and len(axes) != 1:
        raise ValueError("unmodelled matrix: " + strategy)
    labels = axes[0].split(",") if axes else [keys.get("runs-on", "")]
    if "max-parallel:" in strategy:
        limit = int(re.search(r"max-parallel: (\d+)", strategy)[1])
    else:
        limit = len(labels)
    runner = keys.get("runs-on", "")
    macs = sum(bool(re.search(r"macos|xcode", label)) for label in labels) if "matrix.os" in runner else (len(labels) if re.search(r"macos|xcode", runner) else 0)
    if not runner or ("${{" in runner and "matrix.os" not in runner):
        raise ValueError("unmodelled runner: " + runner)
    return min(len(labels), limit), min(macs, limit)


def check(texts):
    problems, counts = [], {}
    nightly_group = proto.top_level(texts["nightly.yml"]).get("concurrency", "").strip()
    if nightly_group != "group: nightly\ncancel-in-progress: false\nqueue: max":
        problems.append("Nightly must queue whole runs separately from its per-job Mac lock")
    # Keep the reusable expansion honest if its architectures/runner change.
    linux = proto.workflow_jobs(texts["linux-binaries.yml"])
    if (set(linux) != {"build"} or linux["build"].get("runs-on") != "ubuntu-latest"
            or len(re.findall(r"^- arch:", linux["build"].get("strategy", ""), re.M)) != 2):
        problems.append("update the Linux reusable capacity model")
    for name, text in texts.items():
        if name == "linux-binaries.yml":
            continue
        jobs = proto.workflow_jobs(text)
        total, mac = {}, {}
        workflow_lock = proto.top_level(text).get("concurrency", "").strip() == GROUP
        for job, keys in jobs.items():
            total[job], mac[job] = widths(keys)
            lock = keys.get("concurrency", "").strip() == GROUP
            if name == "ci.yml":
                if lock or workflow_lock:
                    problems.append("landing CI must not wait on background-macos")
            elif mac[job] and not (workflow_lock or lock):
                problems.append(f"{name}/{job} escapes background-macos")
        mac_peak = peak(jobs, mac)
        if name != "ci.yml" and mac_peak > 1:
            problems.append(f"{name} can use {mac_peak} Macs; only one background slot is available")
        counts[name] = peak(jobs, total), mac_peak
    if counts["ci.yml"][1] > 4:
        problems.append("CI exceeds its four reserved Macs")
    # Nightly runs Linux checks while a Canary/Release/tunnel workflow owns the
    # background group. Their Macs cannot overlap, but count their Linux work
    # conservatively as if it all could. Only one whole shipping workflow runs.
    landing = counts["ci.yml"][0]
    nightly_linux = sum(widths(k)[0] for k in proto.workflow_jobs(texts["nightly.yml"]).values() if widths(k)[1] == 0)
    shipping = max(counts[n][0] for n in ("canary.yml", "release.yml", "tunnel-e2e.yml"))
    if landing + nightly_linux + shipping > 20:
        problems.append("landing + Nightly Linux + shipping exceeds 20 total jobs")
    for name in ("canary.yml", "release.yml", "tunnel-e2e.yml"):
        if proto.top_level(texts[name]).get("concurrency", "").strip() != GROUP:
            problems.append(f"{name} must hold background-macos for the whole workflow")
    return problems, counts, landing + nightly_linux + shipping


def main():
    paths = ROOT / ".github/workflows"
    texts = {n: (paths / n).read_text() for n in ("ci.yml", "nightly.yml", "canary.yml", "release.yml", "tunnel-e2e.yml", "linux-binaries.yml")}
    problems, counts, total = check(texts)
    for path in paths.glob("*.yml"):
        if path.name not in texts and re.search(r"^    runs-on:.*(?:macos|xcode)", path.read_text(), re.M):
            problems.append(f"{path.name}: add this Apple workflow to the reservation model")
    if "--self-test" in sys.argv:
        cases = [
            ("Canary overlaps Nightly", "canary.yml", "  group: background-macos", "  group: canary"),
            ("Canary's Macs overlap", "canary.yml", "needs: [linux, plan, quiet, ios, wire]", "needs: [linux, plan, quiet, wire]"),
            ("Release's Macs overlap", "release.yml", "    needs: [macos]\n", ""),
            ("Nightly pending run replacement", "nightly.yml", "  queue: max\n", ""),
            ("Nightly pending shard replacement", "nightly.yml", "      queue: max\n", ""),
            ("Nightly cancels shipping", "nightly.yml", "      cancel-in-progress: false", "      cancel-in-progress: true"),
            ("Tunnel escapes", "tunnel-e2e.yml", "  group: background-macos", "  group: tunnel"),
            ("a second Nightly Mac", "nightly.yml", "    runs-on: ubuntu-latest", "    runs-on: xcode-27"),
        ]
        for label, name, old, new in cases:
            changed = dict(texts)
            changed[name] = changed[name].replace(old, new)
            if changed[name] == texts[name] or not check(changed)[0]:
                problems.append("self-test did not reject " + label)
        # Exercise graph width independently of current hardcoded job counts.
        graph = {"a": {}, "b": {}, "c": {"needs": "[a, b]"}}
        if peak(graph, {"a": 2, "b": 1, "c": 4}) != 4:
            problems.append("weighted dependency antichain self-test failed")
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 1
    print(f"Mac capacity: landing {counts['ci.yml'][1]} + shared background 1 = 5; total-job upper bound {total}/20")
    print("Workflow peaks (total, macOS): " + str(counts))
    return 0


if __name__ == "__main__":
    sys.exit(main())
