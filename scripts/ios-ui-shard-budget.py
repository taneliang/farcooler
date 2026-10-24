#!/usr/bin/env python3
"""No iOS UI shard runs within a quarter of its timeout (ov-322).

Since ov-534 the shards run nightly (nightly.yml), not on every push, so this
reads the last nightly runs on main. The per-push smoke job ("iOS UI (smoke)")
is not a shard and is ignored.

The `phone` shard hit the `ios-ui` job's 40-minute timeout twice in one day
(adc85e94, then c85bf83d), each time because new test classes had landed in
it. Each one cost a red main and about an hour, and the run said nothing
about the tests themselves: a cancelled job reports no verdict. The times in
scripts/ios-ui-shards.py are measured once, when someone rebalances, and
nothing noticed them drifting.

This reads what each shard actually took in the last completed nightly runs and
fails when one is past 75% of the timeout nightly.yml gives it. That leaves room
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
  - A skipped shard measures nothing. A skipped job reports no run time, or a
    zero one that would pass for a fast run and drag the median down. Only
    success, failure and a cancel at the timeout are read, and a job whose name
    is still the unexpanded matrix template (how GitHub lists a skipped matrix
    job) matches no shard. (Before ov-534 pushes to main skipped the shards
    when the iOS app was untouched; a nightly run never does.)
  - A job cancelled well short of its timeout was superseded by a newer push
    and measured nothing. One cancelled at the timeout counts, at the timeout.

The capture jobs ("iOS UI captures", "iOS UI captures2", "iOS UI captures3"; CAPTURE_SHARDS) are
measured the same way against the `captures` job's own timeout (ov-725).

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
WORKFLOW = ROOT / ".github/workflows/nightly.yml"
SHARDS_FILE = "scripts/ios-ui-shards.py"
REPO = "taneliang/farcooler"
LIMIT = 0.75
SMOKE_JOB = "iOS UI (smoke)"
SMOKE_WARN_S = 12 * 60  # the aim is 10 minutes (ov-534); this warns, it never fails
RUNS = 3
AT_LEAST = 2
# `iOS UI (phone)` for a shard of the `ios-ui` job; `iOS UI captures2` for a
# capture job (ov-725), whose name has no parentheses.
JOB = re.compile(r"^iOS UI (?:\((\w+)\)|(captures\d*))$")
CAPTURE_JOB = "captures"  # the nightly.yml job whose matrix runs CAPTURE_SHARDS


def timeout_minutes(workflow_text, job="ios-ui"):
    """A job's `timeout-minutes` (the `ios-ui` job's by default), read from nightly.yml."""
    block = re.search(rf"^  {re.escape(job)}:\n(.*?)(?=^  [\w-]+:\n|\Z)", workflow_text, re.M | re.S)
    if not block:
        return None
    found = re.search(r"^    timeout-minutes:\s*(\d+)\s*$", block.group(1), re.M)
    return int(found.group(1)) if found else None


def shards_in(source):
    """SHARDS and CAPTURE_SHARDS from a version of scripts/ios-ui-shards.py, without running it."""
    found = {}
    for node in ast.parse(source).body:
        if isinstance(node, ast.Assign) and any(
            isinstance(t, ast.Name) and t.id in ("SHARDS", "CAPTURE_SHARDS") for t in node.targets
        ):
            found.update({k: set(v) for k, v in ast.literal_eval(node.value).items()})
    return found


def timeouts(workflow_text, names):
    """Seconds of timeout for each shard: the capture jobs' own, the `ios-ui` job's for the rest.

    None if a needed job has no timeout-minutes."""
    ui, cap = timeout_minutes(workflow_text), timeout_minutes(workflow_text, CAPTURE_JOB)
    out = {}
    for name in names:
        minutes = cap if name.startswith("captures") else ui
        if minutes is None:
            return None
        out[name] = minutes * 60
    return out


def limit_of(timeout_s, name):
    """`timeout_s` is one number for every shard, or a {shard: seconds} map."""
    return timeout_s[name] if isinstance(timeout_s, dict) else timeout_s


def seconds(job):
    from datetime import datetime

    def at(stamp):
        return datetime.fromisoformat(stamp.replace("Z", "+00:00"))

    return (at(job["completed_at"]) - at(job["started_at"])).total_seconds()


def measure(runs, current, timeout_s_all):
    """Each shard's comparable times, newest first.

    `runs` is newest first: [{"shards": {name: set}, "jobs": [job, ...]}].
    """
    times = {name: [] for name in current}
    for run in runs:
        for job in run["jobs"]:
            m = JOB.match(job.get("name", ""))
            name = m and (m.group(1) or m.group(2))
            if not name or name not in current:
                continue
            timeout_s = limit_of(timeout_s_all, name)
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


def verdict(times, timeout_s_all):
    """Lines to print, and whether any shard is over the limit."""
    lines, over = [], False
    for name, took in times.items():
        timeout_s = limit_of(timeout_s_all, name)
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
    longest = max(timeout_s.values()) if isinstance(timeout_s, dict) else timeout_s
    print("ios-ui-shard-budget: each shard's job time in main's last runs")
    print("\n".join(lines))
    if over:
        print(
            f"::error::An iOS UI shard is past {LIMIT:.0%} of its job's "
            f"timeout (up to {longest / 60:.0f} minutes). Move classes by their measured "
            "seconds in scripts/ios-ui-shards.py (or add a shard to it and to the "
            "nightly.yml matrix; for captures, CAPTURE_SHARDS) before the next class lands and the shard is cancelled "
            "with no verdict."
        )
        return 1
    print("ios-ui-shard-budget: ok")
    return 0


def smoke_times(job_lists):
    """Seconds the per-push smoke job took, newest run first, at most RUNS of them.

    `job_lists` is each of ci.yml's recent runs' jobs, newest first. A skipped
    job (no iOS change) or a cancelled one (superseded by the next push)
    measured nothing."""
    times = []
    for jobs in job_lists:
        for job in jobs:
            if job.get("name") == SMOKE_JOB and job.get("conclusion") in ("success", "failure"):
                if job.get("started_at") and job.get("completed_at"):
                    times.append(seconds(job))
    return times[:RUNS]


def smoke_verdict(times):
    """(line, warn): the smoke job's median against SMOKE_WARN_S."""
    if not times:
        return "  smoke: not measured yet", False
    median = statistics.median(times)
    line = f"  smoke: {median / 60:.1f} min (runs: {', '.join(f'{t / 60:.1f}' for t in times)})"
    if len(times) >= AT_LEAST and median > SMOKE_WARN_S:
        return line + f"  <- over {SMOKE_WARN_S // 60} min", True
    return line, False


