#!/usr/bin/env python3
"""No iOS UI shard runs within a quarter of its timeout (ov-322).

The `phone` shard hit the `ios-ui` job's 40-minute timeout twice in one day
(adc85e94, then c85bf83d), each time because new test classes had landed in
it. Each one cost a red main and about an hour, and the run said nothing
about the tests themselves: a cancelled job reports no verdict. The times in
scripts/ios-ui-shards.py are measured once, when someone rebalances, and
nothing noticed them drifting.

This reads what each shard actually took in main's last completed CI runs and
fails when one is past 75% of the timeout ci.yml gives it. That leaves room
for the next class or two, and tells whoever lands the next one to rebalance
first.

  - A shard's time is its job's run time, from start to finish, setup
    included: that is what the timeout measures.
  - It is the median of the last three runs that measured the shard as it is
    now, and it needs at least two, so one slow runner (the same class has
    taken 276 s and 442 s) does not trip it alone.
  - "As it is now": a run counts for a shard only when that shard's classes
    at the run's commit are a subset of its classes today. Moving a class
    out (a rebalance) makes the old times overstate it, so they no longer
    count, and the check waits for main to measure the new split. Adding a
    class keeps the old times, as a floor, until the next run measures it.
  - A skipped shard measures nothing. Since ov-301 a push to main whose changes
    do not touch the iOS app skips all five shards (scripts/ios-ui-plan.py), and
    a skipped job reports no run time, or a zero one that would pass for a fast
    run and drag the median down. Only success, failure and a cancel at the
    timeout are read, and a job whose name is still the unexpanded matrix
    template (how GitHub lists a skipped matrix job) matches no shard. Such runs
    still use up the window of runs read, so it is wider than it was.
  - A job cancelled well short of its timeout was superseded by a newer push
    and measured nothing. One cancelled at the timeout counts, at the timeout.

  ./scripts/ios-ui-shard-budget.py              main's last runs, via `gh`
  ./scripts/ios-ui-shard-budget.py --self-test  the rule against planted runs

It needs the full history (`fetch-depth: 0`) to read each run's shard lists,
and `gh` with a token that can read Actions.
"""

import ast
import json
import pathlib
import re
import statistics
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/ci.yml"
SHARDS_FILE = "scripts/ios-ui-shards.py"
REPO = "taneliang/farcooler"
LIMIT = 0.75
RUNS = 3
AT_LEAST = 2
JOB = re.compile(r"^iOS UI \((\w+)\)$")


def timeout_minutes(workflow_text):
    """The `ios-ui` job's `timeout-minutes`, read from ci.yml."""
    block = re.search(r"^  ios-ui:\n(.*?)(?=^  [\w-]+:\n|\Z)", workflow_text, re.M | re.S)
    if not block:
        return None
    found = re.search(r"^    timeout-minutes:\s*(\d+)\s*$", block.group(1), re.M)
    return int(found.group(1)) if found else None


def shards_in(source):
    """SHARDS from a version of scripts/ios-ui-shards.py, without running it."""
    for node in ast.parse(source).body:
        if isinstance(node, ast.Assign) and any(
            isinstance(t, ast.Name) and t.id == "SHARDS" for t in node.targets
        ):
            return {k: set(v) for k, v in ast.literal_eval(node.value).items()}
    return {}


def seconds(job):
    from datetime import datetime

    def at(stamp):
        return datetime.fromisoformat(stamp.replace("Z", "+00:00"))

    return (at(job["completed_at"]) - at(job["started_at"])).total_seconds()


def measure(runs, current, timeout_s):
    """Each shard's comparable times, newest first.

    `runs` is newest first: [{"shards": {name: set}, "jobs": [job, ...]}].
    """
    times = {name: [] for name in current}
    for run in runs:
        for job in run["jobs"]:
            m = JOB.match(job.get("name", ""))
            if not m or m.group(1) not in current:
                continue
            name = m.group(1)
            if not job.get("started_at") or not job.get("completed_at"):
                continue
            if job.get("conclusion") not in ("success", "failure", "cancelled"):
                continue
            then = run["shards"].get(name)
            if then is None or not then <= current[name]:
                continue
            took = seconds(job)
            if job["conclusion"] == "cancelled":
                if took < 0.95 * timeout_s:
                    continue
                took = max(took, timeout_s)
            if len(times[name]) < RUNS:
                times[name].append(took)
    return times


