"""Local landing policy (ov-1185); CI and plain lane-gate retain full proof.

Families have private GitHub environment files. Apple jobs share SwiftPM output
and run serially. Cargo arbitrates its shared target directory with its own
lock; do not give each worker a cold target. Admission still uses scripts/heavy.
"""
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import threading
import time


# Reviewed transitive consumers, not an inference from missing command text.
# The retry helper is imported by ci-runner-retry-test.py; that test reads the
# workflow and runs in wire. The workflow itself only operates the GitHub API.
# No platform build/test consumes these three inputs (ov-1324). All other
# scripts/workflows keep CI's full fallback until their users are reviewed.
SCRIPT_USERS = {
    "scripts/ci-runner-retry.py": {"wire"},
    "scripts/ci-runner-retry-test.py": {"wire"},
    ".github/workflows/ci-runner-retry.yml": {"wire"},
}


def script_users(path, commands):
    """Reviewed consumers plus any newly named gate users; None means unknown.

    Command text alone cannot prove absence of transitive consumers. Never
    narrow an unreviewed path just because a job does not name it directly.
    """
    if path not in SCRIPT_USERS:
        return None
    users = set(SCRIPT_USERS[path])
    for job, command in commands.items():
        if path in command:
            users.add(job)
    return users


def workflow_checks(plans, files, step_type):
    """Keep workflow validation without selecting a platform build for it."""
    if any(path.startswith(".github/workflows/") for path in files):
        plans["wire"].extend([
            step_type("wire", "Workflow lint", "./scripts/actionlint.sh\n"),
            step_type("wire", "Workflow shard check", "./scripts/ios-ui-shards.py --check\n"),
        ])


def fold_base(repo, given):
    """Read the same exact workspace/item/candidate index as daemon process_base.

    Only durable evidence can narrow an old runner base. Remote refs, commit
    subjects and HEAD's parent cannot prove where a multi-commit fold began.
    Keep the given base on missing, foreign, malformed or non-ancestral evidence.
    """
    def git(*args):
        return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)

    try:
        workspace, item, candidate, receipt = (os.environ.get(key, "") for key in
            ("FARCOOLER_WORKSPACE", "FARCOOLER_WORKFLOW_ITEM", "FARCOOLER_CANDIDATE", "FARCOOLER_RECEIPT"))
        if not all((workspace, item, candidate, receipt)):
            raise ValueError("missing fold context")
        head = git("rev-parse", "HEAD")
        if head.returncode or candidate != head.stdout.strip():
            raise ValueError("candidate differs from HEAD")
        # farcooler_workflow::digest serializes this tuple as compact UTF-8 JSON.
        identity = json.dumps([workspace, item], ensure_ascii=False, separators=(",", ":")).encode()
        name = f"fold-wf1:{hashlib.sha256(identity).hexdigest()}-{candidate}.json"
        record = json.loads((Path(receipt).parent / name).read_text())
        if not isinstance(record, dict) or record.get("item") != item or record.get("folded") != candidate:
            raise ValueError("foreign fold record")
        base = record.get("base")
        if not isinstance(base, str) or not re.fullmatch(r"[0-9a-fA-F]{40}", base):
            raise ValueError("invalid fold base")
        for older, newer in ((given, base), (base, candidate)):
            if git("merge-base", "--is-ancestor", older, newer).returncode:
                raise ValueError("fold base does not advance the given base within candidate history")
    except (OSError, ValueError) as error:
        print(f"lane-gate: fold base unavailable ({error}); falling back to given base {given}")
        return given
    if base.lower() == given.lower():
        return given
    print(f"lane-gate: using fold base {base} instead of stale given base {given}")
    return base


def readonly_git(env):
    """Suppress optional index writes while retaining dirty reads and real writes."""
    env.setdefault("GIT_OPTIONAL_LOCKS", "0")
    # Git diff's automatic refresh can still write with optional locks off.
    # Append command-scoped config without replacing caller-provided entries.
    count = int(env.get("GIT_CONFIG_COUNT", "0"))
    env[f"GIT_CONFIG_KEY_{count}"] = "diff.autoRefreshIndex"
    env[f"GIT_CONFIG_VALUE_{count}"] = "false"
    env["GIT_CONFIG_COUNT"] = str(count + 1)


