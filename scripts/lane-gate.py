#!/usr/bin/env python3
"""Run the CI jobs a lane's change reaches, and print what ran (ov-799).

Plain lane-gate retains full suites; --landing is the local engine policy
(ov-1185): bounded parallel families, conservative Rust selection against a
verified green ancestor, release bundle proof left to CI, and warm target kept.
CI on main remains full proof. Each selection/fallback is saved with step logs.

A lane that says "gated alone" has to have run every full suite CI will run for
the paths it touched, under the same wrappers (test-leak-check, tmux-leak-check,
swift-test-guard, nextest --profile ci). That used to be recalled by hand from
.github/workflows/ci.yml, and a lane would run a filtered suite or skip a lint.

    scripts/lane-gate.py                      # this worktree against origin/main
    scripts/lane-gate.py --dry-run            # the plan only: nothing runs
    scripts/lane-gate.py --only wire,relay    # or --skip android
    scripts/lane-gate.py --base main --log-dir /tmp/gate-logs
    scripts/lane-gate.py --self-test

Each real run also saves its summary, with the full HEAD sha it gated, to
`<git-dir>/lane-gate-summary.txt` (one per worktree); scripts/gate-summary-check.py
reads it, and the push guard calls that before a single-lane push (ov-822).

It prints a Markdown table (job, command, result, seconds, test counts) to paste
into a lane report, and exits non-zero if any step failed.

Nothing here is a second copy of CI. The commands are read out of ci.yml (and
doc-comments.yml) at run time: each job's `run:` steps, in order, with the
`${{ }}` expressions they use filled in. Steps that only install a toolchain on
a CI runner (apt-get, brew, sdkmanager, the tmux build) are skipped, and so are
a few steps that need the Linux toolchain the web job downloads; every skip is
listed in the output with its reason. A new job in ci.yml that this script does
not know fails the self-test.

Which jobs a change reaches (ov-1189). ci-changes.py's families are the minimum
coverage: their union over the combined diff selects the platform suites.
Wire and doc-comments always run; docs and reports need no builds. Tunnel and
site retain CI coverage for every non-doc change. Cross-cutting
inputs (Cargo.lock, the protocol crate, workflows, unknown paths) run everything.
The ov-1116 reverse crate closure is printed for Rust changes. It remains an
impact diagnostic: CI still runs the full Rust suite, so this gate retains it,
including doctests and feature checks, rather than enabling shadow skips here.

The ios job runs the steps of .github/actions/ios-ui-build that compile: the
Rust xcframeworks, the generated project, and `xcodebuild build-for-testing` of
the FarCooler scheme, so a compile error in the app or FarCoolerUITests is red
here as it is in CI (ov-812).

Not run here, and said so in the output: the iOS UI smoke (a simulator; use
scripts/ios-ui-tests.sh, one simulator at most) and the parts of the web job
that download Linux binaries. A heavy job waits while the 1-minute load average
is 20 or more.

Only one heavy job runs on this Mac at a time (ov-831). Ten lanes each running a
full gate pushed the load past 190 and made timing tests fail for no reason, so a
gate takes a place in a first-come, first-served queue (scripts/heavy_lock.py, shared with
scripts/heavy) before its first heavy job. A waiting gate says who holds it (worktree, PID,
start time) and how many are ahead. The place is held for the gate's whole run of heavy jobs, so
gates finish one after another, oldest first; --release-between-jobs queues again per job, which
interleaves gates and makes each take as long as all of them. Light jobs (wire, doc-comments,
relay, site) never take it. --no-lock turns it off, and it is off by default when
the CI environment variable is set: CI runners are separate machines.

A lane worktree's Rust target/ was 10-84 GB and filled the disk (ov-837). In a linked worktree the rust job builds
without debug info and keeps target/ under 20 GB (scripts/heavy_lock.py has the measurements), and a fully green
gate removes target/ at the end; --keep-target keeps it. FARCOOLER_LEAN_BUILD=off opts out of the first two.
"""

import argparse
import contextlib
import importlib.util
import json
import os
import pathlib
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
import process_guard
import heavy_lock as hl  # noqa: E402  the machine-wide lock, shared with scripts/heavy
from heavy_lock import LOCK_POLL, machine_lock  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent

# Order is the order they run in: cheap and fast to fail first.
JOBS = ["wire", "doc-comments", "relay", "site", "tunnel", "rust", "swift-tests", "swift", "ios", "android", "web"]
HEAVY = {"tunnel", "rust", "swift-tests", "swift", "ios", "android", "web"}
APP_JOBS = {"rust", "swift-tests", "swift", "ios", "android", "tunnel"}
# ci.yml jobs this script covers under another name, or on purpose does not.
CI_JOB_ALIASES = {
    "wire": "wire", "tunnel": "tunnel", "tunnel-helper": "tunnel", "rust": "rust", "swift": "swift", "swift-tests": "swift-tests",
    "rust-build": "rust", "rust-tests": "rust", "rust-macos-tests": "rust", "rust-macos-build": "rust",
    "ios": "ios", "ios-ui-plan": "ios", "android": "android", "android-build": "android", "android-ui": "android", "relay": "relay", "site": "site", "web": "web",
}
CI_JOBS_NOT_RUN = {
    "changes": "decides which jobs CI runs; this script does that from the diff",
    "ios-ui": "a simulator run: scripts/ios-ui-tests.sh",
}

# Steps that set up a CI runner rather than check anything.
SETUP = re.compile(r"apt-get|brew install|sdkmanager|ci-install-tmux\.sh|Install git-lfs|Install fish|sshd -V")
# What the web job downloads for Linux x86 (wasm-bindgen, binaryen, Chromium),
# through scripts/gh-download.sh since ov-1030 (curl before it).
WEB_LINUX_ONLY = re.compile(
    r"gh-download\.sh|curl -fsSL|playwright|WASM_BINDGEN|WASM_OPT|wasm-stable|wasm-canary|size-gate\.mjs dist|fake_daemon|crates/web/build\.sh|\./build\.sh"
)
# The ios job's own steps build an archive; its plan job's checks are the cheap half.
IOS_KEEP = re.compile(r"import-ios-certificate-test|generator-test|ios-ui-shards\.py|ios-ui-pick-simulator\.py|ios-ui-plan\.py --self-test")


# The steps of ios-ui-build that compile. The rest check the runner's Xcode
# label, install the watchOS platform (several GB, into the system Xcode) or
# package the products for CI's UI shards.
IOS_BUILD_KEEP = re.compile(r"build-ios-frameworks\.sh|generate-project\.py|^xcodebuild -project")


class Step:
    def __init__(self, job, name, run, cwd=None, env=None, skip=None):
        self.job, self.name, self.run, self.cwd, self.env, self.skip = job, name, run, cwd, env or {}, skip

    def label(self):
        lines = [l.strip() for l in self.run.splitlines() if l.strip() and not l.strip().startswith(("#", "set -"))]
        first = lines[0] if lines else self.name
        if len(lines) > 1:
            first += " ..."
        return first


# ---------------------------------------------------------------- reading ci.yml


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True
    spec.loader.exec_module(module)
    return module


def job_block(text, job):
    """The lines of one job under `jobs:`, or None."""
    lines = text.splitlines()
    start = None
    for i, line in enumerate(lines):
        if line == f"  {job}:":
            start = i + 1
            break
    if start is None:
        return None
    out = []
    for line in lines[start:]:
        if re.match(r"^  [A-Za-z0-9_-]+:\s*$", line):
            break
        out.append(line)
    return out


def job_ids(text):
    in_jobs = False
    ids = []
    for line in text.splitlines():
        if line.startswith("jobs:"):
            in_jobs = True
        elif in_jobs and re.match(r"^  [A-Za-z0-9_-]+:\s*$", line):
            ids.append(line.strip()[:-1])
    return ids


def env_scalar(value):
    """The plain/single/double-quoted environment scalars ci.yml uses."""
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] == "'":
        return value[1:-1].replace("''", "'")
    if len(value) >= 2 and value[0] == value[-1] == '"':
        return json.loads(value)
    return value


