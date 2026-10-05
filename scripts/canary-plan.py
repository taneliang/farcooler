#!/usr/bin/env python3
"""Should Canary build this commit? Decided before any macOS runner starts (ov-301).

canary.yml runs when CI completes on main (`workflow_run`), and this is its
first job. It answers two questions the trigger cannot:

  - WHICH COMMIT. A workflow_run workflow runs with `github.sha` and
    `github.ref` of main's head at the moment it starts, not of the commit CI
    just passed. The commit to build is the CI run's `head_sha`, and this
    prints it, so every later job checks out one value.
  - WHETHER ANYTHING APP-RELEVANT CHANGED. `paths-ignore` is a `push` filter
    and workflow_run has none, so the same rule is applied here: a commit
    whose changes are all under services/relay/, docs/ or proto/baseline/, or
    are Markdown files, ships nothing new.

    scripts/canary-plan.py            reads the environment, writes $GITHUB_OUTPUT
    scripts/canary-plan.py --self-test

Environment: EVENT (the workflow's event name), HEAD (the CI run's head_sha),
SHA (github.sha, for a manual dispatch) and CANDIDATES (a file with one CI
run per line, as JSON with a `head_sha`: main's successful CI runs).

Outputs `build=true|false`, `sha=<commit>` and the reason, printed.

THE RANGE. The changed files are those since the last commit whose CI run was
green and is in HEAD's history, not since the previous push. CI cancels a run
when the next push lands, so a push with no green verdict of its own is
common, and judging each push alone would drop an app change whenever a docs
push followed it. With no such commit, or none in the history (a force push),
everything builds.

SUPERSEDED. A green CI run for a commit that is no longer main's head, a
re-run of an old one, builds nothing: the newer commit ships when its own CI is
green, and an old build uploaded after a newer one would go out with a lower
build number. Commits after it that are not app-relevant (the baseline
recording, docs) do not supersede it.

A commit that is not on main is refused outright. Canary signs and uploads
with secrets, and `workflow_run` also fires for CI runs on pull requests from
forks, one of which can be named `main`; canary.yml checks the event too, and
this is the second lock.
"""

import json
import os
import pathlib
import subprocess
import sys
import tempfile

# What canary.yml's `paths-ignore` said, by the same rules: a push is skipped
# only when EVERY changed file matches one of these.
IGNORED_PREFIXES = ("services/relay/", "docs/", "proto/baseline/")
IGNORED_SUFFIXES = (".md",)


def ignored(path):
    return path.startswith(IGNORED_PREFIXES) or path.endswith(IGNORED_SUFFIXES)


def relevant(paths):
    """The changed files that make a Canary build worth its runner time."""
    return [p for p in paths if p and not ignored(p)]


def git(repo, *args):
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)


def changed(repo, base, head):
    out = git(repo, "diff", "--name-only", f"{base}..{head}")
    if out.returncode != 0:
        return None
    return out.stdout.splitlines()


def base_commit(repo, head, candidates):
    """The nearest ancestor of `head` among `candidates`, or None."""
    wanted = {c for c in candidates if c != head}
    if not wanted:
        return None
    out = git(repo, "rev-list", head)
    if out.returncode != 0:
        return None
    for sha in out.stdout.split()[1:]:
        if sha in wanted:
            return sha
    return None


def decide(repo, head, tip, candidates):
    """(build, reason) for a commit whose CI run just went green."""
    if git(repo, "merge-base", "--is-ancestor", head, tip).returncode != 0:
        return False, f"{head[:8]} is not on main, so it is not built"
    ahead = changed(repo, head, tip)
    if ahead is None:
        return True, f"could not compare {head[:8]} with main's head; building"
    if relevant(ahead):
        return False, (
            f"{head[:8]} is not main's head and what came after it changes the app, "
            "so a newer commit ships instead"
        )
    base = base_commit(repo, head, candidates)
    if base is None:
        return True, "no earlier green CI run in this history to compare with; building"
    files = changed(repo, base, head)
    if files is None:
        return True, f"could not compare {base[:8]} with {head[:8]}; building"
    hit = relevant(files)
    if hit:
        return True, f"{len(hit)} app-relevant file(s) changed since {base[:8]}, the last green CI run: {hit[0]}"
    return False, f"nothing app-relevant changed since {base[:8]}, the last green CI run ({len(files)} file(s))"


def read_candidates(path):
    if not path or not os.path.exists(path):
        return []
    rows = [json.loads(line) for line in pathlib.Path(path).read_text().splitlines() if line.strip()]
    return [r["head_sha"] for r in rows if r.get("head_sha")]


def emit(build, sha, reason):
    print(f"canary-plan: {'build' if build else 'skip'} {sha[:8]}: {reason}")
    out = os.environ.get("GITHUB_OUTPUT")
    if out:
        with open(out, "a") as f:
            f.write(f"build={'true' if build else 'false'}\nsha={sha}\n")


def main_run():
    event = os.environ.get("EVENT", "")
    if event == "workflow_dispatch":
        sha = os.environ.get("SHA", "")
        if not sha:
            print("canary-plan: SHA is empty", file=sys.stderr)
            return 2
        emit(True, sha, "dispatched by hand: builds whatever it was dispatched on")
        return 0
    if event != "workflow_run":
        print(f"canary-plan: unexpected event {event!r}", file=sys.stderr)
        return 2
    head = os.environ.get("HEAD", "")
    if len(head) != 40:
        print(f"canary-plan: HEAD {head!r} is not a commit", file=sys.stderr)
        return 2
    repo = pathlib.Path.cwd()
    if git(repo, "rev-parse", "--is-shallow-repository").stdout.strip() != "false":
        print("canary-plan: this checkout is shallow; it needs fetch-depth: 0", file=sys.stderr)
        return 2
    tip = git(repo, "rev-parse", "origin/main").stdout.strip()
    if not tip or git(repo, "cat-file", "-e", f"{head}^{{commit}}").returncode != 0:
        emit(False, head, f"{head[:8]} is not in main's history, so it is not built")
        return 0
    build, reason = decide(repo, head, tip, read_candidates(os.environ.get("CANDIDATES")))
    emit(build, head, reason)
    return 0


