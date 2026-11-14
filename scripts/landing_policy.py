"""Local landing policy (ov-1185); CI and plain lane-gate retain full proof.

Families have private GitHub environment files. Apple jobs share SwiftPM output
and run serially. Cargo arbitrates its shared target directory with its own
lock; do not give each worker a cold target. Admission still uses scripts/heavy.
"""
from concurrent.futures import ThreadPoolExecutor
import json
import os
import shlex
import subprocess
import time


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
    shadow = load_module("landing_rust_shadow", repo / "scripts/rust-shadow.py")
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
                                             "--format-version", "1", "--all-features"))
        packages, reasons, full = shadow.select(repo, metadata, changes)
        return {"full": full, "packages": packages, "reasons": reasons, "base": green}
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as exc:
        return {"full": True, "packages": [], "reasons": [f"full: {exc}"]}


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
            if job == "rust" and "python3 scripts/rust-shadow.py --out " in step.run:
                # Shadow runs full + inventories + selected replay; landing runs
                # once, still under exactly the leak wrappers CI supplies.
                _, separator, command = step.run.partition(" -- ")
                if not separator or "cargo nextest run --profile ci --workspace --no-fail-fast" not in command:
                    raise ValueError("Rust CI command changed; refusing to narrow an unknown shape")
                step.run = command.rstrip() + "\n"
                if not proposal["full"]:
                    expression = " or ".join(f"package(={p})" for p in proposal["packages"])
                    if not expression:
                        raise ValueError("empty Rust package selection")
                    step.run = step.run.rstrip() + " -E " + shlex.quote(expression) + "\n"


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
    def worker(jobs):
        own_rows = []
        failed = False
        for job in jobs:
            job_tmp = tmp / job
            job_tmp.mkdir()
            try:
                failed = run_one_job(job, plans, own_rows, repo, job_tmp, ctx, logdir, args) or failed
            except Exception as exc:
                own_rows.append((job, "family runner", f"FAIL: {type(exc).__name__}: {exc}", "", ""))
                failed = True
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