def parse_steps(block):
    """[(name, run, working-directory, env)] for each step of a job with a `run:`.
    `uses:` steps have no commands and are not returned. Reads only the shapes
    ci.yml uses: a `run:` that is inline or a `|` block, step-level `env:` and
    `working-directory:`, and a job-level `defaults: run: working-directory:`."""
    default_wd = None
    job_env = {}
    for i, line in enumerate(block):
        if line.startswith("        working-directory:") and block[i - 1].strip() == "run:":
            default_wd = line.split(":", 1)[1].strip()
    in_env = False
    for line in block:
        if line.startswith("    env:"):
            in_env = True
            continue
        if in_env:
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            m = re.match(r"^      ([A-Z_0-9]+): (.*)$", line)
            if not m:
                break
            job_env[m.group(1)] = env_scalar(m.group(2))
    steps, cur = [], None
    in_steps = False
    for line in block:
        if line.startswith("    steps:"):
            in_steps = True
            continue
        if not in_steps:
            continue
        if line.startswith("      - "):
            cur = []
            steps.append(cur)
            line = "        " + line[8:]
        if cur is not None:
            cur.append(line)
    out = []
    for lines in steps:
        name, run, wd, env = None, None, default_wd, dict(job_env)
        i = 0
        while i < len(lines):
            line = lines[i]
            m = re.match(r"^        (name|run|working-directory): ?(.*)$", line)
            if m:
                key, val = m.groups()
                if key == "name":
                    name = val.strip()
                elif key == "working-directory":
                    wd = val.strip()
                elif val.strip() in ("|", "|-", ">"):
                    body = []
                    i += 1
                    while i < len(lines) and (lines[i].startswith("          ") or not lines[i].strip()):
                        body.append(lines[i][10:])
                        i += 1
                    run = "\n".join(body).rstrip("\n") + "\n"
                    continue
                else:
                    run = val.strip() + "\n"
            elif line.startswith("        env:"):
                i += 1
                while i < len(lines) and lines[i].startswith("          "):
                    if lines[i].strip() and not lines[i].lstrip().startswith("#"):
                        k, _, v = lines[i].strip().partition(":")
                        env[k] = env_scalar(v)
                    i += 1
                continue
            i += 1
        if run is not None:
            out.append((name or "", run, wd, env))
    return out


def action_as_job(text):
    """A composite action's lines shaped like a ci.yml job block, for parse_steps.

    The action nests `steps:` two spaces in and a job nests it four, so every
    line moves two to the right and the parser reads it unchanged."""
    return ["  " + l for l in text.splitlines()]


def expand(text, ctx):
    """Fill `${{ expr }}`. An expression this does not know is an error, never a blank."""
    def sub(m):
        expr = m.group(1).strip()
        if expr in ctx:
            return ctx[expr]
        raise KeyError(f"no value for ${{{{ {expr} }}}} in ci.yml; teach scripts/lane-gate.py")
    return re.sub(r"\$\{\{(.*?)\}\}", sub, text)


def web_asset_skips(steps):
    """Carry only the web asset producer's deliberate skip to its dry-run.

    A producer that runs (even if it later fails) must not hide a consumer's
    failure. Track channels separately because both builds write the same dist.
    """
    producers = {}
    for step in steps:
        build = re.search(r'WEB_WASM_DIR=.*?\./build\.sh --channel (stable|canary)\b', step.run)
        if build:
            producers[build.group(1)] = step.skip
        if re.search(r'\bnpx wrangler deploy --dry-run\b', step.run):
            channel = "canary" if re.search(r'--env\s+canary\b', step.run) else "stable"
            if producers.get(channel) and not step.skip:
                step.skip = f"{channel} web asset build skipped ({producers[channel]}); dry-run left to CI"


def steps_for(job, workflows, ctx):
    """The Steps of one job, skip reasons attached."""
    text = workflows["ci"]
    if job == "doc-comments":
        base = ctx["base"]
        return [
            Step(job, "self-test", "./scripts/doc-comment-check.py --self-test\n"),
            Step(job, "this lane's commits", f"./scripts/doc-comment-check.py --base {base} --head HEAD\n"),
        ]
    ids = ["ios-ui-plan", "ios"] if job == "ios" else [job]
    if job == "android":
        ids = ["android-build", "android-ui:first", "android-ui:rest", "android"]
    if job == "tunnel":
        ids = ["tunnel", "tunnel-helper"]
    steps = []
    for ci_spec in ids:
        ci_id, _, shard = ci_spec.partition(":")
        step_ctx = {**ctx, "matrix.shard": shard} if shard else ctx
        block = job_block(text, ci_id)
        if block is None:
            raise KeyError(f"ci.yml has no job {ci_id}")
        parsed = [(None, *p) for p in parse_steps(block)]
        if ci_id == "ios":
            # The job's build is a composite action, which parse_steps never sees
            # as it has no `run:` in the job itself; CI runs it first.
            parsed = [(True, *p) for p in parse_steps(action_as_job(workflows["ios-ui-build"]))] + parsed
        for from_action, name, run, wd, env in parsed:
            skip = None
            hay = name + "\n" + run
            if from_action:
                if not IOS_BUILD_KEEP.search(run):
                    skip = "checks the runner, installs the watchOS platform or packages products for CI's UI shards"
            elif SETUP.search(hay):
                skip = "installs a toolchain on a CI runner"
            elif name == "Green bases for Rust shadow (ov-1116)":
                skip = "CI lists verified green bases; local shadow safely proposes full without that evidence"
            elif job == "rust" and name == "macOS native Rust tests":
                skip = "local gate runs the full Rust suite; the filtered Mac CI replay would duplicate it"
            elif job == "web" and WEB_LINUX_ONLY.search(hay):
                skip = "needs the Linux x86 wasm toolchain CI downloads; left to CI"
            elif job == "ios" and not IOS_KEEP.search(run):
                skip = "decides what CI runs, or builds the iOS archive; see scripts/ios-ui-tests.sh"
            if not skip:  # a skipped step is never run, so its expressions need no value
                env = {k: expand(v, step_ctx) for k, v in env.items()}
                run = expand(run, step_ctx)
            steps.append(Step(job, name, run, wd, env, skip))
    if job == "rust":
        # CI runs one prebuilt archive in partitions. The local equivalent is
        # one unfiltered workspace run, under the same wrappers and profile.
        # Read the command from CI; reject an unfamiliar or narrowed prefix.
        shard = job_block(text, "rust-tests")
        full = [p for p in parse_steps(shard or []) if p[0] == "Full Rust workspace shard"]
        if len(full) != 1:
            raise ValueError("CI must define exactly one full Rust workspace shard command")
        _name, run, wd, env = full[0]
        prefix, archive, _remote = run.partition(" --archive-file ")
        match = re.fullmatch(
            r"(\./scripts/test-leak-check\.py -- \./scripts/tmux-leak-check\.py -- (?:python3 scripts/quarantine\.py run -- )?cargo nextest run --profile \S+) --no-fail-fast",
            prefix.strip())
        if not archive or not match:
            raise ValueError("cannot derive an unfiltered local Rust suite from CI's shard command")
        local = Step(job, "Full Rust workspace (local)",
                     match[1] + " --workspace --no-fail-fast\n", wd, env)
        before_doc = next((i for i, step in enumerate(steps)
                           if step.run.strip() == "cargo test --workspace --doc"), len(steps))
        steps.insert(before_doc, local)
    if job == "web":
        web_asset_skips(steps)
    return steps