def self_test():
    failures = []

    def expect(what, got, want):
        if got != want:
            failures.append(f"{what}: got {got!r}, want {want!r}")

    # The path rule, one case per line of the old paths-ignore.
    for path, want in [
        ("services/relay/src/lib.rs", False), ("docs/plan.md", False), ("README.md", False),
        ("apps/ios/x/NOTES.md", False), ("proto/baseline/canary.proto", False),
        ("apps/ios/App.swift", True), ("proto/farcooler.proto", True),
        (".github/workflows/canary.yml", True), ("crates/core/src/lib.rs", True),
        ("services/relayx/a.rs", True), ("scripts/docs/x.py", True), ("mdbook", True),
    ]:
        expect(f"relevant {path}", bool(relevant([path])), want)
    expect("one relevant file among ignored ones", relevant(["docs/a.md", "apps/a.swift"]), ["apps/a.swift"])

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
        a = commit("a", "apps/a.swift")           # green, shipped
        b = commit("b", "docs/b.md")              # no CI verdict of its own (cancelled)
        c = commit("c", "apps/c.swift")           # no CI verdict (cancelled by d)
        d = commit("d", "docs/d.md")              # green: docs only on its own
        e = commit("e", "services/relay/e.rs")    # green
        f = commit("f", "apps/f.swift")           # green
        g = commit("g", "proto/baseline/canary.proto")  # the baseline bot's commit
        tip = g

        # d is superseded by f once main has moved on.
        expect("an older green commit is superseded", decide(repo, d, g, [a, e, f])[0], False)
        # d is docs-only, but the app change c, which never had a green run of
        # its own, is in the range since a: judged push by push it would be lost.
        expect("the range reaches back to the last green run", decide(repo, d, d, [a])[0], True)
        expect("... and says which file", "apps/c.swift" in decide(repo, d, d, [a])[1], True)
        # Since d, e is relay-only: nothing to ship.
        expect("an ignored-only range ships nothing", decide(repo, e, e, [d])[0], False)
        expect("an app change since the last green run ships on a later push", decide(repo, e, e, [a])[0], True)
        # The baseline bot's commit after f does not supersede it.
        expect("a baseline commit does not supersede", decide(repo, f, tip, [e])[0], True)
        # A newer app change does.
        h = commit("h", "apps/h.swift")
        expect("a newer app change supersedes a re-run", decide(repo, f, h, [e])[0], False)
        expect("... and says so", "newer commit ships" in decide(repo, f, h, [e])[1], True)
        # Docs after it do not.
        i = commit("i", "docs/i.md")
        expect("newer docs do not supersede", decide(repo, h, i, [f])[0], True)
        # No earlier green run: everything builds, including a docs-only head.
        expect("no earlier green run builds", decide(repo, i, i, [])[0], True)
        # An earlier green run that is not in the history (force push) is ignored.
        expect("a candidate outside the history is ignored", decide(repo, i, i, ["0" * 40])[0], True)
        # The nearest ancestor wins, whatever the candidate order.
        expect("the nearest green ancestor is the base", base_commit(repo, i, [a, h, f]), h)
        expect("the head itself is not its own base", base_commit(repo, i, [i]), None)
        # Nearest base, docs only since: skip.
        expect("docs since the last green run skip", decide(repo, i, i, [h])[0], False)
        # A commit off main is refused.
        run("checkout", "-q", "-b", "side", c)
        s = commit("s", "apps/s.swift")
        expect("a commit not on main is refused", decide(repo, s, i, [a])[0], False)
        expect("... with the reason", "not on main" in decide(repo, s, i, [a])[1], True)

        # And through the environment, the way the workflow calls it.
        run("checkout", "-q", "main")
        run("update-ref", "refs/remotes/origin/main", i)
        outputs = repo / "out"
        cands = repo / "cands"
        cands.write_text(json.dumps({"head_sha": h}) + "\n")
        env = {**os.environ, "EVENT": "workflow_run", "HEAD": i, "CANDIDATES": str(cands),
               "GITHUB_OUTPUT": str(outputs)}
        done = subprocess.run([sys.executable, __file__], cwd=repo, env=env, capture_output=True, text=True)
        expect("the script runs", done.returncode, 0)
        expect("it writes build=false and the commit", outputs.read_text(), f"build=false\nsha={i}\n")
        outputs.unlink()
        env.update(EVENT="workflow_dispatch", SHA=c)
        done = subprocess.run([sys.executable, __file__], cwd=repo, env=env, capture_output=True, text=True)
        expect("a dispatch builds what it was given", outputs.read_text(), f"build=true\nsha={c}\n")
        env.update(EVENT="pull_request")
        done = subprocess.run([sys.executable, __file__], cwd=repo, env=env, capture_output=True, text=True)
        expect("an unexpected event is an error", done.returncode, 2)

    for failure in failures:
        print(f"canary-plan self-test: {failure}", file=sys.stderr)
    if failures:
        return 1
    print("canary-plan self-test: ok")
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        sys.exit(self_test())
    if sys.argv[1:]:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    sys.exit(main_run())
