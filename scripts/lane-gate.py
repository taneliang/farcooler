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

Not run here, and said so in the output: the iOS build and UI smoke
(`.github/actions/ios-ui-*` build an archive and a simulator; use
scripts/ios-ui-tests.sh, one simulator at most) and the parts of the web job
that download Linux binaries. A heavy job waits while the 1-minute load average
is 20 or more.
"""

import argparse
import importlib.util
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import time

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
        for name, run, wd, env in parse_steps(block):
            skip = None
            hay = name + "\n" + run
            if SETUP.search(hay):
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


def load1():
    return os.getloadavg()[0]


def wait_for_load(limit=20.0, poll=30):
    started = time.time()
    announced = 0
    while load1() >= limit:
        if time.time() - started >= announced * 300:
            print(f"  waiting: load average {load1():.1f} is {limit:.0f} or more", flush=True)
            announced += 1
        time.sleep(poll)


COUNTS = [
    (re.compile(r"Summary \[.*?\]\s+(\d+) tests? run: (\d+) passed"), "nextest", 2),
    (re.compile(r"test result: ok\. (\d+) passed"), "doc/unit", 1),
    (re.compile(r"Test run with (\d+) tests?"), "swift-testing", 1),
    (re.compile(r"Tests\s+(\d+) passed"), "vitest", 1),
    (re.compile(r"UI tests that ran: (\d+) in"), "ui", 1),
    (re.compile(r"^ok\s+\S+", re.M), "go pkgs", 0),
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


def java_home(version):
    try:
        out = subprocess.run(["/usr/libexec/java_home", "-v", str(version)], capture_output=True, text=True)
        return out.stdout.strip() if out.returncode == 0 else ""
    except OSError:
        return ""


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


def base_env(repo, tmp, ctx):
    env = dict(os.environ)
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
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()

    repo = pathlib.Path(args.worktree).resolve()
    workflows = {"ci": (repo / ".github/workflows/ci.yml").read_text()}
    tmp = pathlib.Path(tempfile.mkdtemp(prefix="lane-gate-"))
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
    rows, failed = [], False
    for j in chosen:
        if j in HEAVY and not args.no_load_wait:
            wait_for_load()
        env = base_env(repo, tmp, ctx)
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
            code, secs, output, log = run_step(s, repo, env, logdir, tmp)
            counts = count_tests(output)
            result = "pass" if code == 0 else f"FAIL (exit {code})"
            rows.append((j, s.label(), result, f"{secs:.0f}", counts))
            if code != 0:
                failed = True
                tail = "\n".join(output.splitlines()[-15:])
                print(f"  FAILED, log {log}\n" + "\n".join("    " + l for l in tail.splitlines()))
                if j in ("rust", "swift", "swift-tests", "ios", "android", "web", "tunnel"):
                    pass  # keep going: a lane wants every red thing in one pass
    if "ios" in chosen:
        rows.append(("ios", "iOS build and UI smoke", "not run here: use scripts/ios-ui-tests.sh", "", ""))

    print("\n## lane-gate summary\n")
    print(f"Branch HEAD {subprocess.run(['git', '-C', str(repo), 'rev-parse', '--short', 'HEAD'], capture_output=True, text=True).stdout.strip()} against {args.base}; logs in {logdir}\n")
    print("| Job | Command | Result | Secs | Tests |")
    print("| --- | --- | --- | --- | --- |")
    for r in rows:
        print("| " + " | ".join(md(c) for c in r) + " |")
    shutil.rmtree(tmp, ignore_errors=True)
    print("\nlane-gate: FAILED" if failed else "\nlane-gate: all steps passed")
    return 1 if failed else 0


# ---------------------------------------------------------------- self-test


def self_test():
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
    wf = {"ci": text}

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
    expect("vitest is counted", count_tests(" Tests  42 passed (42)"), "42 vitest")

    for failure in failures:
        print(f"lane-gate self-test: {failure}", file=sys.stderr)
    if failures:
        return 1
    print("lane-gate self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
