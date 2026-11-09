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
    # The web client's Worker and the session code it shares with the site
    # (ov-764): TypeScript Workers that no app job builds or tests. Its wasm
    # crates, when they come, live under crates/ and stay code.
    r"services/web/.+",
    r"services/common/.+",
    r"\.github/workflows/web\.yml",
    r"docs/.+",
    r"[^/]+\.md",
    r"\.github/workflows/site\.yml",
    r"\.github/workflows/site-analytics\.yml",
    r"scripts/site-proof-milestones\.json",
    r"scripts/site-proof-tokens\.json",
    r"scripts/site-proof-refresh\.sh",
    r"scripts/site-proof\.py",
    r"scripts/site-proof-fixtures/.+",
]


def pattern(names):
    return re.compile("^(?:" + "|".join(names) + ")$")


SITE = pattern(SITE_ONLY)


# Which job families a path reaches (ov-1021). `app` stays what it was: any file off the
# allowlist above. The families narrow it: a Swift-only change has no business in the
# Rust, Android or relay jobs. Anything not named here, and every shared input (the
# wire in proto/, Cargo.*, every crate the apps link, the toolchain, scripts, the
# workflows and actions), reaches ALL of them: a job a change can break never skips.
FAMILIES = ["rust", "apple", "android", "web", "relay"]
FLAGS = [*FAMILIES, "docs", "app"]
# crates/android (farcooler-jni) is linked by Android only; crates/cli is built by the Mac
# (apps/macos/test.sh `cargo build --bins`) and not by Android; crates/web is its own
# wasm workspace, linked by neither app (checked with `cargo metadata`, ov-1021). Every
# other crate is in both apps' closures, so it is shared.
NARROW = [
    # Cross-reads, found by grep (ov-1021): Rust and wasm tests include the relay's fixtures;
    # relay imports services/common and Notifier.kt; Android tests read AgentKit and its
    # build reads iOS's fonts. test/fixtures is read by every platform, so it is not narrowed.
    (r"services/relay/test/fixtures/.+", {"relay", "rust", "web"}),
    (r"services/relay/.+", {"relay"}),
    (r"services/common/.+", {"web", "relay"}),
    (r"services/web/.+|\.github/workflows/web\.yml", {"web"}),
    (r"apps/android/.+", {"android", "relay"}),
    (r"apps/ios/FarCooler/Fonts/.+|apps/shared/.+", {"apple", "android"}),
    (r"apps/(ios|macos)/.+", {"apple"}),
    (r"crates/android/.+", {"rust", "android"}),
    (r"crates/cli/.+", {"rust", "apple"}),
    (r"crates/web/.+", {"rust", "web"}),
    (r"test/live_agents\.rs|\.config/.+", {"rust"}),
]


def families(files):
    """{flag: bool} for a list of changed files; None (unknown) turns every flag on."""
    if files is None:
        return {k: True for k in FLAGS}
    hit = set()
    app = docs = False
    for f in files:
        narrow = next((fam for rx, fam in NARROW if re.fullmatch(rx, f)), None)
        if SITE.match(f):
            docs = True
            hit |= narrow or set()
        else:
            app = True
            hit |= narrow if narrow is not None else set(FAMILIES)
    return {**{k: k in hit for k in FAMILIES}, "docs": docs, "app": app}


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
    flags, why = decide_flags(repo, event, after, base, head, candidates, listing_error)
    return flags["app"], why


def decide_flags(repo, event, after="", base="", head="", candidates=(), listing_error=""):
    """({flag: bool}, why): which job families run. Anything unknown turns them all on."""
    unknown = families(None)
    flags, why = _decide(repo, event, after, base, head, candidates, listing_error)
    return (unknown if flags is None else flags), why


