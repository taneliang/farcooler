#!/usr/bin/env python3
"""Check the repository's four landing / one background Mac reservation (ov-1190).

Uses the same small workflow reader as the wire lint; no third-party modules.
All repository triggers are conservatively allowed to overlap one landing;
path filters and deployment variables never reduce the reservation.
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
    # Maximum-weight independent set of the comparability graph. Branch on
    # each job and discard its relatives; this avoids enumerating all subsets.
    conflicts = {n: {m for m in names if m != n and
                    (m in ancestors[n] or n in ancestors[m])} for n in names}
    def search(remaining):
        if not remaining:
            return 0
        n = max(remaining, key=lambda item: len(conflicts[item] & remaining))
        rest = remaining - {n}
        return max(search(rest), weights[n] + search(rest - conflicts[n]))
    return search(set(names))



def widths(keys, texts):
    """Runnable total/Mac matrix width for the checked-in workflow shapes."""
    if "uses" in keys:
        name = keys["uses"].removeprefix("./.github/workflows/")
        if keys["uses"] != "./.github/workflows/" + name or name not in texts:
            raise ValueError("unmodelled reusable workflow: " + keys["uses"])
        jobs = proto.workflow_jobs(texts[name])
        expanded = {job: widths(value, texts) for job, value in jobs.items()}
        return tuple(peak(jobs, {job: value[i] for job, value in expanded.items()})
                     for i in (0, 1))
    strategy = keys.get("strategy", "").strip()
    if strategy == "fail-fast: false\nmatrix:\nchannel: ${{ fromJSON(needs.plan.outputs.channels) }}":
        # Site's plan emits two channels on push, one on dispatch/call. Refuse
        # changes to that producer until this upper bound is reviewed.
        if "channels='[\"canary\",\"production\"]'" not in texts["site.yml"]:
            raise ValueError("update the Site dynamic matrix capacity model")
        if keys.get("runs-on") != "ubuntu-latest":
            raise ValueError("update the Site runner capacity model")
        return 2, 0
    if "include:" in strategy:
        if (keys.get("runs-on") != "ubuntu-latest"
                or not re.fullmatch(r"matrix:\ninclude:(?:\n- arch: [^\n]+\ntarget: [^\n]+\nmusl_sha256: [^\n]+)+", strategy)):
            raise ValueError("unmodelled include matrix: " + strategy)
        return len(re.findall(r"^- arch:", strategy, re.M)), 0
    # Exclusions can only reduce capacity. Validate this known targeted-shard
    # form, then count the full scheduled matrix as the conservative bound.
    if "\nexclude:" in strategy:
        strategy, excludes = strategy.split("\nexclude:", 1)
        axis = re.search(r"^shard: \[([^\]]+)\]$", strategy, re.M)
        if not axis:
            raise ValueError("unmodelled targeted matrix")
        labels = [label.strip() for label in axis[1].split(",")]
        expected = [f"- shard: ${{{{ inputs.shard != '' && inputs.shard != 'all' && inputs.shard != '{label}' && '{label}' || '__none__' }}}}" for label in labels]
        if excludes.strip().splitlines() != expected:
            raise ValueError("unmodelled matrix exclusions: " + excludes)
    axes = re.findall(r"^(?:os|shard|part): \[([^\]]+)\]$", strategy, re.M)
    known_strategy = all(re.fullmatch(
        r"(?:fail-fast: (?:false|true)|max-parallel: [1-9]\d*|matrix:|(?:os|shard|part): \[[^\]]+\])", line)
        for line in strategy.splitlines())
    if not known_strategy or ("matrix:" in strategy and len(axes) != 1):
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
    problems, counts, entries = [], {}, []
    nightly_group = proto.top_level(texts["nightly.yml"]).get("concurrency", "").strip()
    if nightly_group != "group: nightly\ncancel-in-progress: false\nqueue: max":
        problems.append("Nightly must queue whole runs separately from its per-job lock")
    for name, text in texts.items():
        top = proto.top_level(text)
        trigger_block = re.search(r"^on:\n((?:[ \t].*\n|\n|#.*\n)*)", text, re.M)
        triggers = set(re.findall(r"^  ([a-z_]+):", trigger_block[1], re.M)) if trigger_block else set()
        if not triggers or triggers - {"push", "pull_request", "workflow_run", "schedule", "workflow_dispatch", "workflow_call"}:
            problems.append(f"{name}: unmodelled trigger")
        # A call-only workflow consumes runners inside its caller's lock, not
        # as an independent run. widths() expands its actual jobs recursively.
        if triggers == {"workflow_call"}:
            continue
        jobs = proto.workflow_jobs(text)
        expanded = {job: widths(keys, texts) for job, keys in jobs.items()}
        total = {job: value[0] for job, value in expanded.items()}
        mac = {job: value[1] for job, value in expanded.items()}
        workflow_lock = top.get("concurrency", "").strip() == GROUP
        counts[name] = peak(jobs, total), peak(jobs, mac)
        if workflow_lock:
            entries.append((name, *counts[name]))
            if counts[name][1] > 1:
                problems.append(f"{name} can use more than one background Mac")
        for job, keys in jobs.items():
            lock = keys.get("concurrency", "").strip() == GROUP
            called = keys.get("uses", "").removeprefix("./.github/workflows/")
            called_lock = proto.top_level(texts.get(called, "")).get("concurrency", "").strip() == GROUP
            child_groups = (keys.get("concurrency", "").splitlines()
                            + proto.top_level(texts.get(called, "")).get("concurrency", "").splitlines())
            if workflow_lock and "group: background-macos" in child_groups:
                problems.append(f"{name}/{job} would wait for its own workflow's lock")
            if name == "ci.yml":
                if lock or workflow_lock or called_lock:
                    problems.append("landing CI must not wait on background-macos")
            elif name == "canary-baseline.yml":
                # The recorder must never lose a shipped baseline to queue
                # overflow/replacement. Its separate single job is reserved.
                if workflow_lock or lock or called_lock or mac[job]:
                    problems.append("Canary baseline must remain outside the background queue on Linux")
            elif not workflow_lock:
                if name == "nightly.yml" and job.startswith("yield-"):
                    # Linux gate that waits for main CI; holds no Mac (ov-1226).
                    if keys.get("runs-on") != "ubuntu-latest" or total[job] != 1 or mac[job]:
                        problems.append(f"{job} must be one Linux job")
                    continue
                if lock:
                    entries.append((f"{name}/{job}", total[job], mac[job]))
                elif not called_lock:
                    problems.append(f"{name}/{job} escapes background-macos")
        if name == "nightly.yml":
            for job, keys in jobs.items():
                if mac[job] and not any(n.startswith("yield-") for n in proto.needs_of(keys.get("needs"))):
                    problems.append(f"Nightly {job} must wait on a yield-* gate for main CI")
        if name == "nightly.yml" and counts[name][1] > 1:
            problems.append("Nightly's Apple phases must remain sequential")
    landing = counts["ci.yml"]
    background = max((entry[1] for entry in entries), default=0)
    background_mac = max((entry[2] for entry in entries), default=0)
    recorder = counts["canary-baseline.yml"][0]
    # One Nightly yield gate can be waiting beside the background entry.
    total = landing[0] + background + recorder + 1
    if landing[1] + background_mac > 5:
        problems.append("landing plus shared background exceeds five Macs")
    if total > 20:
        problems.append("landing + all overlapping background workflows + recorder exceeds 20 total jobs")
    # Keep the recorder's independent reservation explicitly bounded.
    if recorder != 1:
        problems.append("update the Canary baseline reservation")
    return problems, counts, total


def main():
    paths = ROOT / ".github/workflows"
    texts = {path.name: path.read_text() for path in paths.glob("*.y*ml")}
    problems, counts, total = check(texts)
    if "--self-test" in sys.argv:
        cases = [
            ("Canary overlaps Nightly", "canary.yml", "  group: background-macos", "  group: canary"),
            ("Canary's Macs overlap", "canary.yml", "needs: [linux, plan, quiet, ios, wire]", "needs: [linux, plan, quiet, wire]"),
            ("Release's Macs overlap", "release.yml", "    needs: [macos]\n", ""),
            ("Nightly pending run replacement", "nightly.yml", "  queue: max\n", ""),
            ("Nightly pending shard replacement", "nightly.yml", "      queue: max\n", ""),
            ("Nightly cancels shipping", "nightly.yml", "      cancel-in-progress: false", "      cancel-in-progress: true"),
            ("Tunnel escapes", "tunnel-e2e.yml", "  group: background-macos", "  group: tunnel"),
            ("Nightly iOS UI ignores main CI", "nightly.yml", "needs: [ios, yield-ui]", "needs: [ios]"),
            ("Nightly perf ignores main CI", "nightly.yml", "needs: [captures, yield-perf]", "needs: [captures]"),
            ("a second Nightly Mac", "nightly.yml", "    runs-on: ubuntu-latest", "    runs-on: xcode-27"),
        ]
        cases += [
            ("research workflow escapes", "ov1186-cache.yml", "group: background-macos", "group: research"),
            ("Nightly Rust Mac ignores main CI", "nightly.yml", "needs: [yield-rust]", "needs: []"),
            ("Nightly Rust overlaps iOS", "nightly.yml", "needs: [rust-macos]", "needs: []"),
            ("platform checks overlap native archive", "ci.yml", "needs: [changes, rust-macos-build]", "needs: changes"),
            ("native shards overlap Swift build", "ci.yml", "needs: [changes, rust-macos-build, swift, swift-tests]", "needs: [changes, rust-macos-build, swift-tests]"),
            ("native shards overlap Swift tests", "ci.yml", "needs: [changes, rust-macos-build, swift, swift-tests]", "needs: [changes, rust-macos-build, swift]"),
            ("short Linux checks overlap Rust/Android", "ci.yml", "needs: [changes, rust-tests, android]", "needs: changes"),
            ("tunnel helper overlaps Rust/Android", "ci.yml", "needs: [changes, rust-tests, android]", "needs: [changes]"),
            ("Doc comments outside the budget", "doc-comments.yml", "group: background-macos", "group: doc-comments"),
            ("Site outside the budget", "site.yml", "group: background-macos", "group: site"),
            ("Web outside the budget", "web.yml", "group: background-macos", "group: web"),
            ("Relay outside the budget", "relay.yml", "group: background-macos", "group: relay"),
            ("Nightly Linux outside the budget", "nightly.yml", "  budget:\n    # Waiting for the shared budget consumes no runner (ov-1190).\n    concurrency:\n      group: background-macos\n      cancel-in-progress: false\n      queue: max\n", "  budget:\n"),
            ("maintenance outside the budget", "openai-prices.yml", "group: background-macos", "group: prices"),
            ("Promote outside the budget", "promote.yml", "group: background-macos", "group: promote"),
            ("total-job overflow", "ci.yml", "jobs:\n", "jobs:\n  overflow:\n    runs-on: ubuntu-latest\n    strategy:\n      matrix:\n        shard: [1, 2, 3, 4]\n"),
        ]
        for label, name, old, new in cases:
            changed = dict(texts)
            changed[name] = changed[name].replace(old, new)
            if changed[name] == texts[name] or not check(changed)[0]:
                problems.append("self-test did not reject " + label)
        nested = dict(texts)
        nested["canary.yml"] = nested["canary.yml"].replace("group: canary-ios", "group: background-macos")
        if not any("own workflow's lock" in p for p in check(nested)[0]):
            problems.append("parent/child lock deadlock escaped the model")
        # The review's simultaneous landing/Nightly/Canary/doc/site/web/relay
        # case. Removing their shared queue recreates the rejected policy.
        simultaneous = dict(texts)
        for name in ("site.yml", "web.yml", "relay.yml", "doc-comments.yml"):
            simultaneous[name] = simultaneous[name].replace("group: background-macos", "group: private")
        simultaneous_problems, simultaneous_counts, _ = check(simultaneous)
        if (not all(any(name in problem for problem in simultaneous_problems)
                    for name in ("site.yml", "web.yml", "relay.yml", "doc-comments.yml"))
                or total > 20 or simultaneous_counts["site.yml"] != (2, 0)):
            problems.append("simultaneous deployment/landing reservation regression")
        new_workflow = dict(texts, **{"new.yml": "on:\n  push:\njobs:\n  job:\n    runs-on: ubuntu-latest\n"})
        if not check(new_workflow)[0]:
            problems.append("a newly discovered workflow escaped the budget")
        # Scheduled/full runs retain every shard; targeted exclusions may only
        # reduce that width, and changes to their shape require review.
        targeted = proto.workflow_jobs(texts["nightly.yml"])["ios-ui"]
        if widths(targeted, texts) != (1, 1):
            problems.append("targeted Nightly matrix lost its conservative full-run bound")
        malformed = dict(targeted, strategy=targeted["strategy"].replace("'__none__'", "'shell'", 1))
        try:
            widths(malformed, texts)
        except ValueError:
            pass
        else:
            problems.append("unreviewed Nightly exclusions escaped the model")
        # Exercise graph width independently of current hardcoded job counts.
        graph = {"a": {}, "b": {}, "c": {"needs": "[a, b]"}}
        if peak(graph, {"a": 2, "b": 1, "c": 4}) != 4:
            problems.append("weighted dependency antichain self-test failed")
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 1
    print(f"Mac capacity: landing {counts['ci.yml'][1]} + shared background 1 = 5; total-job upper bound {total}/20 (incl. one Nightly yield gate)")
    print("All triggers/path matches conservatively overlap; reusable calls expanded inside their caller budget")
    print("Workflow peaks (total, macOS): " + str(counts))
    return 0


if __name__ == "__main__":
    sys.exit(main())
