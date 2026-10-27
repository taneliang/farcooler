#!/usr/bin/env python3
"""Run the CI jobs a lane's change reaches, and print what ran (ov-799).

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

Which jobs a change reaches. CI itself decides only "website and docs" versus
"app" (scripts/ci-changes.py, whose allowlist is imported here, not copied),
and then runs every app job. A lane cannot afford that for every change, so
this narrows by path, always towards running more:

  - Always: `wire` (the lints and script self-tests) and `doc-comments`.
  - Code shared by every app (crates/, proto/, Cargo.*, test/, toolchain files):
    rust, swift, swift-tests and android; ios when scripts/ios-ui-plan.py says
    the iOS app is built from the path; tunnel for crates/tailcat; web for
    crates/web.
  - apps/macos and apps/shared: swift and swift-tests. apps/ios: ios.
    apps/android: android. services/relay, services/site, services/web and
    services/common: their own jobs.
  - A file under scripts/: every job whose commands name that script.
  - .github/workflows/ci.yml: every job.
  - Any other path outside the site-only allowlist (one this table does not
    know): every app job, the same safe side ci-changes.py takes.

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
"""

import argparse
import contextlib
import importlib.util
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
import heavy_lock as hl  # noqa: E402  the machine-wide lock, shared with scripts/heavy
from heavy_lock import LOCK_POLL, machine_lock  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent

# Order is the order they run in: cheap and fast to fail first.
JOBS = ["wire", "doc-comments", "relay", "site", "tunnel", "rust", "swift-tests", "swift", "ios", "android", "web"]
HEAVY = {"tunnel", "rust", "swift-tests", "swift", "ios", "android", "web"}
APP_JOBS = {"rust", "swift-tests", "swift", "ios", "android", "tunnel"}
# ci.yml jobs this script covers under another name, or on purpose does not.
CI_JOB_ALIASES = {
    "wire": "wire", "tunnel": "tunnel", "rust": "rust", "swift": "swift", "swift-tests": "swift-tests",
    "ios": "ios", "ios-ui-plan": "ios", "android": "android", "relay": "relay", "site": "site", "web": "web",
}
CI_JOBS_NOT_RUN = {
    "changes": "decides which jobs CI runs; this script does that from the diff",
    "ios-ui": "a simulator run: scripts/ios-ui-tests.sh",
}

# Steps that set up a CI runner rather than check anything.
SETUP = re.compile(r"apt-get|brew install|sdkmanager|ci-install-tmux\.sh|Install git-lfs|Install fish|sshd -V")
# What the web job downloads for Linux x86 (wasm-bindgen, binaryen, Chromium).
WEB_LINUX_ONLY = re.compile(
    r"curl -fsSL|playwright|WASM_BINDGEN|WASM_OPT|wasm-stable|wasm-canary|size-gate\.mjs dist|fake_daemon|crates/web/build\.sh|\./build\.sh"
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
            m = re.match(r"^      ([A-Z_0-9]+): (.*)$", line)
            if not m:
                break
            job_env[m.group(1)] = m.group(2).strip()
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
                    k, _, v = lines[i].strip().partition(":")
                    env[k] = v.strip()
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
    steps = []
    for ci_id in ids:
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
            elif job == "web" and WEB_LINUX_ONLY.search(hay):
                skip = "needs the Linux x86 wasm toolchain CI downloads; left to CI"
            elif job == "ios" and not IOS_KEEP.search(run):
                skip = "decides what CI runs, or builds the iOS archive; see scripts/ios-ui-tests.sh"
            if not skip:  # a skipped step is never run, so its expressions need no value
                env = {k: expand(v, ctx) for k, v in env.items()}
                run = expand(run, ctx)
            steps.append(Step(job, name, run, wd, env, skip))
    return steps


# ---------------------------------------------------------------- which jobs


def changed_files(repo, base):
    """Repo-relative paths changed since the merge base with `base`: commits,
    uncommitted edits to tracked files, and untracked files that are not ignored."""
    def git(*args):
        return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)
    mb = git("merge-base", base, "HEAD")
    if mb.returncode != 0:
        raise SystemExit(f"lane-gate: no merge base between {base} and HEAD: {mb.stderr.strip()}")
    diff = git("diff", "--no-renames", "--name-only", mb.stdout.strip())
    untracked = git("ls-files", "--others", "--exclude-standard")
    for out in (diff, untracked):
        if out.returncode != 0:
            raise SystemExit(f"lane-gate: git failed: {out.stderr.strip()}")
    return sorted({f for f in diff.stdout.splitlines() + untracked.stdout.splitlines() if f})


def jobs_for(files, site, ios_relevant, commands):
    """{job: [the paths that reach it]}. `commands` is {job: all its run text}."""
    reach = {"wire": [], "doc-comments": []}

    def add(jobs, f):
        for j in jobs:
            reach.setdefault(j, [])
            if f not in reach[j]:
                reach[j].append(f)

    shared = {"rust", "swift", "swift-tests", "android"}
    for f in files:
        if f.startswith("services/relay/"):
            add({"relay"}, f)
        elif f.startswith("services/site/"):
            add({"site"}, f)
        elif f.startswith(("services/web/", "services/common/")):
            add({"web", "site"}, f)
        elif site.match(f):
            continue  # docs, a root .md, the site workflows: only the lints above read them
        elif f == ".github/workflows/ci.yml":
            add(set(JOBS), f)
        elif f.startswith(".github/workflows/"):
            continue
        elif f.startswith(".github/actions/"):
            add({"ios"}, f)
        elif f.startswith("scripts/"):
            for j, text in commands.items():
                if f in text:
                    add({j}, f)
        elif f.startswith("apps/android/"):
            add({"android"}, f)
        elif f.startswith("apps/ios/"):
            add({"ios"}, f)
        elif f.startswith(("apps/macos/", "apps/shared/")):
            add({"swift", "swift-tests"}, f)
        elif f.startswith("crates/web/"):
            add(shared | {"web"}, f)
        elif f.startswith("crates/tailcat/"):
            add(shared | {"tunnel", "web"}, f)
        elif f.startswith(("crates/", "proto/", "test/")) or re.match(r"^(Cargo\.(toml|lock)|rust-toolchain.*|Cross\.toml|\.cargo/.*|\.config/.*)$", f):
            add(shared, f)
        else:
            add(APP_JOBS, f)  # unknown: the safe side
        if ios_relevant(f) and not f.startswith(("services/", ".github/workflows/")) and not site.match(f):
            add({"ios"}, f)
    return reach


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