def smoke_report():
    """Print the smoke job's time from ci.yml's last runs; a slow one is a warning."""
    try:
        listed = gh_json(
            "run", "list", "-R", REPO, "--workflow", "ci.yml", "--branch", "main",
            "--status", "completed", "-L", str(RUNS * 6), "--json", "databaseId",
        )
        lists = [
            gh_json("api", f"repos/{REPO}/actions/runs/{r['databaseId']}/jobs?per_page=100")["jobs"]
            for r in listed
        ]
    except RuntimeError as error:
        print(f"::warning::The smoke job's time could not be read: {error}")
        return
    line, warn = smoke_verdict(smoke_times(lists))
    print("ios-ui-shard-budget: the per-push smoke job, in ci.yml's last runs")
    print(line)
    if warn:
        print(
            f"::warning::The iOS UI smoke job is past {SMOKE_WARN_S // 60} minutes. Trim SMOKE in "
            "scripts/ios-ui-shards.py (its seconds are claims; this is the measurement)."
        )


def git(*args):
    return subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True, text=True)


def gh_json(*args):
    # GitHub's API answers an occasional 502 or 503 that the next try gets past
    # (Oct 13: three gates and a main run failed on one); anything else fails at once.
    import time
    for attempt in range(4):
        out = subprocess.run(["gh", *args], capture_output=True, text=True)
        if out.returncode == 0:
            return json.loads(out.stdout)
        if not re.search(r"HTTP 50[234]", out.stderr) or attempt == 3:
            raise RuntimeError(f"gh {' '.join(args)}: {out.stderr.strip()}")
        time.sleep(5 * (attempt + 1))


