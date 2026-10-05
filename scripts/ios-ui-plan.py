#!/usr/bin/env python3
"""Do the iOS UI shards run for this push or pull request? (ov-301)

ci.yml's `ios-ui-plan` job runs this. The shards hold two `xcode-27` runners
for about half an hour each, so they run only when something the iOS app is
built from changed:

    EVENT=push BEFORE=<sha> AFTER=<sha> scripts/ios-ui-plan.py
    EVENT=pull_request BASE=<sha> HEAD=<sha> scripts/ios-ui-plan.py
    scripts/ios-ui-plan.py --self-test

It writes `run=true|false` to $GITHUB_OUTPUT and says why.

A pull request compares merge-base..head (`BASE...HEAD`). A push to main
compares `BEFORE..AFTER`, the commits the push added. Anything that stops that
range from being computed runs everything, because a skipped test is the one
failure nobody sees: a force push (BEFORE is no longer in the history), a new
branch (BEFORE is all zeros), an empty or unknown BEFORE, a missing object, or
any event but the two above.

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

# What the iOS app is built from: the app, AgentKit under apps/shared, the
# crates the xcframeworks link (farcooler-client and its path dependencies
# core, fence, protocol, tailcat, transport, plus farcooler-vt, per `cargo
# metadata`), the .proto protocol compiles, the shared fixtures, and this
# job's own scripts and workflow.
RELEVANT = re.compile(
    r"^(apps/ios/|apps/shared/|crates/(client|core|fence|protocol|tailcat|transport|vt)/|proto/|"
    r"test/fixtures/|Cargo\.(toml|lock)$|"
    r"scripts/(ios-ui-tests\.sh|ios-ui-shards\.py|ios-ui-plan\.py|build-ios-frameworks\.sh)$|"
    r"\.github/workflows/ci\.yml$)"
)
ZEROS = re.compile(r"^0+$")


def git(repo, *args):
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)


def decide(repo, event, before="", after="", base="", head=""):
    """(run, why)."""
    if event == "pull_request":
        spec = f"{base}...{head}"
    elif event == "push":
        if not before or ZEROS.match(before):
            return True, "a new branch or no earlier commit: the UI tests run."
        if git(repo, "cat-file", "-e", f"{before}^{{commit}}").returncode != 0:
            return True, "the push's earlier commit is not in this history (a force push): the UI tests run."
        spec = f"{before}..{after}"
    else:
        return True, f"{event or 'no event'}: the UI tests run."
    out = git(repo, "diff", "--name-only", spec)
    if out.returncode != 0:
        return True, f"could not compare {spec}: the UI tests run."
    files = out.stdout.splitlines()
    for f in files:
        if RELEVANT.match(f):
            return True, f"{f} is what the iOS app is built from: the UI tests run."
    return False, f"{len(files)} changed file(s), none of what the iOS app is built from: the UI tests are skipped."


def main():
    run, why = decide(
        pathlib.Path.cwd(), os.environ.get("EVENT", ""),
        os.environ.get("BEFORE", ""), os.environ.get("AFTER", ""),
        os.environ.get("BASE", ""), os.environ.get("HEAD", ""),
    )
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

    for path, want in [
        ("apps/ios/FarCooler/App.swift", True), ("apps/shared/AgentKit/x.swift", True),
        ("crates/client/src/lib.rs", True), ("crates/transport/a.rs", True), ("proto/farcooler.proto", True),
        ("Cargo.lock", True), ("scripts/ios-ui-shards.py", True), (".github/workflows/ci.yml", True),
        ("crates/daemon/src/lib.rs", False), ("apps/macos/Sources/x.swift", False),
        ("apps/android/a.kt", False), ("services/relay/x.rs", False), ("docs/a.md", False),
        ("crates/clientele/a.rs", False), (".github/workflows/canary.yml", False),
    ]:
        expect(f"relevant {path}", bool(RELEVANT.match(path)), want)

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
        a = commit("a", "apps/ios/a.swift")
        b = commit("b", "crates/daemon/b.rs")
        c = commit("c", "apps/macos/c.swift", "docs/c.md")
        d = commit("d", "crates/client/d.rs")
        e = commit("e", "crates/daemon/e.rs")

        expect("an unrelated push is skipped", decide(repo, "push", a, c)[0], False)
        expect("one relevant commit in a multi-commit push runs", decide(repo, "push", c, e)[0], True)
        expect("only the range counts: an earlier relevant commit does not", decide(repo, "push", d, e)[0], False)
        expect("an iOS commit runs", decide(repo, "push", e, commit("f", "apps/ios/f.swift"))[0], True)
        expect("a new branch runs everything", decide(repo, "push", "0" * 40, e)[0], True)
        expect("an empty before runs everything", decide(repo, "push", "", e)[0], True)
        expect("a force push (before not in history) runs everything", decide(repo, "push", "1" * 40, e)[0], True)
        expect("an unknown event runs everything", decide(repo, "schedule", "", "")[0], True)
        expect("a pull request compares against the merge base", decide(repo, "pull_request", base=a, head=c)[0], False)
        expect("... and runs for an iOS change", decide(repo, "pull_request", base=a, head=d)[0], True)
        expect("a pull request with a bad base runs", decide(repo, "pull_request", base="1" * 40, head=d)[0], True)

        # A push is the tree change from `before` to `after`, so a before that
        # sits on another line of history still counts what it had: two dots.
        run("checkout", "-q", "-b", "side", c)
        side = commit("side", "apps/ios/side.swift")
        run("checkout", "-q", "main")
        expect("a before off main's line compares trees, not the merge base", decide(repo, "push", side, c)[0], True)

        outputs = repo / "out"
        env = {**os.environ, "EVENT": "push", "BEFORE": a, "AFTER": c, "GITHUB_OUTPUT": str(outputs)}
        done = subprocess.run([sys.executable, __file__], cwd=repo, env=env, capture_output=True, text=True)
        expect("the script exits 0", done.returncode, 0)
        expect("and writes the output", outputs.read_text() if outputs.exists() else None, "run=false\n")

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