def verdict(times, timeout_s):
    """Lines to print, and whether any shard is over the limit."""
    lines, over = [], False
    for name, took in times.items():
        if not took:
            lines.append(f"  {name}: not measured as it is now yet")
            continue
        median = statistics.median(took)
        share = median / timeout_s
        runs = ", ".join(f"{t / 60:.1f}" for t in took)
        line = f"  {name}: {median / 60:.1f} min, {share:.0%} of {timeout_s / 60:.0f} (runs: {runs})"
        if len(took) < AT_LEAST:
            line += f"  (one run; judged from {AT_LEAST})"
        elif share > LIMIT:
            over = True
            line += "  <- over 75%"
        lines.append(line)
    return lines, over


def check(runs, current, timeout_s):
    lines, over = verdict(measure(runs, current, timeout_s), timeout_s)
    print("ios-ui-shard-budget: each shard's job time in main's last runs")
    print("\n".join(lines))
    if over:
        print(
            f"::error::An iOS UI shard is past {LIMIT:.0%} of the ios-ui job's "
            f"{timeout_s / 60:.0f}-minute timeout. Move classes by their measured "
            "seconds in scripts/ios-ui-shards.py (or add a shard to it and to the "
            "ci.yml matrix) before the next class lands and the shard is cancelled "
            "with no verdict."
        )
        return 1
    print("ios-ui-shard-budget: ok")
    return 0


def git(*args):
    return subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True, text=True)


def gh_json(*args):
    out = subprocess.run(["gh", *args], capture_output=True, text=True)
    if out.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args)}: {out.stderr.strip()}")
    return json.loads(out.stdout)


def fetch(count):
    listed = gh_json(
        "run", "list", "-R", REPO, "--workflow", "ci.yml", "--branch", "main",
        "--status", "completed", "-L", str(count), "--json", "databaseId,headSha",
    )
    runs = []
    for run in listed:
        sha = run["headSha"]
        if git("cat-file", "-e", f"{sha}^{{commit}}").returncode != 0:
            raise RuntimeError(
                f"commit {sha[:8]} is not in this checkout; it needs fetch-depth: 0"
            )
        source = git("show", f"{sha}:{SHARDS_FILE}")
        shards = shards_in(source.stdout) if source.returncode == 0 else {}
        jobs = gh_json(
            "api", f"repos/{REPO}/actions/runs/{run['databaseId']}/jobs?per_page=100"
        )["jobs"]
        runs.append({"shards": shards, "jobs": jobs})
    return runs


