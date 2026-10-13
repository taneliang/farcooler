#!/usr/bin/env python3
"""Does this push or pull request change anything but the website and docs? (ov-507)

ci.yml's `changes` job runs this, and the expensive jobs (Swift, Rust on macOS
and Linux, Android, iOS and the UI shards) wait on its `app` output:

    EVENT=push AFTER=<sha> CANDIDATES=<file> scripts/ci-changes.py
    EVENT=pull_request BASE=<sha> HEAD=<sha> scripts/ci-changes.py
    scripts/ci-changes.py --self-test

It writes `app=true|false` to $GITHUB_OUTPUT and says why. `false` only when
every changed file is on the SITE_ONLY allowlist below; a single file off it,
or anything this script cannot work out, is `true`: a skipped test is the one
failure nobody sees.

The comparison is the one scripts/ios-ui-plan.py makes. A pull request
compares merge-base..head. A push to main compares the nearest ancestor of
`AFTER` whose CI run on main was GREEN, not the push's own `before`: CI cancels
a run when the next push lands, and a run can end red, so the previous push
may never have had its tests pass. Judging by `before` would let a site-only
push go green on top of an unverified code change. By induction the rule is
sound: a green run that skipped the app jobs had no code change since its own
green base. With no green ancestor (a first push, a force push, a zero
`before`), a listing that failed, or a range that will not compute, the app
jobs run.

The allowlist is what no test, lint or build reads. The checks that read the
whole tree (the `wire` job's scripts, which also run the site-proof and this
script's self-tests) always run, so a change to a script is never skipped
because of where it sits: scripts/ is NOT on the list, bar the site-proof
refresher and its fixtures, which only the website consumes and whose own
self-test runs in `wire` regardless.
"""

import importlib.util
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent

# One alternative per entry, each matched against the whole repo-relative path.
# The self-test holds a sample for each and a list of code paths that must
# never match. No test reads docs/ or a root-level .md (checked for ov-507:
# the Rust and Swift tests that name a README.md write their own into a
# scratch repository).
SITE_ONLY = [
    r"services/site/.+",
    r"docs/.+",
    r"[^/]+\.md",
    r"\.github/workflows/site\.yml",
    r"\.github/workflows/site-analytics\.yml",
    r"scripts/site-proof-milestones\.json",
    r"scripts/site-proof-refresh\.sh",
    r"scripts/site-proof\.py",
    r"scripts/site-proof-fixtures/.+",
]


def pattern(names):
    return re.compile("^(?:" + "|".join(names) + ")$")


SITE = pattern(SITE_ONLY)


def canary_plan():
    spec = importlib.util.spec_from_file_location("canary_plan", ROOT / "scripts" / "canary-plan.py")
    module = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True
    spec.loader.exec_module(module)
    return module


def git(repo, *args):
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)


def decide(repo, event, after="", base="", head="", candidates=(), listing_error=""):
    """(app, why): whether the app jobs run."""
    if event == "push" and listing_error:
        return True, f"could not list the green CI runs ({listing_error}): the app jobs run."
    if event == "pull_request":
        spec = f"{base}...{head}"
    elif event == "push":
        green = canary_plan().base_commit(repo, after, list(candidates))
        if green is None:
            return True, "no earlier green CI run in this history to compare with: the app jobs run."
        spec = f"{green}..{after}"
    else:
        return True, f"{event or 'no event'}: the app jobs run."
    out = git(repo, "diff", "--name-only", spec)
    if out.returncode != 0:
        return True, f"could not compare {spec}: the app jobs run."
    files = [f for f in out.stdout.splitlines() if f]
    if not files:
        return True, f"no changed files in {spec}: the app jobs run."
    for f in files:
        if not SITE.match(f):
            return True, f"{f} is not website or docs: the app jobs run."
    return False, f"{len(files)} changed file(s) since {spec.split('..')[0][:8]}, all website or docs: the app jobs are skipped."


def main():
    error = os.environ.get("CANDIDATES_ERROR", "").strip()
    candidates = [] if error else canary_plan().read_candidates(os.environ.get("CANDIDATES"))
    app, why = decide(
        pathlib.Path.cwd(), os.environ.get("EVENT", ""), os.environ.get("AFTER", ""),
        os.environ.get("BASE", ""), os.environ.get("HEAD", ""),
        candidates, error,
    )
    print(f"::notice::{why}" if error and os.environ.get("EVENT") == "push" else why)
    out = os.environ.get("GITHUB_OUTPUT")
    if out:
        with open(out, "a") as f:
            f.write(f"app={'true' if app else 'false'}\n")
    return 0