def run_step(step, repo, env, logdir, tmp):
    gh_env, gh_path = tmp / "github_env", tmp / "github_path"
    gh_env.write_text("")
    gh_path.write_text("")
    e = {**env, **step.env, "GITHUB_ENV": str(gh_env), "GITHUB_PATH": str(gh_path)}
    cwd = repo / step.cwd if step.cwd else repo
    log = logdir / f"{step.job}-{abs(hash(step.run)) % 10**8}.log"
    started = time.time()
    with open(log, "w") as f:
        done = subprocess.run(["bash", "-e", "-c", step.run], cwd=cwd, env=e, stdout=f, stderr=subprocess.STDOUT)
    secs = time.time() - started
    output = log.read_text(errors="replace")
    # What the step left for the ones after it, as GitHub's files do.
    for line in gh_env.read_text().splitlines():
        k, _, v = line.partition("=")
        if k:
            env[k] = v
    for p in gh_path.read_text().splitlines():
        if p:
            env["PATH"] = p + os.pathsep + env["PATH"]
    return done.returncode, secs, output, log


NEXTEST_VERSION = re.search(r"cargo-nextest@([\d.]+)", (ROOT / ".github/workflows/ci.yml").read_text())
NEXTEST_HINT = f"cargo install cargo-nextest --locked --version {NEXTEST_VERSION.group(1) if NEXTEST_VERSION else '<the version in ci.yml>'}"


def blocker(step, repo, env):
    """Why `step` cannot run on this host, or "" when it can.

    A missing tool is a named FAIL row, never a crash and never a quiet pass.
    cargo-nextest is the case that matters: `cargo test` is not a stand-in. CI's
    `cargo nextest run --profile ci` runs one process per test, with the
    test-groups in .config/nextest.toml that keep the tmux and file-event tests
    from running beside each other; under `cargo test` a `static Mutex` that
    nextest makes useless would hide the failures nextest finds, and the
    gate would print green for a suite CI can still fail. So it says how to
    install the pinned version and is red until then (ov-802).
    """
    path = env.get("PATH", "")
    cwd = repo / step.cwd if step.cwd else repo
    if not cwd.is_dir():
        return f"working directory {step.cwd or cwd} does not exist (an earlier step should have made it)"
    if re.search(r"\bcargo nextest\b", step.run) and not shutil.which("cargo-nextest", path=path):
        return f"cargo-nextest is not installed; install it with: {NEXTEST_HINT}"
    if re.search(r"(^|\s)\./gradlew\b", step.run):
        home = env.get("JAVA_HOME", "")
        if jdk_major(home) < 17:
            return "gradle needs JDK 17 and this host has none; install it with: brew install openjdk@17"
        if "captureJdk" in step.run and jdk_major(env.get("CAPTURE_JDK", "")) < 21:
            return "the Android capture tests need JDK 21 and this host has none; install it with: brew install openjdk@21"
    for tool, hint in (("go", "brew install go"), ("npm", "install Node"), ("swift", "install Xcode"), ("xcodebuild", "install Xcode"), ("cargo", "install Rust with rustup")):
        if re.search(rf"(^|&&|\|\||[;|(])[ \t]*(?:\./)?{tool}[ \t]", step.run, re.M) and not shutil.which(tool, path=path):
            return f"{tool} is not installed; {hint}"
    return ""


# Known gaps in THIS host's environment, not in the code under test (ov-822). A failed step whose output
# matches one is tagged "[env-gap: NAME]" in its Result cell, and scripts/gate-summary-check.py lets a push
# through when every failure carries a tag it lists. Add one only for a gap the host owner can't close, and add it to
# GAP_NAMES in gate-summary-check.py too. The match is on output, so a step that fails for a gap AND a real reason is
# still waved through: the log named in the summary is where to look.
ENV_GAPS = (
    ("git-lfs-missing", re.compile(r"git-lfs[^\n]*(isn't|is not|not) (on PATH|installed)|'lfs' is not a git command"), re.compile(r"cargo nextest|cargo test")),
    ("wrangler-dry-run-without-dist", re.compile(r"(assets|dist)[^\n]*(does not exist|ENOENT|no such file)|ENOENT[^\n]*dist", re.I), re.compile(r"wrangler deploy --dry-run")),
)


def env_gap(step, output):
    """The name of the known environment gap that explains this failed step, or ""."""
    for name, in_output, in_step in ENV_GAPS:
        if in_step.search(step.run) and in_output.search(output):
            return name
    return ""


DISK_FULL = "the disk is full (no space left on device); free some space and run the gate again"
NO_SPACE = re.compile(r"No space left on device|os error 28\b|ENOSPC")


def run_step_safely(step, repo, env, logdir, tmp):
    """(result, seconds, counts, ok, log, output) for one step; never raises.

    A tool the host lacks, a directory a failed step never made, or anything
    else this script trips over becomes that step's FAIL row, so the summary
    table still prints for every step that did run (ov-802)."""
    reason = blocker(step, repo, {**env, **step.env})
    if reason:
        return f"FAIL: {reason}", 0.0, "", False, None, reason
    try:
        code, secs, output, log = run_step(step, repo, env, logdir, tmp)
    except Exception as e:  # noqa: BLE001 -- any crash is this step's failure
        reason = DISK_FULL if hl.disk_full(e) else f"lane-gate could not run it: {type(e).__name__}: {e}"
        return f"FAIL: {reason}", 0.0, "", False, None, reason
    result = "pass" if code == 0 else f"FAIL (exit {code})"
    gap = "" if code == 0 else env_gap(step, output)
    if gap:
        result += f" [env-gap: {gap}]"
    if code != 0 and NO_SPACE.search(output):
        result += "; the disk is full (\"No space left on device\" in its output)"
    notes = "" if "--self-test" in step.run else leak_notes(output)  # a self-test prints notes on purpose
    if notes:
        result += f"; {notes}"
    return result, secs, count_tests(output), code == 0, log, output


LEAK_NOTE = re.compile(r"test-leak-check: note: (\d+) new (entries|plists) appeared")


def leak_notes(output):
    """What test-leak-check let pass because this Mac is shared, for the Result column.

    It cannot say whose they are: a sibling lane's Swift tests leave the same
    UUID-named folders. So the count is shown rather than dropped, and
    --strict-leaks (on a quiet Mac) turns it back into a failure."""
    found = LEAK_NOTE.findall(output)
    if not found:
        return ""
    total = sum(int(n) for n, _ in found)
    return f"leak note: {total} unattributed new temp/plist entries (siblings or this lane; see the log, or rerun with --strict-leaks)"


