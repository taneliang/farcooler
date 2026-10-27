#!/usr/bin/env python3
"""Say whether this worktree's last lane-gate run lets HEAD be pushed (ov-822).

    scripts/gate-summary-check.py [--worktree DIR] [--waive "<reason>"]
    scripts/gate-summary-check.py --self-test

scripts/lane-gate.py saves its summary to `<git-dir>/lane-gate-summary.txt` with the
full sha it gated. This exits 0 only if that file exists, names the current HEAD, is
not PARTIAL (an --only/--skip run that left out a job the diff reaches), and is
either all passed or FAILED only on known environment gaps (GAP_NAMES, below).
Anything else exits 1 with the reason.

--waive "<reason>" lets a refused push through, exits 0, and appends the reason, the
sha and what it overrode to `<git-dir>/gate-waivers.log` and to stderr. A waiver is
for a gate that can't be run (say, the host lacks a toolchain), never for a red one.
"""
import argparse
import datetime
import pathlib
import subprocess
import sys
import tempfile

SUMMARY_NAME = "lane-gate-summary.txt"  # keep equal to lane-gate.py's
# Failures a host can have that say nothing about the code. Each name is tagged by
# ENV_GAPS in lane-gate.py:
#   git-lfs-missing               the two daemon tests that need git-lfs, run with CI=true on a Mac without it
#   wrangler-dry-run-without-dist `wrangler deploy --dry-run` in the web job when the client dist was not built here
GAP_NAMES = {"git-lfs-missing", "wrangler-dry-run-without-dist"}


def git(repo, *args):
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True).stdout.strip()


def parse(text):
    """(header dict, failed steps as [(job, command, gap)]) from a saved summary."""
    head, fails = {}, []
    for line in text.split("\n\n", 1)[0].splitlines():
        key, _, val = line.partition(": ")
        if key == "failed-step":
            job, cmd, gap = (x.strip() for x in val.rsplit(" | ", 2)) if val.count(" | ") >= 2 else (val, "", "gap=none")
            fails.append((job, cmd, gap.removeprefix("gap=")))
        elif val:
            head[key] = val
    return head, fails


def problem(repo):
    """Why the gate does not cover HEAD, or "" when it does."""
    gitdir = git(repo, "rev-parse", "--absolute-git-dir")
    path = pathlib.Path(gitdir) / SUMMARY_NAME
    sha = git(repo, "rev-parse", "HEAD")
    if not sha or not gitdir:
        return f"{repo} is not a git checkout"
    if not path.is_file():
        return f"no gate summary at {path}; run scripts/lane-gate.py --worktree {repo}"
    head, fails = parse(path.read_text())
    if "head" not in head:
        return f"the gate summary at {path} is unreadable (no head line)"
    if head["head"] != sha:
        return f"the gate summary is for {head['head'][:9]}, but HEAD is {sha[:9]}; run scripts/lane-gate.py again"
    verdict = head.get("verdict")
    if verdict == "partial":
        return "the gate summary is PARTIAL (--only/--skip left out a job this diff reaches); run scripts/lane-gate.py with no filters"
    if verdict == "ok":
        return ""
    if verdict == "failed":
        bad = [f"{j}: {c}" for j, c, gap in fails if gap not in GAP_NAMES]
        if not fails:
            return "the gate summary is FAILED with no failed step recorded (the gate itself broke); run scripts/lane-gate.py again"
        if bad:
            return "the gate summary is FAILED on " + "; ".join(bad)
        return ""
    return f"the gate summary at {path} is unreadable (verdict {verdict!r})"


def check(repo, waive=""):
    """(exit code, message)."""
    why = problem(repo)
    if not why:
        return 0, "gate-summary-check: ok, the gate covers HEAD"
    if waive.strip():
        gitdir = git(repo, "rev-parse", "--absolute-git-dir")
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        line = f"{stamp} {git(repo, 'rev-parse', 'HEAD')} WAIVED ({waive.strip()}): {why}"
        with open(pathlib.Path(gitdir) / "gate-waivers.log", "a") as f:
            f.write(line + "\n")
        return 0, "gate-summary-check: " + line
    return 1, "gate-summary-check: refused: " + why