def _decide(repo, event, after, base, head, candidates, listing_error):
    if event == "push" and listing_error:
        return None, f"could not list the green CI runs ({listing_error}): the app jobs run."
    if event == "pull_request":
        spec = f"{base}...{head}"
    elif event == "push":
        green = canary_plan().base_commit(repo, after, list(candidates))
        if green is None:
            return None, "no earlier green CI run in this history to compare with: the app jobs run."
        spec = f"{green}..{after}"
    else:
        return None, f"{event or 'no event'}: the app jobs run."
    out = git(repo, "diff", "--no-renames", "--name-only", spec)
    if out.returncode != 0:
        return None, f"could not compare {spec}: the app jobs run."
    files = [f for f in out.stdout.splitlines() if f]
    if not files:
        return None, f"no changed files in {spec}: the app jobs run."
    flags = families(files)
    if flags["app"]:
        off = [k for k in FAMILIES if not flags[k]]
        first = next(f for f in files if not SITE.match(f))
        return flags, f"{first} is not website or docs: the app jobs run" + (f" (not {', '.join(off)})." if off else ".")
    return flags, f"{len(files)} changed file(s) since {spec.split('..')[0][:8]}, all website or docs: the app jobs are skipped."


def main():
    error = os.environ.get("CANDIDATES_ERROR", "").strip()
    candidates = [] if error else canary_plan().read_candidates(os.environ.get("CANDIDATES"))
    flags, why = decide_flags(
        pathlib.Path.cwd(), os.environ.get("EVENT", ""), os.environ.get("AFTER", ""),
        os.environ.get("BASE", ""), os.environ.get("HEAD", ""),
        candidates, error,
    )
    print(f"::notice::{why}" if error and os.environ.get("EVENT") == "push" else why)
    out = os.environ.get("GITHUB_OUTPUT")
    if out:
        with open(out, "a") as f:
            for k in FLAGS:
                f.write(f"{k}={'true' if flags[k] else 'false'}\n")
    return 0