def self_test():
    failures = []

    def expect(what, got, want):
        if got != want:
            failures.append(f"{what}: got {got!r}, want {want!r}")

    # Every alternative has a sample only it matches, so deleting one goes red.
    SAMPLES = [
        "services/site/src/index.ts",
        "docs/releasing.md",
        "docs/superpowers/specs/a/b.json",
        "README.md",
        ".github/workflows/site.yml",
        ".github/workflows/site-analytics.yml",
        "scripts/site-proof-milestones.json",
        "scripts/site-proof-refresh.sh",
        "scripts/site-proof.py",
        "scripts/site-proof-fixtures/page.html",
    ]
    for sample in SAMPLES:
        expect(f"{sample} is site-only", bool(SITE.match(sample)), True)
    for name in SITE_ONLY:
        rest = pattern([n for n in SITE_ONLY if n != name])
        alone = [x for x in SAMPLES if re.match(f"^(?:{name})$", x) and not rest.match(x)]
        expect(f"{name} has a sample only it matches", bool(alone), True)

    # Code and build input must never be classed site-only.
    for path in [
        "apps/macos/Sources/x.swift", "apps/ios/FarCooler/App.swift", "apps/android/app/src/A.kt",
        "apps/android/README.md", "apps/shared/AgentKit/README.md",
        "crates/core/src/lib.rs", "crates/core/fixtures/session-logs/README.md",
        "crates/daemon/assets/manager/SKILL.md", "crates/daemon/src/git.rs",
        "Cargo.toml", "Cargo.lock", "proto/farcooler.proto", "test/fixtures/a.json",
        "services/relay/src/lib.rs", "services/site-x/a.ts", "services/sitemap/a.ts", "services/site",
        "docs", "docsx/a.md", "mydocs/a.md", "scripts/docs/x.py", "scripts/copy-lint.py",
        "scripts/ci-changes.py", "scripts/site-proof-extra.py", "scripts/site-proofs/x",
        "scripts/manager-skill-pressure/scenarios.md",
        ".github/workflows/ci.yml", ".github/workflows/relay.yml", ".github/actions/ios-ui-shard/action.yml",
        "build-app.sh", "rust-toolchain.toml", "x.mdx", "a/README.md", "notes.md/x.rs",
    ]:
        expect(f"{path} is code", bool(SITE.match(path)), False)

    with tempfile.TemporaryDirectory() as tmp:
        repo = pathlib.Path(tmp)

        def run(*args):
            out = git(repo, "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", *args)
            if out.returncode != 0:
                raise RuntimeError(out.stderr)
            return out.stdout.strip()

        def commit(name, *files):
            for f in files:
                p = repo / f
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_text(f"{name}\n")
            run("add", "-A")
            run("commit", "-m", name)
            return run("rev-parse", "HEAD")

        run("init", "-q", "-b", "main")
        a = commit("a", "crates/daemon/a.rs")                       # green
        b = commit("b", "services/site/b.ts", "docs/b.md")           # site only
        c = commit("c", "README.md")                                 # site only
        d = commit("d", "crates/daemon/d.rs")                       # code, its CI cancelled
        e = commit("e", "services/site/e.ts")                       # site only
        f = commit("f", "services/site/f.ts", "crates/daemon/f.rs")  # mixed

        expect("a site-only push after a green one skips", decide(repo, "push", b, candidates=[a])[0], False)
        expect("two site-only pushes since the green one skip", decide(repo, "push", c, candidates=[a])[0], False)
        # The hole: d's own CI never went green, so e must not be judged by d..e.
        expect("a code push whose CI was cancelled, then a site-only push: the app jobs run",
               decide(repo, "push", e, candidates=[c])[0], True)
        expect("... also when the nearest green run is older still", decide(repo, "push", e, candidates=[a])[0], True)
        expect("after a green run past the code change, a site-only push skips",
               decide(repo, "push", e, candidates=[d])[0], False)
        expect("one code file among site files runs", decide(repo, "push", f, candidates=[e])[0], True)
        expect("no green ancestor runs everything", decide(repo, "push", e, candidates=[])[0], True)
        expect("a green run outside the history (force push) runs everything",
               decide(repo, "push", e, candidates=["1" * 40])[0], True)
        expect("the commit's own run is no base", decide(repo, "push", e, candidates=[e])[0], True)
        expect("an unknown event runs everything", decide(repo, "schedule")[0], True)
        expect("a workflow_dispatch runs everything", decide(repo, "workflow_dispatch", after=e, candidates=[d])[0], True)
        expect("a pull request of site files skips", decide(repo, "pull_request", base=a, head=c)[0], False)
        expect("... and runs for a code change", decide(repo, "pull_request", base=a, head=d)[0], True)
        expect("a pull request with a bad base runs", decide(repo, "pull_request", base="1" * 40, head=d)[0], True)
        expect("a pull request with nothing changed runs", decide(repo, "pull_request", base=c, head=c)[0], True)
        expect("a failed listing runs everything even with a green base",
               decide(repo, "push", c, candidates=[a], listing_error="HTTP 502")[0], True)

        outputs = repo / "out"
        cands = repo / "cands"
        cands.write_text(json.dumps({"head_sha": a}) + "\n")
        env = {**os.environ, "EVENT": "push", "AFTER": c, "CANDIDATES": str(cands), "GITHUB_OUTPUT": str(outputs)}
        env.pop("CANDIDATES_ERROR", None)
        done = subprocess.run([sys.executable, __file__], cwd=repo, env=env, capture_output=True, text=True)
        expect("the script exits 0", done.returncode, 0)
        expect("and writes app=false", outputs.read_text() if outputs.exists() else None, "app=false\n")
        outputs.unlink()
        env["CANDIDATES_ERROR"] = "gh: Server Error (HTTP 502)"
        done = subprocess.run([sys.executable, __file__], cwd=repo, env=env, capture_output=True, text=True)
        expect("a failed listing exits 0", done.returncode, 0)
        expect("with a notice", done.stdout.startswith("::notice::could not list the green CI runs"), True)
        expect("and writes app=true", outputs.read_text() if outputs.exists() else None, "app=true\n")

    for failure in failures:
        print(f"ci-changes self-test: {failure}", file=sys.stderr)
    if failures:
        return 1
    print("ci-changes self-test: ok")
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        sys.exit(self_test())
    if sys.argv[1:]:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    sys.exit(main())