# ---------------------------------------------------------------- which jobs


def changed_inputs(repo, base):
    """The combined branch, index, worktree and untracked diff; retain deletions
    and both sides of renames so moving code into docs cannot hide its impact."""
    def git(*args):
        return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)
    mb = git("merge-base", base, "HEAD")
    if mb.returncode != 0:
        raise SystemExit(f"lane-gate: no merge base between {base} and HEAD: {mb.stderr.strip()}")
    diff = git("diff", "--no-renames", "--name-status", "-z", mb.stdout.strip())
    untracked = git("ls-files", "--others", "--exclude-standard", "-z")
    for out in (diff, untracked):
        if out.returncode != 0:
            raise SystemExit(f"lane-gate: git failed: {out.stderr.strip()}")
    raw = diff.stdout.split("\0")[:-1]
    entries = set(zip(raw[::2], raw[1::2]))
    entries.update(("A", f) for f in untracked.stdout.split("\0") if f)
    return sorted(entries)


def changed_files(repo, base):
    return sorted({f for _, f in changed_inputs(repo, base)})


# CI's apple condition covers all three jobs, including the iOS build even when
# ios-ui-plan skips simulator smoke. Never narrow below that condition.
FAMILY_JOBS = {"rust": {"rust"}, "apple": {"swift", "swift-tests", "ios"},
               "android": {"android"}, "web": {"web"}, "relay": {"relay"}}


def jobs_for(files, site, ios_relevant, commands, families):
    """{job: [paths]}. CI families are a floor, never overridden by local rules.

    Keep the site/ios inputs in this public API for gate_evidence's canonical
    planner. Unknown scripts and workflows take CI's full fallback too.
    """
    reach = {"wire": [], "doc-comments": []}

    def add(jobs, f):
        for j in jobs:
            reach.setdefault(j, [])
            if f not in reach[j]:
                reach[j].append(f)

    for f in files:
        flags = families([f])
        for family, jobs in FAMILY_JOBS.items():
            if flags[family]:
                add(jobs, f)
        if all(flags[k] for k in FAMILY_JOBS):
            add(set(JOBS), f)
        if flags["non_docs"]:
            add({"site", "tunnel"}, f)
    return reach


def rust_impact(repo, inputs):
    """Reuse ov-1116's conservative crate selector without enabling its skips.

    Offline metadata resolves all features/targets, never builds or downloads.
    Missing metadata or selector evidence keeps the full Rust suite.
    """
    if not inputs or any(status not in ("A", "M")
                         or not re.fullmatch(r"crates/(?!protocol/)[^/]+/(?:src|tests|examples)/.+\.rs", path)
                         for status, path in inputs):
        return "full workspace (cross-cutting, deleted or unknown input)"
    try:
        shadow = load_module("rust_shadow", repo / "scripts/rust-shadow.py")
        metadata = subprocess.run(["cargo", "metadata", "--offline", "--locked",
                                   "--format-version", "1", "--all-features"],
                                  cwd=repo, capture_output=True, text=True, check=True)
        import json
        packages, reasons, full = shadow.select(repo, json.loads(metadata.stdout), inputs)
        return ("full workspace" if full else ", ".join(packages)) + "; " + "; ".join(reasons)
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as exc:
        return f"full workspace (impact unavailable: {type(exc).__name__})"