def self_test():
    failures = []

    def expect(what, got, want):
        if got != want:
            failures.append(f"{what}: got {got!r}, want {want!r}")

    # Every alternative has a sample only it matches, so deleting one goes red.
    SAMPLES = [
        "services/site/src/index.ts",
        "services/web/src/index.ts",
        "services/common/session.ts",
        ".github/workflows/web.yml",
        "docs/releasing.md",
        "docs/superpowers/specs/a/b.json",
        "README.md",
        ".github/workflows/site.yml",
        ".github/workflows/site-analytics.yml",
        "scripts/site-proof-milestones.json",
        "scripts/site-proof-tokens.json",
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
        "services/web", "services/webx/a.ts", "crates/web/src/lib.rs", ".github/workflows/web.ymlx",
        "docs", "docsx/a.md", "mydocs/a.md", "scripts/docs/x.py", "scripts/copy-lint.py",
        "scripts/ci-changes.py", "scripts/site-proof-extra.py", "scripts/site-proofs/x",
        "scripts/manager-skill-pressure/scenarios.md",
        ".github/workflows/ci.yml", ".github/workflows/relay.yml", ".github/actions/ios-ui-shard/action.yml",
        "build-app.sh", "rust-toolchain.toml", "x.mdx", "a/README.md", "notes.md/x.rs",
    ]:
        expect(f"{path} is code", bool(SITE.match(path)), False)


    # Which job families a change reaches (ov-1021). A shared input runs all of them.
    ALL = {"rust", "apple", "android", "web", "relay"}
    for what, files, want in [
        ("a Rust-only crate change", ["crates/web/src/lib.rs"], {"rust", "web"}),
        ("a Swift-only change", ["apps/macos/Sources/x.swift", "apps/ios/FarCooler/a.swift"], {"apple"}),
        ("an iOS-only change", ["apps/ios/FarCooler/App.swift"], {"apple"}),
        ("a Kotlin change, which relay.test.ts reads", ["apps/android/app/src/A.kt"], {"android", "relay"}),
        ("the JNI crate", ["crates/android/src/lib.rs"], {"rust", "android"}),
        ("the CLI crate, which the Mac bundles", ["crates/cli/src/main.rs"], {"rust", "apple"}),
        ("the Rust live-agent test", ["test/live_agents.rs"], {"rust"}),
        ("nextest config", [".config/nextest.toml"], {"rust"}),
        ("a shared contract fixture, which every platform's tests read", ["test/fixtures/plan.json"], ALL),
        ("a relay fixture, which Rust and the wasm tests include", ["services/relay/test/fixtures/device-proof.json"], {"relay", "rust", "web"}),
        ("AgentKit, whose files Android tests read", ["apps/shared/AgentKit/Sources/AgentKit/RunnerRefusal.swift"], {"apple", "android"}),
        ("Kotlin, which relay.test.ts imports", ["apps/android/app/src/main/java/com/farcooler/notify/Notifier.kt"], {"android", "relay"}),
        ("session code relay imports", ["services/common/visitor.ts"], {"web", "relay"}),
        ("fonts Android builds from iOS's folder", ["apps/ios/FarCooler/Fonts/A.ttf"], {"apple", "android"}),
        ("a relay change", ["services/relay/src/lib.rs"], {"relay"}),
        ("a web worker change", ["services/web/src/index.ts"], {"web"}),
        ("a docs-only change", ["docs/a.md", "README.md"], set()),
        ("a wire change", ["proto/farcooler.proto"], ALL),
        ("Cargo.lock", ["Cargo.lock"], ALL),
        ("the client crate", ["crates/client/src/lib.rs"], ALL),
        ("a core crate", ["crates/core/src/lib.rs"], ALL),
        ("a script", ["scripts/copy-lint.py"], ALL),
        ("ci.yml", [".github/workflows/ci.yml"], ALL),
        ("a path nobody classified", ["Makefile"], ALL),
        ("Swift and Kotlin together", ["apps/macos/a.swift", "apps/android/a.kt"], {"apple", "android", "relay"}),
        ("Swift and a wire change", ["apps/macos/a.swift", "proto/farcooler.proto"], ALL),
        ("a renamed-into-docs crate file is still code", ["crates/daemon/src/a.rs", "docs/a.md"], ALL),
    ]:
        got = {k for k, v in families(files).items() if v and k in ALL}
        expect(f"{what} selects {sorted(want)}", got, want)
    expect("docs-only sets the docs flag", families(["docs/a.md"])["docs"], True)
    expect("docs-only leaves app off", families(["docs/a.md"])["app"], False)
    expect("a Swift change leaves docs off", families(["apps/macos/a.swift"])["docs"], False)
    expect("every flag is on when nothing is known", families(None), {k: True for k in [*ALL, "docs", "app"]})

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
        # A rename shows only its new path in `git diff --name-only`: moving a
        # code file into docs/ must still run the app jobs.
        run("mv", "crates/daemon/f.rs", "docs/f.rs")
        run("commit", "-m", "move")
        m = run("rev-parse", "HEAD")
        expect("a code file moved into docs/ runs the app jobs", decide(repo, "push", m, candidates=[f])[0], True)
        expect("... also in a pull request", decide(repo, "pull_request", base=f, head=m)[0], True)
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
        expect("and writes every flag off but docs", outputs.read_text() if outputs.exists() else None,
               "rust=false\napple=false\nandroid=false\nweb=false\nrelay=false\ndocs=true\napp=false\n")
        outputs.unlink()
        env["CANDIDATES_ERROR"] = "gh: Server Error (HTTP 502)"
        done = subprocess.run([sys.executable, __file__], cwd=repo, env=env, capture_output=True, text=True)
        expect("a failed listing exits 0", done.returncode, 0)
        expect("with a notice", done.stdout.startswith("::notice::could not list the green CI runs"), True)
        expect("and writes every flag on", outputs.read_text() if outputs.exists() else None,
               "rust=true\napple=true\nandroid=true\nweb=true\nrelay=true\ndocs=true\napp=true\n")

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