def budget_worker(env, workers):
    """Share the host between families instead of each builder taking every CPU.

    Tests still use CI's profile and groups. Only local scheduling changes;
    Cargo and Go compilation concurrency do not alter artifact contents.
    """
    # gen-wire-types rewrites identical tracked outputs, then git status/diff
    # refresh their stat cache. Protocol watches the index for its build stamp;
    # that optional refresh otherwise invalidates its entire reverse closure.
    # Reads still inspect the worktree and real git writes still take locks.
    readonly_git(env)
    share = max(1, (os.cpu_count() or 1) // workers)
    # Cargo serializes writers to a shared profile. A per-family fraction
    # leaves that single active compiler underfed while other families test.
    # Cap it at four, retaining room for Swift/Gradle and explicit user limits.
    env.setdefault("CARGO_BUILD_JOBS", str(max(share, min(4, os.cpu_count() or 1))))
    env.setdefault("GOMAXPROCS", str(share))
    # Most integration tests wait on I/O. Dividing this pool between compiler
    # families serializes those waits; six is the measured stable local pool
    # documented in nextest.toml. Keep CI's groups and stricter user limits.
    env.setdefault("NEXTEST_TEST_THREADS", str(min(6, os.cpu_count() or 1)))
    env["GOFLAGS"] = (env.get("GOFLAGS", "") + f" -p={share}").strip()
    env["GRADLE_OPTS"] = (env.get("GRADLE_OPTS", "") + f" -Dorg.gradle.workers.max={share}").strip()


def record_timing(logdir, job, step, result, started, ended):
    """Flush each completed step so an interrupted gate retains real timings.

    A job has one worker and one file, so parallel families never interleave
    JSON writes. These timestamps include preflight and Cargo lock waits.
    """
    row = {"job": job, "command": step.label(), "run": step.run,
           "result": result, "seconds": ended - started,
           "start_monotonic": started, "end_monotonic": ended}
    with (logdir / f"{job}-timings.jsonl").open("a") as stream:
        stream.write(json.dumps(row) + "\n")


def rust_selection(repo, base, load_module):
    """Select against a verified green ancestor, including staged/dirty inputs.

    Missing network, metadata, green history, or unknown paths means full. Never
    treat the previous commit or an arbitrary --base as proof of a green base.
    """
    shadow = load_module("landing_rust_select", repo / "scripts/rust-select.py")
    try:
        candidates_file = os.environ.get("CANDIDATES")
        if candidates_file:
            with open(candidates_file) as stream:
                candidates = [json.loads(line)["head_sha"] for line in stream if line.strip()]
        else:
            raw = subprocess.run([
                "gh", "api", "repos/{owner}/{repo}/actions/workflows/ci.yml/runs?branch=main&event=push&status=success&per_page=100",
                "--jq", ".workflow_runs[].head_sha"], cwd=repo, capture_output=True, text=True,
                check=True, timeout=30).stdout
            candidates = raw.splitlines()
        head = shadow.capture(repo, "git", "rev-parse", "HEAD").strip()
        green = shadow.green_base(repo, head, candidates)
        if not green:
            raise ValueError("no verified green ancestor")
        # Include BOTH the requested range and changes since green, plus dirty
        # files. Unknown untracked files force full just like tracked unknowns.
        merge = shadow.capture(repo, "git", "merge-base", base, "HEAD").strip()
        changes = []
        for start in (green, merge):
            raw = shadow.capture(repo, "git", "diff", "--no-renames", "--name-status", "-z", start).split("\0")
            changes.extend(zip(raw[0:-1:2], raw[1:-1:2]))
        raw = shadow.capture(repo, "git", "ls-files", "--others", "--exclude-standard", "-z")
        changes.extend(("A", p) for p in raw.split("\0") if p)
        metadata = json.loads(shadow.capture(repo, "cargo", "metadata", "--locked", "--offline",
                                             "--format-version", "1", "--all-features", "--no-deps"))
        def workflow_texts(path):
            return shadow.capture(repo, "git", "show", f"{green}:{path}"), (repo / path).read_text()
        packages, reasons, full = shadow.select(repo, metadata, changes, workflow_texts)
        return with_binaries(shadow, metadata, {"full": full, "packages": packages, "reasons": reasons, "base": green})
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as exc:
        return {"full": True, "packages": [], "reasons": [f"full: {exc}"], "selected_binaries": [], "skipped_binaries": [],
                "binaries_note": "unavailable: selection failed before the workspace was read"}


def with_binaries(shadow, metadata, proposal):
    """Name the test binaries the proposal runs and skips, for the gate's evidence (ov-1278).

    A red main is traced to a skip by reading the last landing's rust-selection.json.
    """
    names = set(proposal["packages"])
    if proposal["full"]:
        names = {p["name"] for p in metadata["packages"] if p["id"] in set(metadata["workspace_members"])}
    run, skipped = shadow.test_binaries(metadata, names)
    return {**proposal, "selected_binaries": run, "skipped_binaries": skipped}


def forced_full(repo, load_module, reason):
    """The whole suite on request (--full-rust), still with every binary named in the evidence."""
    proposal = {"full": True, "packages": [], "reasons": [reason], "selected_binaries": [], "skipped_binaries": [],
                "binaries_note": "unavailable: workspace metadata could not be read"}
    try:
        shadow = load_module("landing_rust_select", repo / "scripts/rust-select.py")
        metadata = json.loads(shadow.capture(repo, "cargo", "metadata", "--locked", "--offline",
                                             "--format-version", "1", "--all-features", "--no-deps"))
        return with_binaries(shadow, metadata, {k: proposal[k] for k in ("full", "packages", "reasons")})
    except (OSError, ValueError, KeyError, subprocess.SubprocessError):
        return proposal


def apply(plans, proposal):
    """Change only the local default-host nextest; keep features/lints/docs full."""
    for job, steps in plans.items():
        for step in steps:
            if step.skip:
                continue
            if job == "web" and "GOOS=js GOARCH=wasm go test" in step.run:
                # Go/Wasm reserves only 8 KiB for argv + env. A developer's
                # App Store session is unrelated to this test and can exhaust
                # that space before any test starts; CI doesn't supply it.
                # Its runtime has no OS threads, unlike the host Go builders.
                step.env = {**step.env, "FASTLANE_SESSION": "", "GOMAXPROCS": "1"}
            if job == "swift" and (step.name == "Build Far Cooler.app"
                                   or "apps/macos/build/$APP_NAME.app" in step.run
                                   or step.name == "Resolve this channel's app name"):
                if "macos-entitlements-test.sh" in step.run:
                    step.run = "./scripts/macos-entitlements-test.sh\n"
                else:
                    step.skip = "local landing: release bundle and its consumers run in CI"
            apply_rust(job, step, proposal)


def apply_rust(job, step, proposal):
    """Narrow the default-host nextest step to the selected packages (ov-1268).

    Shared by the landing gate and the plain lane gate. Features, lints and
    doctests stay full, and CI on main always runs the whole suite.
    """
    if job == "rust" and step.name == "Full Rust workspace (local)":
        wrappers = "./scripts/test-leak-check.py -- ./scripts/tmux-leak-check.py -- "
        tail = "cargo nextest run --profile gate --no-fail-fast --archive-file "
        # CI wraps the run in the flaky-test quarantine runner (ov-1229);
        # the local run is the unpartitioned archive run (ov-1210), after
        # a line or two that link the channel aliases the tests spawn.
        lines = step.run.rstrip().split("\n")
        command = lines[-1]
        # The gate (not CI on main) also reruns what failed once, alone (ov-1285).
        rerun = "python3 scripts/gate-flaky.py run -- "
        if not (command.startswith(wrappers + tail)
                or command.startswith(wrappers + "python3 scripts/quarantine.py run -- " + tail)
                or command.startswith(wrappers + rerun + "python3 scripts/quarantine.py run -- " + tail)):
            raise ValueError("Rust CI command changed; refusing to narrow an unknown shape")
        if not proposal["full"]:
            expression = " or ".join(f"package(={p})" for p in proposal["packages"])
            if not expression:
                step.skip = "no Rust test input changed (docs and non-Rust files only); CI on main runs the full suite"
                return
            lines[-1] = command + " -E " + shlex.quote(expression)
            step.run = "\n".join(lines) + "\n"


def families(chosen):
    groups = {}
    for job in chosen:
        # swift build and tests mutate the same .build; iOS shares AgentKit.
        family = "apple" if job in {"swift-tests", "swift", "ios"} else job
        groups.setdefault(family, []).append(job)
    # Start the long critical paths first; light checks use the first freed
    # worker instead of delaying every heavy family by the wire suite.
    priority = {"rust": 0, "apple": 1, "android": 2, "rust-lints": 3}
    return [jobs for family, jobs in sorted(groups.items(), key=lambda item: priority.get(item[0], 4))]


def run(chosen, plans, rows, repo, tmp, ctx, logdir, args, run_one_job):
    """Bounded parallel families, ordered steps, deterministic complete results."""
    import copy
    args = copy.copy(args)
    args.stop_event = threading.Event() if getattr(args, "fail_fast", False) else None
    def worker(jobs):
        own_rows = []
        failed = False
        for job in jobs:
            if args.stop_event and args.stop_event.is_set():
                break
            job_tmp = tmp / job
            job_tmp.mkdir()
            try:
                failed = run_one_job(job, plans, own_rows, repo, job_tmp, ctx, logdir, args) or failed
            except Exception as exc:
                own_rows.append((job, "family runner", f"FAIL: {type(exc).__name__}: {exc}", "", ""))
                failed = True
            if failed and args.stop_event:
                args.stop_event.set()
        return failed, own_rows

    # Clippy does not consume the binaries/PATH exported by Rust's test setup.
    # Run its feature variants serially beside the test family. Cargo's build
    # lock still protects artifacts; test execution can overlap lint compilation.
    chosen, plans = list(chosen), dict(plans)
    if "rust" in chosen:
        lints = [s for s in plans["rust"] if s.run.lstrip().startswith("cargo clippy ")]
        if lints:
            plans["rust-lints"] = lints
            plans["rust"] = [s for s in plans["rust"] if s not in lints]
            chosen.append("rust-lints")
    started = time.monotonic()
    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = [pool.submit(worker, jobs) for jobs in families(chosen)]
        failed = False
        for future in futures:
            bad, job_rows = future.result()
            rows.extend(job_rows)
            failed = bad or failed
    if "ios" in chosen:
        rows.append(("ios", "iOS UI smoke", "not run here: CI; use scripts/ios-ui-tests.sh", "", ""))
    rows.append(("landing", "parallel families wall time", "FAIL" if failed else "pass",
                 f"{time.monotonic() - started:.2f}", ""))
    return failed