def self_test():
    failures = []

    def expect(what, got, want):
        if got != want:
            failures.append(f"{what}: got {got!r}, want {want!r}")

    def job(name, minutes, conclusion="success"):
        return {
            "name": f"iOS UI ({name})",
            "conclusion": conclusion,
            "started_at": "2026-10-05T00:00:00Z",
            "completed_at": f"2026-10-05T00:{int(minutes):02d}:{int(minutes % 1 * 60):02d}Z",
        }

    timeout = 40 * 60
    now = {"a": {"X", "Y"}, "b": {"Z"}}

    def run(jobs, shards=None):
        return {"shards": shards or now, "jobs": jobs}

    # A planted slow shard: `b` at 31 minutes, past 30 (75% of 40).
    slow = [run([job("a", 20), job("b", 31)])] * 3
    expect("a planted slow shard fails", verdict(measure(slow, now, timeout), timeout)[1], True)
    # 30 minutes exactly is at the limit, not past it.
    edge = [run([job("a", 20), job("b", 30)])] * 3
    expect("75% exactly passes", verdict(measure(edge, now, timeout), timeout)[1], False)
    # One run alone is not enough to judge, however slow; two are.
    one = [run([job("b", 35)])]
    expect("a single slow run is not judged", verdict(measure(one, now, timeout), timeout)[1], False)
    two = [run([job("b", 35)])] * 2
    expect("two slow runs are", verdict(measure(two, now, timeout), timeout)[1], True)
    # One slow runner among three is the median's job to ignore.
    once = [run([job("b", 35)]), run([job("b", 20)]), run([job("b", 21)])]
    expect("one slow run is outvoted", verdict(measure(once, now, timeout), timeout)[1], False)
    # Only the last three comparable runs count.
    old = [run([job("b", 20)])] * 3 + [run([job("b", 39)])] * 3
    expect("a fourth run back is not read", measure(old, now, timeout)["b"], [1200.0] * 3)
    # A superseded cancel measured nothing; a cancel at the timeout counts.
    sup = [run([job("b", 5, "cancelled")]), run([job("b", 32)]), run([job("b", 32)])]
    expect("a superseded cancel is skipped", measure(sup, now, timeout)["b"], [1920.0, 1920.0])
    hit = [run([job("b", 40, "cancelled")])] * 2 + [run([job("b", 20)])]
    expect("a timeout counts as the timeout", verdict(measure(hit, now, timeout), timeout)[1], True)
    # Skipped shards (ov-301): a push that skips them lists the jobs as
    # skipped, with no times or a zero-length run, and must read as nothing.
    skipped = {"conclusion": "skipped", "started_at": "2026-10-05T00:00:00Z", "completed_at": "2026-10-05T00:00:00Z"}
    zero = [run([{**skipped, "name": "iOS UI (b)"}])] * 3
    expect("skipped shards measure nothing", measure(zero, now, timeout)["b"], [])
    unexpanded = [run([{**skipped, "name": "iOS UI (${{ matrix.shard }})"}, {"name": "iOS UI (b)", "conclusion": "skipped"}])] * 3
    expect("a skipped matrix job is not a shard", measure(unexpanded, now, timeout)["b"], [])
    # They do not pull a slow shard's median down: two slow runs among skipped
    # ones still judge, and skipped runs are not counted as fast ones.
    mixed = [run([{**skipped, "name": "iOS UI (b)"}]), run([job("b", 35)]), run([{**skipped, "name": "iOS UI (b)"}]), run([job("b", 35)])]
    expect("skipped runs do not dilute a slow shard", verdict(measure(mixed, now, timeout), timeout)[1], True)
    # A shard whose classes moved out since: its old time is not its time now.
    moved = [run([job("a", 35)], {"a": {"X", "Y", "W"}, "b": {"Z"}})] * 3
    expect("a rebalanced shard waits for a new time", measure(moved, now, timeout)["a"], [])
    # A class added since: the old time still counts, as a floor.
    grown = [run([job("a", 35)], {"a": {"X"}, "b": {"Z"}})] * 3
    expect("an added class keeps the old time", verdict(measure(grown, now, timeout), timeout)[1], True)
    # A shard that did not exist at the run's commit, and jobs that aren't shards.
    other = [run([job("b", 35), {"name": "Swift (shared + macOS)", "conclusion": "success",
                                 "started_at": "2026-10-05T00:00:00Z",
                                 "completed_at": "2026-10-05T00:50:00Z"}], {"a": {"X"}})] * 3
    expect("a new shard is not measured yet", measure(other, now, timeout)["b"], [])
    # The timeout is the ios-ui job's, not a neighbor's.
    flow = "jobs:\n  ios:\n    timeout-minutes: 15\n  ios-ui:\n    name: x\n    timeout-minutes: 40\n  android:\n    timeout-minutes: 25\n"
    expect("the ios-ui timeout is read", timeout_minutes(flow), 40)
    expect("no ios-ui job, no timeout", timeout_minutes("jobs:\n  ios:\n    timeout-minutes: 15\n"), None)
    # SHARDS is read from a source with comments in the literal.
    src = 'SHARDS = {\n    "a": [\n        "X",  # 10 s\n    ],\n}\nOTHER = 1\n'
    expect("SHARDS is read from source", shards_in(src), {"a": {"X"}})
    # And the real files agree with the reader.
    expect("ci.yml has an ios-ui timeout", timeout_minutes(WORKFLOW.read_text()) is not None, True)
    expect("the shard file parses", bool(shards_in((ROOT / SHARDS_FILE).read_text())), True)

    for failure in failures:
        print(f"ios-ui-shard-budget self-test: {failure}", file=sys.stderr)
    if failures:
        return 1
    print("ios-ui-shard-budget self-test: ok")
    return 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    if argv:
        print(__doc__, file=sys.stderr)
        return 2
    minutes = timeout_minutes(WORKFLOW.read_text())
    if minutes is None:
        print("ios-ui-shard-budget: no timeout-minutes on the ios-ui job in ci.yml", file=sys.stderr)
        return 2
    current = shards_in((ROOT / SHARDS_FILE).read_text())
    try:
        runs = fetch(RUNS * 6)
    except RuntimeError as error:
        print(f"ios-ui-shard-budget: {error}", file=sys.stderr)
        return 2
    return check(runs, current, minutes * 60)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