def fetch(count):
    listed = gh_json(
        "run", "list", "-R", REPO, "--workflow", "nightly.yml", "--branch", "main",
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
    other = [run([job("b", 35), {"name": "Swift (app build + bundle checks)", "conclusion": "success",
                                 "started_at": "2026-10-05T00:00:00Z",
                                 "completed_at": "2026-10-05T00:50:00Z"}], {"a": {"X"}})] * 3
    expect("a new shard is not measured yet", measure(other, now, timeout)["b"], [])
    # The timeout is the ios-ui job's, not a neighbor's.
    flow = "jobs:\n  ios:\n    timeout-minutes: 15\n  ios-ui:\n    name: x\n    timeout-minutes: 40\n  android:\n    timeout-minutes: 25\n"
    expect("the ios-ui timeout is read", timeout_minutes(flow), 40)
    expect("no ios-ui job, no timeout", timeout_minutes("jobs:\n  ios:\n    timeout-minutes: 15\n"), None)
    # The smoke job (ci.yml): only its own finished runs count.
    def smoke(minutes, conclusion="success"):
        return {**job("x", minutes, conclusion), "name": SMOKE_JOB}

    slow = smoke_times([[smoke(14)], [smoke(13)], [smoke(15)]])
    expect("a slow smoke job warns", smoke_verdict(slow)[1], True)
    expect("a 10-minute smoke job does not", smoke_verdict(smoke_times([[smoke(10)]] * 3))[1], False)
    expect("one slow run is not judged", smoke_verdict(smoke_times([[smoke(20)]]))[1], False)
    expect("cancelled and skipped smoke runs measure nothing",
           smoke_times([[smoke(5, "cancelled")], [{"name": SMOKE_JOB, "conclusion": "skipped"}]]), [])
    expect("a shard is not the smoke job", smoke_times([[job("a", 30)]]), [])
    # The capture jobs (ov-725): "iOS UI captures2" has no parentheses, and its
    # timeout is the `captures` job's, not the ios-ui job's.
    def cjob(name, minutes, conclusion="success"):
        return {**job("x", minutes, conclusion), "name": f"iOS UI {name}"}

    cnow = {"phone": {"X"}, "captures": {"C"}, "captures2": {"D"}}
    ctimeout = {"phone": 40 * 60, "captures": 45 * 60, "captures2": 45 * 60}

    def crun(jobs):
        return {"shards": cnow, "jobs": jobs}

    over = [crun([cjob("captures", 35), cjob("captures2", 30)])] * 3  # 75% of 45 is 33.75
    got = measure(over, cnow, ctimeout)
    expect("captures time is read", got["captures"], [2100.0] * 3)
    expect("captures2 time is read", got["captures2"], [1800.0] * 3)
    expect("a captures job over 75% fails", verdict(got, ctimeout)[1], True)
    under = [crun([cjob("captures", 33), cjob("captures2", 30)])] * 3
    expect("captures jobs under 75% pass", verdict(measure(under, cnow, ctimeout), ctimeout)[1], False)
    expect("captures2 alone over fails",
           verdict(measure([crun([cjob("captures2", 34)])] * 3, cnow, ctimeout), ctimeout)[1], True)
    expect("a capture cancel at its timeout counts at 45 min",
           measure([crun([cjob("captures", 45, "cancelled")])], cnow, ctimeout)["captures"], [2700.0])
    cflow = "jobs:\n  ios-ui:\n    timeout-minutes: 40\n  captures:\n    name: x\n    timeout-minutes: 45\n"
    expect("per-shard timeouts", timeouts(cflow, ["phone", "captures2"]), {"phone": 2400, "captures2": 2700})
    expect("no captures timeout, no map", timeouts("jobs:\n  ios-ui:\n    timeout-minutes: 40\n", ["captures"]), None)
    expect("CAPTURE_SHARDS is read too", shards_in('SHARDS = {"a": ["X"]}\nCAPTURE_SHARDS = {"captures": ["C"]}\n'),
           {"a": {"X"}, "captures": {"C"}})
    expect("the real files give captures a timeout",
           bool(timeouts(WORKFLOW.read_text(), shards_in((ROOT / SHARDS_FILE).read_text()))), True)
    # SHARDS is read from a source with comments in the literal.
    src = 'SHARDS = {\n    "a": [\n        "X",  # 10 s\n    ],\n}\nOTHER = 1\n'
    expect("SHARDS is read from source", shards_in(src), {"a": {"X"}})
    # And the real files agree with the reader.
    expect("nightly.yml has an ios-ui timeout", timeout_minutes(WORKFLOW.read_text()) is not None, True)
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
    current = shards_in((ROOT / SHARDS_FILE).read_text())
    limits = timeouts(WORKFLOW.read_text(), current)
    if limits is None:
        print("ios-ui-shard-budget: no timeout-minutes on the ios-ui or captures job in nightly.yml", file=sys.stderr)
        return 2
    try:
        runs = fetch(RUNS * 6)
    except RuntimeError as error:
        if "not found on the default branch" in str(error):
            # nightly.yml has not landed on main yet, so there are no runs to read.
            # A pass, but loud: a renamed nightly.yml would otherwise turn the
            # shard budget off for good without anyone seeing.
            print("::warning::ios-ui-shard-budget: nightly.yml is not on main (not landed yet, or renamed); "
                  "no shard was measured")
            smoke_report()
            return 0
        print(f"ios-ui-shard-budget: {error}", file=sys.stderr)
        return 2
    code = check(runs, current, limits)
    smoke_report()
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