def self_test():
    failures = []

    def expect(what, got, want):
        if got != want:
            failures.append(f"{what}: got {got!r}, want {want!r}")

    def summary(sha, verdict, steps=()):
        lines = ["lane-gate-summary v1", f"head: {sha}", f"verdict: {verdict}"]
        lines += [f"failed-step: {j} | {c} | gap={g}" for j, c, g in steps]
        return "\n".join(lines) + "\n\n## lane-gate summary\n"

    with tempfile.TemporaryDirectory() as t:
        repo = pathlib.Path(t)
        for cmd in (["init", "-q"], ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "a"]):
            subprocess.run(["git", "-C", t, *cmd], check=True)
        sha = git(repo, "rev-parse", "HEAD")
        out = repo / ".git" / SUMMARY_NAME
        cases = [
            ("a missing summary", None, 1, "no gate summary"),
            ("a stale summary", summary("0" * 40, "ok"), 1, "but HEAD is"),
            ("a PARTIAL summary", summary(sha, "partial"), 1, "PARTIAL"),
            ("a FAILED summary with a real failure", summary(sha, "failed", [("rust", "cargo nextest", "none")]), 1, "FAILED on rust"),
            ("a FAILED summary, one gap and one real failure", summary(sha, "failed", [("rust", "x", "git-lfs-missing"), ("web", "y", "none")]), 1, "FAILED on web"),
            ("a FAILED summary naming a gap not on the list", summary(sha, "failed", [("rust", "x", "made-up-gap")]), 1, "FAILED on rust"),
            ("a FAILED summary with no failed step", summary(sha, "failed"), 1, "gate itself broke"),
            ("an unreadable summary", "garbage\n", 1, "unreadable"),
            ("an all-passed summary", summary(sha, "ok"), 0, "ok"),
            ("a FAILED summary on the listed gaps only", summary(sha, "failed", [("rust", "x", "git-lfs-missing"), ("web", "wrangler", "wrangler-dry-run-without-dist")]), 0, "ok"),
        ]
        for what, text, code, word in cases:
            out.unlink(missing_ok=True)
            if text is not None:
                out.write_text(text)
            got, msg = check(repo)
            expect(what, (got, word in msg), (code, True))
        # A waiver passes a refused push, says so, and logs it; a blank one does not.
        out.unlink(missing_ok=True)
        log = repo / ".git" / "gate-waivers.log"
        expect("a blank waiver", check(repo, "  ")[0], 1)
        expect("no log from a refused push", log.exists(), False)
        got, msg = check(repo, "no JDK on this host")
        expect("a waiver", (got, "WAIVED (no JDK on this host)" in msg), (0, True))
        expect("the waiver is logged", log.exists() and sha in log.read_text() and "no gate summary" in log.read_text(), True)
        out.write_text(summary(sha, "ok"))
        before = log.read_text()
        check(repo, "unneeded")
        expect("a waiver on a green gate logs nothing", log.read_text(), before)
        # The real gate's output is read by this: the writer and the reader agree.
        import importlib.util
        spec = importlib.util.spec_from_file_location("lane_gate", pathlib.Path(__file__).with_name("lane-gate.py"))
        lg = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(lg)
        expect("the file name matches lane-gate's", lg.SUMMARY_NAME, SUMMARY_NAME)
        expect("every env gap lane-gate tags is on the list", {n for n, _, _ in lg.ENV_GAPS} <= GAP_NAMES, True)
        import contextlib, io
        for rows, failed, left, want_code in [
            ([("wire", "x", "pass", "1", "")], False, [], 0),
            ([("wire", "x", "pass", "1", "")], False, ["rust"], 1),
            ([("rust", "cargo nextest", "FAIL (exit 101) [env-gap: git-lfs-missing]", "1", "")], True, [], 0),
            ([("rust", "cargo nextest", "FAIL (exit 101)", "1", "")], True, [], 1),
        ]:
            with contextlib.redirect_stdout(io.StringIO()):
                lg.print_summary(rows, repo, "HEAD", "/tmp", failed, left, out)
            expect(f"lane-gate's own summary ({rows[0][2]}, left out {left})", check(repo)[0], want_code)
    for f in failures:
        print(f"gate-summary-check self-test: {f}", file=sys.stderr)
    if failures:
        return 1
    print("gate-summary-check self-test: ok")
    return 0


def main(argv):
    ap = argparse.ArgumentParser(description="Refuse a push the last lane-gate run does not cover.")
    ap.add_argument("--worktree", default=".", help="the checkout to check (default: the current one)")
    ap.add_argument("--waive", default="", metavar="REASON", help="let it through anyway; the reason is logged")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    code, msg = check(pathlib.Path(args.worktree).resolve(), args.waive)
    print(msg, file=sys.stderr if code or "WAIVED" in msg else sys.stdout)
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