def base_env(repo, tmp, ctx, strict_leaks=False):
    env = dict(os.environ)
    # CI=true is for the tests' sake; the Mac is still shared with sibling lanes,
    # whose UUID-named leftovers land in the same system temp folder as this
    # lane's. test-leak-check makes those a note, shown in the Result column,
    # unless the lane asks for strict (ov-802).
    env.pop("FC_LEAK_STRICT", None)
    env["FC_LEAK_SHARED"] = "1"
    if strict_leaks:
        env["FC_LEAK_STRICT"] = "1"
    env["PATH"] = os.path.expanduser("~/.cargo/bin") + os.pathsep + env.get("PATH", "")
    env.update({"RUNNER_TEMP": str(tmp), "RUNNER_OS": "macOS" if sys.platform == "darwin" else "Linux",
                "GITHUB_SHA": ctx["github.sha"], "GITHUB_WORKSPACE": str(repo), "CI": "true"})
    return env


def make_ctx(repo, tmp, base):
    sha = subprocess.run(["git", "-C", str(repo), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    jdk = java_home(21)
    return {"runner.temp": str(tmp), "github.sha": sha, "matrix.os": "local", "base": base,
            "steps.jdk21.outputs.path": jdk, "runner.os": "macOS", "runner.arch": "ARM64"}


def md(cell):
    return cell.replace("|", "\\|").replace("`", "'")


def main(argv):
    ap = argparse.ArgumentParser(description="Run the CI jobs a change reaches.")
    ap.add_argument("--base", default="origin/main", help="what the branch is measured against")
    ap.add_argument("--worktree", default=str(ROOT), help="the checkout to gate (default: this one)")
    ap.add_argument("--dry-run", action="store_true", help="print the plan; run nothing")
    ap.add_argument("--only", default="", help="comma-separated jobs to run (default: every job the diff reaches)")
    ap.add_argument("--skip", default="", help="comma-separated jobs to leave out")
    ap.add_argument("--log-dir", default="", help="where each step's full output goes (default: a new temp dir)")
    ap.add_argument("--no-load-wait", action="store_true", help="do not wait for the load average to fall")
    ap.add_argument("--release-between-jobs", action="store_true", help="queue again for each heavy job instead of holding the lock for the whole gate")
    ap.add_argument("--no-lock", action="store_true", help="skip the machine-wide heavy-job lock (it is off already when CI is set)")
    ap.add_argument("--strict-leaks", action="store_true", help="a new system temp entry or plist fails the run, even though sibling lanes share this Mac")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    hl.survive_dead_output()  # a waiting gate outlives the terminal or pipe that started it (ov-839)
    try:
        return gate(args)
    except OSError as e:
        if not hl.disk_full(e):
            raise
        print(f"lane-gate: FAIL: {DISK_FULL}\n\nlane-gate: FAILED", flush=True)
        return 1


def gate(args):
    repo = pathlib.Path(args.worktree).resolve()
    workflows = {"ci": (repo / ".github/workflows/ci.yml").read_text(),
                 "ios-ui-build": (repo / ".github/actions/ios-ui-build/action.yml").read_text()}
    tmp = pathlib.Path(tempfile.mkdtemp(prefix="lane-gate-"))  # ENOSPC here is caught by main
    ctx = make_ctx(repo, tmp, args.base)
    changes = load_module("ci_changes", repo / "scripts" / "ci-changes.py")
    ios_plan = load_module("ios_ui_plan", repo / "scripts" / "ios-ui-plan.py")
    files = changed_files(repo, args.base)

    plans = {j: steps_for(j, workflows, ctx) for j in JOBS}
    commands = {j: "\n".join(s.run for s in plans[j] if not s.skip) for j in JOBS}
    reach = jobs_for(files, changes.SITE, lambda f: bool(ios_plan.RELEVANT.match(f)), commands)
    only = {j for j in args.only.split(",") if j}
    skip = {j for j in args.skip.split(",") if j}
    for j in only | skip:
        if j not in JOBS:
            raise SystemExit(f"lane-gate: no job {j}; jobs are {', '.join(JOBS)}")
    chosen = [j for j in JOBS if j in reach and (not only or j in only) and j not in skip]

    # --only (or --skip) leaves out jobs the diff reaches. Say so here, and again in the summary, which is
    # what gets pasted into a report: a summary that reads "all steps passed" must not stand for a gate it skipped.
    left_out = [j for j in JOBS if j in reach and j not in chosen]
    partial = bool(left_out) and bool(only or skip)
    if partial:
        print(f"lane-gate: PARTIAL GATE: --only/--skip leaves out {', '.join(left_out)}, which this diff reaches")
    print(f"lane-gate: {len(files)} changed file(s) against {args.base}; jobs: {', '.join(chosen) or 'none'}")
    for j in chosen:
        why = reach[j][:3]
        more = f" and {len(reach[j]) - 3} more" if len(reach[j]) > 3 else ""
        print(f"  {j}: {', '.join(why) if why else 'always'}{more}")
    if args.dry_run:
        for j in chosen:
            print(f"\n[{j}]")
            for s in plans[j]:
                where = f" (in {s.cwd})" if s.cwd else ""
                print(f"  {'SKIP' if s.skip else 'run '} {s.label()}{where}" + (f"  -- {s.skip}" if s.skip else ""))
        return 0

    logdir = pathlib.Path(args.log_dir) if args.log_dir else pathlib.Path(tempfile.mkdtemp(prefix="lane-gate-logs-"))
    logdir.mkdir(parents=True, exist_ok=True)
    rows, failed = [], True  # until run_jobs says otherwise: an exception is a failure
    try:
        failed = run_jobs(chosen, plans, rows, repo, tmp, ctx, logdir, args)
    except BaseException as e:  # an interrupt or a bug still leaves a table of what ran
        full = hl.disk_full(e)  # a full disk is a named row and a red gate, not a traceback
        rows.append(("lane-gate", "(the gate itself)", f"FAIL: {DISK_FULL if full else f'{type(e).__name__}: {e}'}", "", ""))
        failed = True
        if not full:
            raise
    finally:
        try:
            print_summary(rows, repo, args.base, logdir, failed, left_out if partial else [], summary_path(repo))
        except OSError as e:  # the table is on screen; only the saved copy needs room
            print(f"lane-gate: could not save the summary file: {e.strerror or e}", flush=True)
        shutil.rmtree(tmp, ignore_errors=True)
    return 1 if failed else 0


def run_jobs(chosen, plans, rows, repo, tmp, ctx, logdir, args):
    """Run every chosen job's steps, appending a row per step to `rows`. True if any failed."""
    failed = False
    heavy = [j for j in chosen if j in HEAVY] and hl.lock_wanted(getattr(args, "no_lock", False))
    between = getattr(args, "release_between_jobs", False)
    # By default one place in line covers all of this gate's heavy jobs, so gates finish one after
    # another, oldest first; releasing between jobs made nine gates each take nine times as long (ov-836).
    with (machine_lock(repo, "gate: " + ", ".join(j for j in chosen if j in HEAVY)) if heavy and not between else contextlib.nullcontext()):
        for j in chosen:
            locked = bool(heavy) and between and j in HEAVY
            with (machine_lock(repo, j) if locked else contextlib.nullcontext()):
                failed = run_one_job(j, plans, rows, repo, tmp, ctx, logdir, args) or failed
    if "ios" in chosen:
        rows.append(("ios", "iOS UI smoke", "not run here: use scripts/ios-ui-tests.sh", "", ""))
    return failed


def run_one_job(j, plans, rows, repo, tmp, ctx, logdir, args):
    failed = False
    if j in HEAVY and not args.no_load_wait:
        wait_for_load()
    env = base_env(repo, tmp, ctx, args.strict_leaks)
    env.update(job_env(j))
    if j == "android" and ctx["steps.jdk21.outputs.path"]:
        env["CAPTURE_JDK"] = ctx["steps.jdk21.outputs.path"]
    print(f"\n[{j}]", flush=True)
    for s in plans[j]:
        if s.skip:
            rows.append((j, s.label(), f"skipped: {s.skip}", "", ""))
            print(f"  skip {s.label()}  ({s.skip})")
            continue
        print(f"  run  {s.label()}", flush=True)
        # `env` itself, not a copy: what a step writes to $GITHUB_ENV (the
        # swift job's APP_NAME) is read back into it for the steps after
        # (ov-812). The step's own `env:` is laid over it inside run_step.
        result, secs, counts, ok, log, output = run_step_safely(s, repo, env, logdir, tmp)
        rows.append((j, s.label(), result, f"{secs:.0f}", counts))
        if not ok:
            failed = True
            if log is None:
                print(f"  FAILED before it ran: {output}")
            else:
                tail = "\n".join(output.splitlines()[-15:])
                print(f"  FAILED, log {log}\n" + "\n".join("    " + l for l in tail.splitlines()))
    return failed


SUMMARY_NAME = "lane-gate-summary.txt"
FAIL_GAP = re.compile(r"\[env-gap: ([a-z0-9-]+)\]")


def summary_path(repo):
    """Where `repo`'s last gate summary lives: inside its own git dir, so each worktree has one (ov-822)."""
    out = subprocess.run(["git", "-C", str(repo), "rev-parse", "--absolute-git-dir"], capture_output=True, text=True).stdout.strip()
    return pathlib.Path(out) / SUMMARY_NAME if out else None


def print_summary(rows, repo, base, logdir, failed, left_out=(), write_to=None):
    """Print the summary table and, given `write_to`, save it there with the full HEAD sha it gated.

    The file opens with `key: value` lines scripts/gate-summary-check.py reads: head, verdict (ok, partial or
    failed; partial wins over failed) and a `failed-step:` line per failed row with the env gap it matched."""
    sha = subprocess.run(["git", "-C", str(repo), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    verdict = "partial" if left_out else "failed" if failed else "ok"
    lines = ["## lane-gate summary" + (" (PARTIAL)" if left_out else ""), ""]
    lines += [f"Branch HEAD {sha[:9]} against {base}; logs in {logdir}", ""]
    lines += ["| Job | Command | Result | Secs | Tests |", "| --- | --- | --- | --- | --- |"]
    lines += ["| " + " | ".join(md(c) for c in r) + " |" for r in rows]
    note = f" (PARTIAL, NOT A FULL GATE: {', '.join(left_out)} not run by --only/--skip)" if left_out else ""
    if failed:
        last = "lane-gate: FAILED" + note
    elif left_out:
        last = f"lane-gate: PARTIAL, NOT A FULL GATE: {', '.join(left_out)} not run (--only/--skip); the steps that ran passed"
    else:
        last = "lane-gate: all steps passed"
    print("\n".join(lines) + "\n\n" + last, flush=True)
    if write_to:
        head = ["lane-gate-summary v1", f"head: {sha}", f"verdict: {verdict}"]
        for r in rows:
            if str(r[2]).startswith("FAIL"):
                m = FAIL_GAP.search(str(r[2]))
                head.append(f"failed-step: {r[0]} | {md(r[1])} | gap={m.group(1) if m else 'none'}")
        pathlib.Path(write_to).write_text("\n".join(head) + "\n\n" + "\n".join(lines) + "\n\n" + last + "\n")


# ---------------------------------------------------------------- self-test


def self_test():
    global run_jobs
    import contextlib
    import io
    failures = []

    def expect(what, got, want):
        if got != want:
            failures.append(f"{what}: got {got!r}, want {want!r}")

    text = (ROOT / ".github/workflows/ci.yml").read_text()
    changes = load_module("ci_changes", ROOT / "scripts" / "ci-changes.py")
    ios_plan = load_module("ios_ui_plan", ROOT / "scripts" / "ios-ui-plan.py")
    ios_rel = lambda f: bool(ios_plan.RELEVANT.match(f))
    with tempfile.TemporaryDirectory() as t:
        ctx = make_ctx(ROOT, pathlib.Path(t), "origin/main")
    ctx["steps.jdk21.outputs.path"] = ctx["steps.jdk21.outputs.path"] or "/jdk21"
    wf = {"ci": text, "ios-ui-build": (ROOT / ".github/actions/ios-ui-build/action.yml").read_text()}

    # A job added to ci.yml must be added here, or the script quietly under-gates.
    for job in job_ids(text):
        known = job in CI_JOB_ALIASES or job in CI_JOBS_NOT_RUN
        expect(f"ci.yml job {job} is known to lane-gate", known, True)
    for job in set(CI_JOB_ALIASES) | set(CI_JOBS_NOT_RUN):
        expect(f"lane-gate's {job} is still in ci.yml", job in job_ids(text), True)

    plans = {j: steps_for(j, wf, ctx) for j in JOBS}
    for j in JOBS:
        expect(f"{j} has steps", len([s for s in plans[j] if not s.skip]) > 0, True)

    def has(job, needle):
        return any(needle in s.run and not s.skip for s in plans[job])

    # The wrappers and the full, unfiltered suites, as CI runs them.
    for job, needle in [
        ("wire", "./scripts/proto-lint.py --channel canary"),
        ("wire", "./scripts/copy-lint.py"),
        ("wire", "./scripts/test-leak-check.py --lint"),
        ("rust", "./scripts/test-leak-check.py -- ./scripts/tmux-leak-check.py -- cargo nextest run --profile ci --workspace"),
        ("rust", "cargo clippy --workspace --all-targets -- -D warnings"),
        ("rust", "cargo test --workspace --doc"),
        ("swift-tests", "./scripts/swift-test-guard.sh apps/shared/AgentKit/test-floor.txt -- ./scripts/test-leak-check.py -- swift test --package-path apps/shared/AgentKit"),
        ("swift-tests", "./apps/macos/test.sh"),
        ("swift", "apps/macos/build-app.sh"),
        ("android", "./gradlew testInstrumentedUnitTest"),
        ("android", "-Pfarcooler.captures"),
        ("relay", "npm test"),
        ("site", "npm test"),
        ("web", "npm test"),
        ("tunnel", "go test -count=1 ./..."),
        ("ios", "./scripts/generator-test.sh"),
        ("ios", "./scripts/ios-ui-shards.py --check"),
        ("doc-comments", "--base origin/main --head HEAD"),
    ]:
        expect(f"{job} runs {needle}", has(job, needle), True)
    # Nothing that only installs on a CI runner is run, and the skip is recorded.
    for j in JOBS:
        for s in plans[j]:
            if not s.skip:
                # Spelled out here, not read from SETUP, so SETUP losing one goes red.
                for word in ("apt-get", "brew install", "sdkmanager", "ci-install-tmux", "sshd -V"):
                    expect(f"{j}: {s.label()} does not run {word}", word in s.run, False)
    expect("the tmux install is skipped, not dropped", any(s.skip and "ci-install-tmux" in s.run for s in plans["rust"]), True)
    expect("the unfiltered Mac test is not skipped", any(s.run.strip() == "./apps/macos/test.sh" and not s.skip for s in plans["swift-tests"]), True)
    expect("the tunnel steps run in crates/tailcat/go", {s.cwd for s in plans["tunnel"]}, {"crates/tailcat/go"})
    expect("the Android gradle steps run in apps/android", all(s.cwd == "apps/android" for s in plans["android"] if "gradlew" in s.run), True)
    expect("a ${{ }} is filled in", any("${{" in s.run for s in plans["web"]), False)
    expect("Android's JDK 21 reaches the capture step", any(s.env.get("CAPTURE_JDK") == ctx["steps.jdk21.outputs.path"] for s in plans["android"]), True)
    expect("the web job leaves the Linux downloads to CI", any(s.skip and "curl" in s.run for s in plans["web"]), True)

    # The iOS job compiles what CI's does (ov-812): the generated project's
    # FarCooler scheme, build-for-testing, after the frameworks and the project.
    ios_cmds = [s.run for s in plans["ios"] if not s.skip]
    build = [i for i, r in enumerate(ios_cmds) if r.startswith("xcodebuild -project apps/ios/FarCooler.xcodeproj")]
    expect("ios runs one xcodebuild of the project", len(build), 1)
    if build:
        expect("ios builds the FarCooler scheme for testing, as the action does", all(w in ios_cmds[build[0]] for w in ("-scheme FarCooler", "build-for-testing", "ARCHS=arm64", "generic/platform=iOS Simulator")), True)
        expect("... after the frameworks and the generated project", [i for i, r in enumerate(ios_cmds) if "build-ios-frameworks.sh" in r or "generate-project.py" in r], [build[0] - 2, build[0] - 1])
    expect("ios leaves the watchOS download and the packaging to CI", sum(1 for s in plans["ios"] if s.skip and ("downloadPlatform" in s.run or "ios-ui-products" in s.run)) >= 1, True)

    # What a step writes to $GITHUB_ENV reaches the steps after it, through run_jobs
    # as well as run_step: the swift job's APP_NAME was lost between them (ov-812).
    with tempfile.TemporaryDirectory() as t:
        t = pathlib.Path(t)
        chain = [Step("x", "set", 'echo "APP_NAME=Far Cooler Test" >> "$GITHUB_ENV"\n'),
                 Step("x", "read", '[ "$APP_NAME" = "Far Cooler Test" ]\n'),
                 Step("x", "unset", '[ -z "${APP_NAME_NEVER_SET:-}" ] && set -u && : "$APP_NAME_NEVER_SET"\n')]
        rows = []
        with contextlib.redirect_stdout(io.StringIO()):
            failed = run_jobs(["x"], {"x": chain}, rows, t, t, ctx, t, argparse.Namespace(no_load_wait=True, strict_leaks=False))
        expect("a step reads what an earlier step put in GITHUB_ENV", [r[2] for r in rows[:2]], ["pass", "pass"])
        expect("... and an unset variable still fails a step", rows[2][2].startswith("FAIL"), True)
    swift_run = [s for s in plans["swift"] if not s.skip]
    names = [i for i, s in enumerate(swift_run) if 'GITHUB_ENV' in s.run and "APP_NAME=" in s.run]
    users = [i for i, s in enumerate(swift_run) if "$APP_NAME" in s.run]
    expect("the swift job sets APP_NAME before any step uses it", bool(names and users and names[0] < users[0]), True)

    # One heavy job at a time on the machine (ov-831); the lock itself is tested in scripts/heavy_lock.py (ov-836).
    import threading
    with tempfile.TemporaryDirectory() as t:
        t = pathlib.Path(t)
        lock = t / "lane-gate.lock"

        # Through run_jobs: two fake heavy jobs at once do not overlap, but a light job does not wait,
        # and the lock is let go between a gate's jobs so the other gate's job slips in.
        marks = t / "marks"
        def step(name):
            return Step("x", name, f'echo "{name} start" >> "{marks}"; sleep 0.5; echo "{name} end" >> "{marks}"\n')
        plans2 = {"h1": [step("g1h1")], "h2": [step("g1h2")], "g2h": [step("g2h")], "light": [step("g2light")]}
        saved_heavy, saved_ci = set(HEAVY), os.environ.pop("CI", None)
        HEAVY.update({"h1", "h2", "g2h"})
        saved_poll = LOCK_POLL
        globals()["LOCK_POLL"] = 0.05
        orig_lock = machine_lock
        globals()["machine_lock"] = lambda wt, j, **k: orig_lock(wt, j, path=lock, poll=LOCK_POLL, say=lambda *a, **kw: None, **k)
        try:
            ns = argparse.Namespace(no_load_wait=True, strict_leaks=False, no_lock=False)
            with contextlib.redirect_stdout(io.StringIO()):
                t1 = threading.Thread(target=run_jobs, args=(["h1", "h2"], plans2, [], t, t, ctx, t, ns))
                t1.start()
                time.sleep(0.2)
                t2 = threading.Thread(target=run_jobs, args=(["g2h"], plans2, [], t, t, ctx, t, ns))
                t2.start()
                t0 = time.time()
                run_jobs(["light"], plans2, [], t, t, ctx, t, ns)
                light_secs = time.time() - t0
                t1.join(10)
                t2.join(10)
            expect("a light job does not wait for the lock", light_secs < 0.8, True)
            seq = marks.read_text().split("\n")
            spans, open_ = [], []
            for line in filter(None, seq):
                n, what = line.split()
                open_.append(n) if what == "start" else open_.remove(n)
                spans.append(len([x for x in open_ if x != "g2light"]))
            expect("heavy jobs never overlap", max(spans), 1)
            expect("a gate holds the lock for all its heavy jobs, so the other gate waits for the whole gate",
                   [l.split()[0] for l in seq if l.endswith("start") and not l.startswith("g2light")], ["g1h1", "g1h2", "g2h"])
            ns.release_between_jobs = True
            marks.write_text("")
            with contextlib.redirect_stdout(io.StringIO()):
                t1 = threading.Thread(target=run_jobs, args=(["h1", "h2"], plans2, [], t, t, ctx, t, ns))
                t1.start()
                time.sleep(0.2)
                t2 = threading.Thread(target=run_jobs, args=(["g2h"], plans2, [], t, t, ctx, t, ns))
                t2.start()
                t1.join(10)
                t2.join(10)
            expect("--release-between-jobs queues again for each job, so the other gate slips in",
                   [l.split()[0] for l in marks.read_text().split("\n") if l.endswith("start")], ["g1h1", "g2h", "g1h2"])
            ns.release_between_jobs = False
            ns.no_lock = True
            marks.write_text("")
            with contextlib.redirect_stdout(io.StringIO()):
                t1 = threading.Thread(target=run_jobs, args=(["h1"], plans2, [], t, t, ctx, t, ns))
                t1.start()
                time.sleep(0.2)
                run_jobs(["g2h"], plans2, [], t, t, ctx, t, ns)
                t1.join(10)
            expect("--no-lock lets them overlap", marks.read_text().split()[:4], ["g1h1", "start", "g2h", "start"])
            ns.no_lock = False
            os.environ["CI"] = "true"
            expect("CI turns the lock off by default", hl.lock_wanted(ns.no_lock), False)
            os.environ.pop("CI")
            expect("... and without CI or --no-lock it is on", hl.lock_wanted(ns.no_lock), True)
        finally:
            HEAVY.clear(); HEAVY.update(saved_heavy)
            globals()["LOCK_POLL"] = saved_poll
            globals()["machine_lock"] = orig_lock
            if saved_ci is not None:
                os.environ["CI"] = saved_ci

    # The parser, on the shapes ci.yml uses, so a mistake cannot hide in ci.yml.
    sample = (
        "jobs:\n  a:\n    defaults:\n      run:\n        working-directory: sub\n    env:\n      K: v\n    steps:\n"
        "      - uses: x/y@v1\n        with:\n          run: not-a-step\n"
        "      - run: one\n"
        "      - name: Two\n        working-directory: other\n        env:\n          E: ${{ runner.temp }}\n        run: |\n          two\n          three\n"
        "  b:\n    steps:\n      - run: other\n"
    )
    got = parse_steps(job_block(sample, "a"))
    expect("parse: steps with a run", [(n, r) for n, r, _, _ in got], [("", "one\n"), ("Two", "two\nthree\n")])
    expect("parse: the default and the step directory", [w for _, _, w, _ in got], ["sub", "other"])
    expect("parse: env, job and step", [e for _, _, _, e in got], [{"K": "v"}, {"K": "v", "E": "${{ runner.temp }}"}])
    expect("parse: a job stops at the next one", job_block(sample, "b"), ["    steps:", "      - run: other"])
    try:
        expand("${{ secrets.X }}", ctx)
        failures.append("an unknown ${{ }} was not refused")
    except KeyError:
        pass

    # Which jobs a path reaches. Every rule has a sample that only it explains.
    commands = {j: "\n".join(s.run for s in plans[j] if not s.skip) for j in JOBS}

    def reaches(*paths):
        return set(jobs_for(list(paths), changes.SITE, ios_rel, commands))

    always = {"wire", "doc-comments"}
    expect("docs only: the lints alone", reaches("docs/releasing.md", "README.md"), always)
    expect("a site-proof file: the lints alone", reaches("scripts/site-proof.py"), always)
    expect("a script nothing names: the lints alone", reaches("scripts/nonexistent-lane-script.sh"), always)
    expect("a script a job names reaches that job", "swift-tests" in reaches("scripts/swift-test-guard.sh"), True)
    expect("... and not the others", "android" in reaches("scripts/swift-test-guard.sh"), False)
    expect("the nextest wrapper reaches rust", "rust" in reaches("scripts/tmux-leak-check.py"), True)
    expect("relay", reaches("services/relay/src/index.ts"), always | {"relay"})
    expect("site", reaches("services/site/src/index.ts"), always | {"site"})
    expect("common reaches web and site", reaches("services/common/session.ts"), always | {"web", "site"})
    expect("web", reaches("services/web/src/a.ts"), always | {"web", "site"})
    expect("android", reaches("apps/android/app/src/A.kt"), always | {"android"})
    expect("ios", reaches("apps/ios/FarCooler/App.swift"), always | {"ios"})
    expect("macos", reaches("apps/macos/Sources/x.swift"), always | {"swift", "swift-tests"})
    expect("agentkit reaches the iOS app too", reaches("apps/shared/AgentKit/Sources/a.swift"), always | {"swift", "swift-tests", "ios"})
    expect("tailcat reaches the tunnel", "tunnel" in reaches("crates/tailcat/go/a.go"), True)
    expect("a daemon change reaches every app but not the tunnel or iOS",
           reaches("crates/daemon/src/git.rs"), always | {"rust", "swift", "swift-tests", "android"})
    expect("core reaches iOS", "ios" in reaches("crates/core/src/lib.rs"), True)
    expect("the lockfile reaches every app job", reaches("Cargo.lock") >= always | APP_JOBS - {"tunnel"}, True)
    expect("ci.yml reaches every job", reaches(".github/workflows/ci.yml"), set(JOBS))
    expect("another workflow: the lints alone", reaches(".github/workflows/canary.yml"), always)
    expect("an iOS action", reaches(".github/actions/ios-ui-shard/action.yml"), always | {"ios"})
    expect("an unknown path takes the safe side", reaches("mystery/file.txt") >= always | APP_JOBS, True)

    # The diff: commits, uncommitted edits and untracked files all count.
    with tempfile.TemporaryDirectory() as t:
        repo = pathlib.Path(t)

        def g(*a):
            r = subprocess.run(["git", "-C", t, "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", *a], capture_output=True, text=True)
            if r.returncode:
                raise RuntimeError(r.stderr)
            return r.stdout.strip()

        g("init", "-q", "-b", "main")
        (repo / "a.txt").write_text("a\n")
        (repo / "b.txt").write_text("b\n")
        g("add", "-A")
        g("commit", "-qm", "base")
        g("checkout", "-qb", "lane")
        (repo / "c.txt").write_text("c\n")
        g("add", "-A")
        g("commit", "-qm", "c")
        (repo / "a.txt").write_text("changed\n")
        (repo / "new.txt").write_text("n\n")
        g("mv", "b.txt", "moved.txt")
        expect("changed_files", changed_files(repo, "main"), ["a.txt", "b.txt", "c.txt", "moved.txt", "new.txt"])

    expect("a nextest summary is counted", count_tests("     Summary [  12.3s] 1234 tests run: 1230 passed, 4 skipped"), "1230 nextest")
    expect("an XCTest total is counted", count_tests("Executed 17 tests, with 0 failures (0 unexpected) in 1.0 (1.1) seconds"), "17 xctest")
    expect("go packages are counted, a bare 'ok' line is not", count_tests("ok  \tpkg/a\t0.3s\nok  \tpkg/b\t(cached)\nok  \tpkg/c\t1.2s\nok something\n"), "2 go pkgs")
    expect("vitest is counted", count_tests(" Tests  42 passed (42)"), "42 vitest")

    # A host missing a tool, a directory or a JDK is a named FAIL row, not a crash (ov-802).
    with tempfile.TemporaryDirectory() as t:
        t = pathlib.Path(t)

        def fake_jdk(name, version):
            (t / name).mkdir()
            (t / name / "release").write_text(f'JAVA_VERSION="{version}"\n')
            return str(t / name)

        jdk11, jdk17, jdk21 = fake_jdk("j11", "11.0.12"), fake_jdk("j17", "17.0.9"), fake_jdk("j21", "21.0.1")
        jdk8 = fake_jdk("j8", "1.8.0_292")
        expect("jdk_major reads a release file", (jdk_major(jdk11), jdk_major(jdk8), jdk_major(str(t / "none"))), (11, 8, 0))
        expect("JDK 17 is picked by its own version", java_home(17, [jdk11, jdk17, jdk21]), jdk17)
        expect("only JDK 11 installed: no JDK 17 (java_home -v 17 said 11)", java_home(17, [jdk11]), "")
        expect("a newer JDK stands in when there is no exact one", java_home(17, [jdk11, jdk21]), jdk21)
        bindir = t / "bin"
        bindir.mkdir()
        empty = str(t / "empty")
        (t / "empty").mkdir()
        for tool in ("cargo-nextest", "cargo"):
            (bindir / tool).write_text("#!/bin/sh\n")
            (bindir / tool).chmod(0o755)
        nextest = Step("rust", "n", "cargo nextest run --profile ci --workspace\n")
        expect("nextest missing is named, with the install command",
               "cargo-nextest is not installed" in blocker(nextest, t, {"PATH": empty}) and "cargo install cargo-nextest --locked --version" in blocker(nextest, t, {"PATH": empty}), True)
        expect("nextest present: nothing blocks", blocker(nextest, t, {"PATH": str(bindir)}), "")
        gradle = Step("android", "g", "./gradlew assembleDebug\n")
        expect("gradle on JDK 11 is named", "JDK 17" in blocker(gradle, t, {"PATH": empty, "JAVA_HOME": jdk11}), True)
        expect("gradle on JDK 17 is fine", blocker(gradle, t, {"PATH": empty, "JAVA_HOME": jdk17}), "")
        capture = Step("android", "c", "./gradlew test \\\n  -Pfarcooler.captureJdk=$CAPTURE_JDK\n")
        expect("the capture task on a JDK 17 is named", "JDK 21" in blocker(capture, t, {"PATH": empty, "JAVA_HOME": jdk17, "CAPTURE_JDK": jdk17}), True)
        expect("the capture task on a JDK 21 is fine", blocker(capture, t, {"PATH": empty, "JAVA_HOME": jdk17, "CAPTURE_JDK": jdk21}), "")
        goes = Step("tunnel", "go", "go test -count=1 ./...\n")
        expect("go missing is named", "go is not installed" in blocker(goes, t, {"PATH": empty}), True)
        expect("go in prose is not a command", blocker(Step("x", "y", "echo to go on\n"), t, {"PATH": empty}), "")

        logs = t / "logs"
        logs.mkdir()
        gone = Step("android", "r", "true\n", cwd="apps/android/app/build/test-results/missing")
        row = run_step_safely(gone, t, {"PATH": "/usr/bin:/bin"}, logs, t)
        expect("a missing working directory is a FAIL row, not a FileNotFoundError", (row[0].startswith("FAIL: working directory"), row[3]), (True, False))
        row = run_step_safely(Step("x", "ok", "true\n"), t, {"PATH": "/usr/bin:/bin"}, logs, t)
        expect("a step that passes still passes", (row[0], row[3]), ("pass", True))
        noisy = "echo 'test-leak-check: note: 2 new entries appeared in /x' >&2\n"
        expect("a leak note shows in a real run's Result", "leak note" in run_step_safely(Step("x", "n", noisy), t, {"PATH": "/usr/bin:/bin"}, logs, t)[0], True)
        expect("... and not in a self-test's", "leak note" in run_step_safely(Step("x", "n", noisy + "# --self-test\n"), t, {"PATH": "/usr/bin:/bin"}, logs, t)[0], False)
        row = run_step_safely(Step("x", "bad", "exit 3\n"), t, {"PATH": "/usr/bin:/bin"}, logs, t)
        expect("a step that fails says its exit code", row[0], "FAIL (exit 3)")
        global run_step
        real_run_step = run_step
        run_step = lambda *a: (_ for _ in ()).throw(OSError("boom"))
        try:
            row = run_step_safely(Step("x", "boom", "true\n"), t, {"PATH": "/usr/bin:/bin"}, logs, t)
        finally:
            run_step = real_run_step
        expect("an exception inside a step is that step's FAIL row", (row[0], row[3]), ("FAIL: lane-gate could not run it: OSError: boom", False))

    expect("a leak note is carried into the Result column", "leak note: 3" in leak_notes("test-leak-check: note: 3 new entries appeared in /x"), True)
    expect("no note, no addition", leak_notes("test-leak-check: the run left nothing"), "")
    env = base_env(ROOT, pathlib.Path("/tmp"), ctx)
    expect("the gate marks the Mac shared, so siblings' leftovers are a note", (env["FC_LEAK_SHARED"], env["CI"], "FC_LEAK_STRICT" in env), ("1", "true", False))
    expect("--strict-leaks turns strict back on", base_env(ROOT, pathlib.Path("/tmp"), ctx, True).get("FC_LEAK_STRICT"), "1")

    # The table prints even when running the jobs raises (the ov-800 crash printed none).
    real_run_jobs = run_jobs
    def boom(chosen, plans, rows, *rest):
        rows.append(("wire", "a step that ran", "pass", "1", ""))
        raise RuntimeError("the gate tripped")
    run_jobs = boom
    out = io.StringIO()
    try:
        with contextlib.redirect_stdout(out):
            main(["--worktree", str(ROOT), "--base", "HEAD", "--only", "wire", "--no-load-wait"])
        failures.append("an exception in the run was swallowed")
    except RuntimeError:
        pass
    finally:
        run_jobs = real_run_jobs
    text_out = out.getvalue()
    expect("the summary table prints after an exception", ("## lane-gate summary" in text_out, "a step that ran" in text_out, "FAIL: RuntimeError: the gate tripped" in text_out, "lane-gate: FAILED" in text_out), (True, True, True, True))

    # A full disk is a named FAIL row and a red gate, never a traceback (ov-839).
    import errno
    full = OSError(errno.ENOSPC, "No space left on device")
    def full_run(*a, **k):
        raise full
    real_run_step, real_mkdtemp = run_step, tempfile.mkdtemp
    run_step = full_run
    try:
        res = run_step_safely(Step("rust", "n", "true\n"), ROOT, {"PATH": os.environ["PATH"]}, pathlib.Path("/tmp"), pathlib.Path("/tmp"))
    finally:
        run_step = real_run_step
    expect("a step that cannot write for want of room is a named FAIL row", (res[0].startswith("FAIL: the disk is full"), res[3]), (True, False))
    with tempfile.TemporaryDirectory() as t:
        res = run_step_safely(Step("rust", "n", "echo 'error: No space left on device (os error 28)'; exit 101\n"), ROOT, {"PATH": os.environ["PATH"]}, pathlib.Path(t), pathlib.Path(t))
    expect("a step that ran out of room says so beside its exit code", (res[0].startswith("FAIL (exit 101); the disk is full"), res[3]), (True, False))
    for where in ("run_jobs", "mkdtemp"):
        out = io.StringIO()
        if where == "run_jobs":
            run_jobs = full_run
        else:
            tempfile.mkdtemp = full_run
        try:
            with contextlib.redirect_stdout(out):
                rc = main(["--worktree", str(ROOT), "--base", "HEAD", "--only", "wire", "--no-load-wait"])
        except OSError as e:
            rc = f"raised {e!r}"
        finally:
            run_jobs, tempfile.mkdtemp = real_run_jobs, real_mkdtemp
        expect(f"a full disk in {where} gives a named FAIL and exit 1, not a crash", (rc, "FAIL: the disk is full" in out.getvalue(), "lane-gate: FAILED" in out.getvalue()), (1, True, True))
    asked = []
    real_survive = hl.survive_dead_output
    hl.survive_dead_output = lambda: asked.append(1)
    try:
        with contextlib.redirect_stdout(io.StringIO()):
            main(["--worktree", str(ROOT), "--base", "HEAD", "--dry-run"])
    finally:
        hl.survive_dead_output = real_survive
    expect("the gate protects its own output before it queues", asked, [1])

    # A filtered gate says so in its summary heading and its last line, and an unfiltered one does not (ov-817).
    for left, failed_, want_head, want_last in [
        (["relay", "wire"], False, "## lane-gate summary (PARTIAL)", "lane-gate: PARTIAL, NOT A FULL GATE: relay, wire not run"),
        ([], False, "## lane-gate summary\n", "lane-gate: all steps passed"),
    ]:
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            print_summary([("wire", "x", "pass", "1", "")], ROOT, "HEAD", "/tmp", failed_, left)
        o = buf.getvalue()
        expect(f"summary with {len(left)} job(s) left out", (want_head in o, want_last in o, ("PARTIAL" in o) == bool(left)), (True, True, True))

    # The summary file carries the full sha it gated, the verdict, and each failed step's env gap (ov-822).
    with tempfile.TemporaryDirectory() as t:
        dest = pathlib.Path(t) / SUMMARY_NAME
        rows = [("rust", "cargo nextest run", "FAIL (exit 101) [env-gap: git-lfs-missing]", "2", ""), ("web", "tsc", "FAIL (exit 2)", "1", "")]
        with contextlib.redirect_stdout(io.StringIO()):
            print_summary(rows, ROOT, "HEAD", "/tmp", True, [], dest)
        saved = dest.read_text()
        full = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        expect("the saved summary", (f"head: {full}\n" in saved, "verdict: failed\n" in saved, "failed-step: rust | cargo nextest run | gap=git-lfs-missing\n" in saved, "failed-step: web | tsc | gap=none\n" in saved), (True,) * 4)
        with contextlib.redirect_stdout(io.StringIO()):
            print_summary(rows[:0], ROOT, "HEAD", "/tmp", False, ["rust"], dest)
        expect("a partial run saves verdict partial", "verdict: partial\n" in dest.read_text(), True)
    gap_step = Step("rust", "n", "cargo nextest run --profile ci\n")
    expect("a git-lfs gap", env_gap(gap_step, "panicked: CI installs git-lfs, and it isn't on PATH"), "git-lfs-missing")
    expect("the same words from another command", env_gap(Step("wire", "n", "./scripts/x.sh\n"), "git-lfs isn't on PATH"), "")
    expect("a plain test failure", env_gap(gap_step, "assertion failed: left == right"), "")
    expect("a wrangler gap", env_gap(Step("web", "n", "npx wrangler deploy --dry-run --outdir /tmp/w\n"), "The directory specified by assets.directory does not exist: dist"), "wrangler-dry-run-without-dist")

    for failure in failures:
        print(f"lane-gate self-test: {failure}", file=sys.stderr)
    if failures:
        return 1
    print("lane-gate self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