# ---------------------------------------------------------------- running


def wait_for_load(limit=20.0, poll=30):
    hl.wait_for_load(limit, poll)


COUNTS = [
    (re.compile(r"Summary \[.*?\]\s+(\d+) tests? run: (\d+) passed"), "nextest", 2),
    (re.compile(r"test result: ok\. (\d+) passed"), "doc/unit", 1),
    (re.compile(r"Test run with (\d+) tests?"), "swift-testing", 1),
    (re.compile(r"Tests\s+(\d+) passed"), "vitest", 1),
    (re.compile(r"UI tests that ran: (\d+) in"), "ui", 1),
    (re.compile(r"^ok\s+\S+\s+[\d.]+s", re.M), "go pkgs", 0),
]


def count_tests(output):
    parts = []
    for rx, label, group in COUNTS:
        found = list(rx.finditer(output))
        if not found:
            continue
        n = len(found) if group == 0 else sum(int(m.group(group)) for m in found)
        parts.append(f"{n} {label}")
    ex = re.findall(r"Executed (\d+) tests?, with (\d+) failures?", output)
    if ex:
        parts.append(f"{ex[-1][0]} xctest")
    if "BUILD SUCCESSFUL" in output:
        parts.append("gradle ok")
    return ", ".join(parts)


def jdk_major(home):
    """The major version of the JDK at `home`, read from its `release` file; 0 when it is not one."""
    try:
        text = (pathlib.Path(home) / "release").read_text()
    except OSError:
        return 0
    m = re.search(r'JAVA_VERSION="(?:1\.)?(\d+)', text)
    return int(m.group(1)) if m else 0


def jdk_candidates():
    """Every JDK home this host has, best-known places first."""
    homes = []
    for prefix in ("/opt/homebrew/opt", "/usr/local/opt"):
        homes += sorted(str(p) for p in pathlib.Path(prefix).glob("openjdk@*/libexec/openjdk.jdk/Contents/Home"))
        homes += sorted(str(p) for p in pathlib.Path(prefix).glob("openjdk@*"))
    try:
        out = subprocess.run(["/usr/libexec/java_home", "-V"], capture_output=True, text=True)
        homes += re.findall(r"\s(/\S.*?/Home)\s*$", out.stderr + out.stdout, re.M)
    except OSError:
        pass
    homes += sorted(str(p) for p in pathlib.Path("/Library/Java/JavaVirtualMachines").glob("*/Contents/Home"))
    homes += sorted(str(p) for p in pathlib.Path("/usr/lib/jvm").glob("*"))
    return homes


def java_home(version, homes=None):
    """A JDK home whose major version is `version`, else the oldest one above it, else "".

    `/usr/libexec/java_home -v 17` is not trusted: on this Mac, with only JDK 11
    installed, it exits 0 and prints the JDK 11 path, so a gradle build ran on
    11 and died (ov-802). Each candidate's own `release` file says what it is.
    """
    found = {}
    for home in jdk_candidates() if homes is None else homes:
        major = jdk_major(home)
        if major and major not in found:
            found[major] = home
    if version in found:
        return found[version]
    newer = sorted(m for m in found if m > version)
    return found[newer[0]] if newer else ""


def job_env(job):
    """Local stand-ins for what a CI runner's setup actions provide."""
    env = {}
    if job == "android":
        jdk17 = java_home(17)
        if jdk17:
            env["JAVA_HOME"] = jdk17
    return env


def run_step(step, repo, env, logdir, tmp, fail_fast=False):
    gh_env, gh_path = tmp / "github_env", tmp / "github_path"
    gh_output = tmp / "github_output"
    gh_env.write_text("")
    gh_path.write_text("")
    gh_output.write_text("")
    e = {**env, **step.env, "GITHUB_ENV": str(gh_env), "GITHUB_PATH": str(gh_path), "GITHUB_OUTPUT": str(gh_output)}
    cwd = repo / step.cwd if step.cwd else repo
    log = logdir / f"{step.job}-{abs(hash(step.run)) % 10**8}.log"
    started = time.time()
    with open(log, "w") as f:
