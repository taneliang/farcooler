#!/usr/bin/env python3
"""Do the iOS UI shards run for this push or pull request? (ov-301)

ci.yml's `ios-ui-plan` job runs this. The shards hold two `xcode-27` runners
for about half an hour each, so they run only when something the iOS app is
built from changed:

    EVENT=push AFTER=<sha> CANDIDATES=<file> scripts/ios-ui-plan.py
    EVENT=pull_request BASE=<sha> HEAD=<sha> scripts/ios-ui-plan.py
    scripts/ios-ui-plan.py --self-test

It writes `run=true|false` to $GITHUB_OUTPUT and says why.

A pull request compares merge-base..head (`BASE...HEAD`).

A push to main compares the nearest ancestor of `AFTER` whose CI run on main
was GREEN, with `AFTER`: not the push's own `before`. CI cancels a run when
the next push lands (ci.yml's concurrency group), and a run can end red, so
the previous push may never have had its UI tests pass. Diffing from `before`
let an iOS change whose CI was cancelled by a daemon-only push go unrun, and
Canary, which measures from the last green run, then shipped it. By induction
the rule is sound: a green run that skipped its shards had no iOS change since
its own green base. CANDIDATES is a file with one successful CI run per line,
as JSON with a `head_sha`; scripts/canary-plan.py reads the same list and owns
the nearest-ancestor choice (`base_commit`), shared here rather than copied.

When GitHub cannot list the green runs (the workflow step's `gh api` exits
non-zero: a 502, a timeout) the step records why in CANDIDATES_ERROR and
everything runs, with a `::notice::` saying so (ov-340): an outage must not
fail the push's CI, and running every shard is the safe side. A list that
gh returned successfully but that does not parse, and an unknown event, are
still loud failures: those are bugs, not weather.

With no green ancestor in the history (a first run, a force push), or a range that cannot be computed, or an unknown event,
everything runs, because a skipped test is the one failure nobody sees.

The owner chose this on Oct 5 (ov-301): main used to run the shards on every
push as a deliberate backstop, and that, with Canary, kept GitHub's five macOS
runners busy and pushes queued.
"""

import os
import pathlib
import re
import subprocess
import sys
import tempfile
import importlib.util
import json
try:
    import tomllib
except ImportError:  # Python before 3.11
    tomllib = None

ROOT = pathlib.Path(__file__).resolve().parent.parent

# What the iOS app is built from, one alternative per entry; the self-test
# holds a sample path for each, matched by that entry alone. The app,
# AgentKit under apps/shared, the crates the two xcframeworks link (the path
# dependencies of farcooler-client and farcooler-vt, which the self-test reads
# from their Cargo.toml files, so a new one cannot open a silent hole), the
# .proto protocol compiles, the shared fixtures, the workspace manifest and
# lockfile, and this job's own scripts and workflow.
PATTERNS = [
    r"apps/ios/",
    r"apps/shared/",
    r"crates/client/",
    r"crates/core/",
    r"crates/fence/",
    r"crates/ffi-guard/",
    r"crates/protocol/",
    r"crates/tailcat/",
    r"crates/transport/",
    r"crates/vt/",
    r"proto/",
    r"test/fixtures/",
    r"Cargo\.toml$",
    r"Cargo\.lock$",
    r"scripts/ios-ui-tests\.sh$",
    r"scripts/ios-ui-shards\.py$",
    r"scripts/ios-ui-plan\.py$",
    r"scripts/build-ios-frameworks\.sh$",
    r"\.github/workflows/ci\.yml$",
    r"\.github/actions/ios-ui-shard/",
]


def pattern(names):
    return re.compile("^(" + "|".join(names) + ")")


RELEVANT = pattern(PATTERNS)
ZEROS = re.compile(r"^0+$")


def canary_plan():
    spec = importlib.util.spec_from_file_location("canary_plan", ROOT / "scripts" / "canary-plan.py")
    module = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True
    spec.loader.exec_module(module)
    return module


def linked_crates(root=ROOT):
    """The crate directories farcooler-client and farcooler-vt depend on, with themselves.

    Read from Cargo.toml (path dependencies, and workspace ones resolved
    through the root manifest), so it needs no cargo and no registry.
    """
    def load(path):
        return tomllib.loads(path.read_text())

    workspace = load(root / "Cargo.toml").get("workspace", {}).get("dependencies", {})

    def dep_dirs(manifest, directory):
        tables = [manifest.get("dependencies", {}), manifest.get("build-dependencies", {})]
        for target in manifest.get("target", {}).values():
            tables += [target.get("dependencies", {}), target.get("build-dependencies", {})]
        for table in tables:
            for name, spec in table.items():
                if not isinstance(spec, dict):
                    continue
                if "path" in spec:
                    yield os.path.normpath(os.path.join(directory, spec["path"]))
                elif spec.get("workspace") and isinstance(workspace.get(name), dict) and "path" in workspace[name]:
                    yield os.path.normpath(workspace[name]["path"])  # relative to the root

    seen, todo = set(), ["crates/client", "crates/vt"]
    while todo:
        directory = todo.pop()
        if directory in seen:
            continue
        seen.add(directory)
        manifest = load(root / directory / "Cargo.toml")
        todo.extend(dep_dirs(manifest, directory))
    return sorted(seen)


def git(repo, *args):
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)


def decide(repo, event, after="", base="", head="", candidates=(), listing_error=""):
    """(run, why)."""
    if event == "push" and listing_error:
        return True, f"could not list the green CI runs ({listing_error}): the UI tests run."
    if event == "pull_request":
        spec = f"{base}...{head}"
    elif event == "push":
        green = canary_plan().base_commit(repo, after, list(candidates))
        if green is None:
            return True, "no earlier green CI run in this history to compare with: the UI tests run."
        spec = f"{green}..{after}"
    else:
        return True, f"{event or 'no event'}: the UI tests run."
    out = git(repo, "diff", "--name-only", spec)
    if out.returncode != 0:
        return True, f"could not compare {spec}: the UI tests run."
    files = out.stdout.splitlines()
    for f in files:
        if RELEVANT.match(f):
            return True, f"{f} is what the iOS app is built from: the UI tests run."
    return False, f"{len(files)} changed file(s) since {spec.split('..')[0][:8]}, none of what the iOS app is built from: the UI tests are skipped."


def main():
    error = os.environ.get("CANDIDATES_ERROR", "").strip()
    # With the listing failed there is no file to parse; with it listed, a
    # file that does not parse raises, and the job fails.
    candidates = [] if error else canary_plan().read_candidates(os.environ.get("CANDIDATES"))
    run, why = decide(
        pathlib.Path.cwd(), os.environ.get("EVENT", ""), os.environ.get("AFTER", ""),
        os.environ.get("BASE", ""), os.environ.get("HEAD", ""),
        candidates, error,
    )
    if error and os.environ.get("EVENT") == "push":
        print(f"::notice::{why}")
    else:
        print(why)
    out = os.environ.get("GITHUB_OUTPUT")
    if out:
        with open(out, "a") as f:
            f.write(f"run={'true' if run else 'false'}\n")
    return 0


def self_test():
    failures = []

    def expect(what, got, want):
        if got != want:
            failures.append(f"{what}: got {got!r}, want {want!r}")

    # Every alternative is pinned by a sample only it matches, so deleting
    # any one from PATTERNS goes red here.
    SAMPLES = [
        "apps/ios/FarCooler/App.swift",
        "apps/shared/AgentKit/Sources/x.swift",
        "crates/client/src/lib.rs",
        "crates/core/src/lib.rs",
        "crates/fence/src/lib.rs",
        "crates/ffi-guard/src/lib.rs",
        "crates/protocol/src/lib.rs",
        "crates/tailcat/go/go.mod",
        "crates/transport/src/lib.rs",
        "crates/vt/src/lib.rs",
        "proto/farcooler.proto",
        "test/fixtures/a.json",
        "Cargo.toml",
        "Cargo.lock",
        "scripts/ios-ui-tests.sh",
        "scripts/ios-ui-shards.py",
        "scripts/ios-ui-plan.py",
        "scripts/build-ios-frameworks.sh",
        ".github/workflows/ci.yml",
        ".github/actions/ios-ui-shard/action.yml",
    ]
    for sample in SAMPLES:
        expect(f"{sample} is in the path set", bool(RELEVANT.match(sample)), True)
    for name in PATTERNS:
        rest = pattern([n for n in PATTERNS if n != name])
        alone = [x for x in SAMPLES if re.match(f"^({name})", x) and not rest.match(x)]
        expect(f"{name} has a sample only it matches", bool(alone), True)
    for path in ["crates/daemon/src/lib.rs", "crates/review/src/lib.rs", "apps/macos/Sources/x.swift",
                 "apps/android/a.kt", "services/relay/x.rs", "docs/a.md", "crates/clientele/a.rs",
                 ".github/workflows/canary.yml", "scripts/canary-plan.py"]:
        expect(f"{path} is not iOS", bool(RELEVANT.match(path)), False)

    # The crates the xcframeworks link are all in the set, read from the
    # manifests as they are now.
    if tomllib is None:
        failures.append("Python 3.11 or later is needed for tomllib")
    else:
        linked = linked_crates()
        expect("the linked crates include the client and vt", {"crates/client", "crates/vt"} <= set(linked), True)
        expect("the linked crates include ffi-guard", "crates/ffi-guard" in linked, True)
        for directory in linked:
            expect(f"{directory} is in the path set", bool(RELEVANT.match(f"{directory}/src/lib.rs")), True)

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
        a = commit("a", "apps/ios/a.swift")      # green
        b = commit("b", "crates/daemon/b.rs")    # green, shards skipped
        c = commit("c", "apps/macos/c.swift", "docs/c.md")
        d = commit("d", "crates/client/d.rs")    # iOS change, CI cancelled by e
        e = commit("e", "crates/daemon/e.rs")    # daemon only
        f = commit("f", "crates/daemon/f.rs")

        expect("an unrelated push is skipped", decide(repo, "push", c, candidates=[a, b])[0], False)
        # The hole: d's own CI was cancelled, so e must not be judged by d..e.
        expect("an iOS push whose CI was cancelled, then a daemon-only push: the shards run",
               decide(repo, "push", e, candidates=[c])[0], True)
        expect("... also when the nearest green run is older still", decide(repo, "push", f, candidates=[a])[0], True)
        # And once a push after d went green with the shards run, later ones skip.
        expect("after a green run past the iOS change the next daemon push skips",
               decide(repo, "push", f, candidates=[e])[0], False)
        # A red run is no green base: only successes are listed, so a red d is
        # simply absent, and e is measured from before it.
        expect("a red iOS push, then a daemon-only one: the shards run", decide(repo, "push", e, candidates=[b])[0], True)
        expect("no green ancestor runs everything", decide(repo, "push", f, candidates=[])[0], True)
        expect("a green run outside the history (force push) runs everything",
               decide(repo, "push", f, candidates=["1" * 40])[0], True)
        expect("the commit's own run is no base", decide(repo, "push", f, candidates=[f])[0], True)
        expect("an unknown event runs everything", decide(repo, "schedule")[0], True)
        expect("a pull request compares against the merge base", decide(repo, "pull_request", base=a, head=c)[0], False)
        expect("... and runs for an iOS change", decide(repo, "pull_request", base=a, head=d)[0], True)
        expect("a pull request with a bad base runs", decide(repo, "pull_request", base="1" * 40, head=d)[0], True)

        expect("a failed listing runs everything even with a green base to compare with",
               decide(repo, "push", f, candidates=[e], listing_error="HTTP 502")[0], True)
        expect("... and says why", "HTTP 502" in decide(repo, "push", f, listing_error="HTTP 502")[1], True)
        expect("a failed listing does not touch a pull request",
               decide(repo, "pull_request", base=a, head=c, listing_error="HTTP 502")[0], False)

        outputs = repo / "out"
        cands = repo / "cands"
        cands.write_text(json.dumps({"head_sha": e}) + "\n")
        env = {**os.environ, "EVENT": "push", "AFTER": f, "CANDIDATES": str(cands), "GITHUB_OUTPUT": str(outputs)}
        done = subprocess.run([sys.executable, __file__], cwd=repo, env=env, capture_output=True, text=True)
        expect("the script exits 0", done.returncode, 0)
        expect("and writes the output", outputs.read_text() if outputs.exists() else None, "run=false\n")

        # Through the script: a failed listing runs everything and prints a
        # notice; a list gh returned that does not parse still fails.
        outputs.unlink()
        env["CANDIDATES_ERROR"] = "gh: Server Error (HTTP 502)"
        done = subprocess.run([sys.executable, __file__], cwd=repo, env=env, capture_output=True, text=True)
        expect("a failed listing exits 0", done.returncode, 0)
        expect("with a notice", done.stdout.startswith("::notice::could not list the green CI runs (gh: Server Error (HTTP 502))"), True)
        expect("and runs the shards", outputs.read_text() if outputs.exists() else None, "run=true\n")
        del env["CANDIDATES_ERROR"]
        cands.write_text("<html>502 Bad Gateway</html>\n")
        done = subprocess.run([sys.executable, __file__], cwd=repo, env=env, capture_output=True, text=True)
        expect("a malformed list fails loudly", done.returncode != 0, True)

    for failure in failures:
        print(f"ios-ui-plan self-test: {failure}", file=sys.stderr)
    if failures:
        return 1
    print("ios-ui-plan self-test: ok")
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        sys.exit(self_test())
    if sys.argv[1:]:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    sys.exit(main())
