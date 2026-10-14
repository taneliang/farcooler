#!/usr/bin/env python3
"""Tell someone when the nightly iOS UI run goes red (ov-534).

nightly.yml's last job runs this, whatever the other jobs did. It reads the
run's own jobs, and:

  - a night with a failed job opens ONE GitHub issue, labeled `nightly-ui-red`,
    or, if one is already open, adds a comment to it. The issue names the failing
    test classes and methods (read from each failed job's log) and any job that
    ended without a verdict (a timeout, a cancel, a build that did not finish),
    and lists the commits since the last green nightly;
  - a green night closes the open issue, saying which commit was green.

Why an issue, not a board card (the owner asked for a card): the board is
`farcooler-canary`, a local tool talking to the owner's daemon, and a GitHub
runner has no way to reach it. Nothing else in this repo reports a scheduled
failure (claude-prices.yml just goes red), so there was no notifier to join.
The issue is the hook a coordinator can turn into a card: it names the commits.

    scripts/nightly-report.py               # in CI; needs GH_TOKEN, GITHUB_REPOSITORY,
                                            # GITHUB_RUN_ID and GITHUB_SHA, and a full clone
    scripts/nightly-report.py --self-test   # the parsing and the decisions, with a fake gh

It needs `issues: write` and `actions: read`, and `fetch-depth: 0` for the
commit range.
"""

import json
import os
import re
import subprocess
import sys

LABEL = "nightly-ui-red"
WORKFLOW = "nightly.yml"
OWN_JOB = "Report"
FAILED = re.compile(r"Test Case '-\[FarCoolerUITests\.(\w+) (\w+)\]' failed")
BAD = ("failure", "cancelled", "timed_out")
LOG_LIMIT = 40  # commits listed
BUDGET_JOB = "iOS UI shard budget"


def failing_tests(log):
    """{class: [methods]} from an xcodebuild log, in order, each once."""
    found = {}
    for cls, method in FAILED.findall(log):
        methods = found.setdefault(cls, [])
        if method not in methods:
            methods.append(method)
    return found


def classify(jobs):
    """The jobs of this run that did not pass, apart from the report job."""
    return [j for j in jobs if j.get("conclusion") in BAD and j.get("name") != OWN_JOB]


def body(run_url, sha, when, failures, silent, commits, previous, more=0, budget=False):
    """The issue text, short paragraphs and bullets."""
    lines = [f"The nightly iOS UI run on `{sha[:9]}` ({when}) went red: {run_url}", ""]
    if failures:
        lines += ["## Failing tests", ""]
        for job, classes in failures:
            for cls, methods in classes.items():
                lines.append(f"- `{cls}` ({job}): " + ", ".join(f"`{m}`" for m in methods))
        lines.append("")
    if silent:
        lines += ["## Jobs that ended without a test verdict", ""]
        lines += [f"- `{name}` ended {how}" for name, how in silent]
        lines += [
            "",
            "A timeout or a cancel names no test. Open the job's log; a shard past 75% of its "
            "timeout is the shard budget's warning (scripts/ios-ui-shard-budget.py).",
            "",
        ]
    if budget:
        lines += [
            "## A shard is close to its timeout", "",
            "The shard budget job went red: a shard took more than 75% of its job timeout. "
            "Move classes between shards by their measured seconds in scripts/ios-ui-shards.py "
            "before the next class lands and the shard is cancelled with no verdict. Its log "
            "names the shard.", "",
        ]
    if previous:
        lines += [f"## Commits since the last green night (`{previous[:9]}`)", ""]
    else:
        lines += ["## Commits", "", "No earlier green nightly was found, so these are the latest on main.", ""]
    lines += [f"- {c}" for c in commits] or ["- none: main has not moved since the last green night"]
    if more > 0:
        lines.append(f"- and {more} more")
    lines.append("")
    return "\n".join(lines)


def plan(env, jobs, logs, previous, commits, open_issues, more=0):
    """What to do, as a list of ("create"|"comment"|"close", payload) steps.

    `logs` maps a job's id to its log text, `previous` is the last green
    nightly's commit or None, `commits` the one-line subjects since then."""
    failed = classify(jobs)
    run_url = f"https://github.com/{env['GITHUB_REPOSITORY']}/actions/runs/{env['GITHUB_RUN_ID']}"
    sha = env["GITHUB_SHA"]
    if not failed:
        if open_issues:
            text = f"Green again: the nightly run on `{sha[:9]}` passed: {run_url}"
            return [("close", (open_issues[0], text))]
        return []
    failures, silent, budget = [], [], False
    for job in failed:
        if job["name"] == BUDGET_JOB:
            budget = True
            continue
        classes = failing_tests(logs.get(job["id"], ""))
        if classes:
            failures.append((job["name"], classes))
        else:
            silent.append((job["name"], "as " + ("cancelled (a timeout looks the same)" if job["conclusion"] == "cancelled" else job["conclusion"])))
    when = (env.get("NIGHTLY_DATE") or "")
    text = body(run_url, sha, when or "tonight", failures, silent, commits, previous, more, budget)
    if open_issues:
        return [("comment", (open_issues[0], text))]
    names = sorted({cls for _, classes in failures for cls in classes})
    head = f"Nightly iOS UI: {len(names)} failing" if names else "Nightly iOS UI: a job did not finish"
    if not names and not silent and budget:
        head = "Nightly iOS UI: a shard is close to its timeout"
    if len(names) == 1:
        head = f"Nightly iOS UI: {names[0]} failing"
    return [("create", (head, text))]


def sh(*args):
    out = subprocess.run(args, capture_output=True, text=True)
    if out.returncode != 0:
        raise RuntimeError(f"{' '.join(args[:4])}: {out.stderr.strip()[:300]}")
    return out.stdout


def gh_json(*args):
    return json.loads(sh("gh", *args))


def run_live():
    env = os.environ
    for key in ("GH_TOKEN", "GITHUB_REPOSITORY", "GITHUB_RUN_ID", "GITHUB_SHA"):
        if not env.get(key):
            print(f"nightly-report: {key} is not set", file=sys.stderr)
            return 2
    repo, run_id, sha = env["GITHUB_REPOSITORY"], env["GITHUB_RUN_ID"], env["GITHUB_SHA"]
    jobs = gh_json("api", f"repos/{repo}/actions/runs/{run_id}/jobs?per_page=100")["jobs"]
    logs = {}
    for job in classify(jobs):
        try:
            logs[job["id"]] = sh("gh", "api", f"repos/{repo}/actions/jobs/{job['id']}/logs")
        except RuntimeError as error:
            print(f"nightly-report: no log for {job['name']}: {error}", file=sys.stderr)
    previous, commits, more = None, [], 0
    runs = gh_json(
        "api",
        f"repos/{repo}/actions/workflows/{WORKFLOW}/runs?branch=main&status=success&event=schedule&per_page=1",
    )["workflow_runs"]
    if runs:
        previous = runs[0]["head_sha"]
    try:
        args = ["git", "log", "--format=%h %s", f"-{LOG_LIMIT}", f"{previous}..{sha}" if previous else sha]
        commits = [c for c in sh(*args).splitlines() if c]
        if previous and len(commits) >= LOG_LIMIT:
            more = int(sh("git", "rev-list", "--count", f"{previous}..{sha}").strip()) - len(commits)
    except RuntimeError as error:
        print(f"nightly-report: no commit range: {error}", file=sys.stderr)
        previous = None
    open_issues = [
        i["number"]
        for i in gh_json("issue", "list", "-R", repo, "--label", LABEL, "--state", "open", "--json", "number")
    ]
    steps = plan(
        {**env, "NIGHTLY_DATE": sh("date", "-u", "+%Y-%m-%d").strip()},
        jobs, logs, previous, commits, open_issues, more,
    )
    if not steps:
        print("nightly-report: green, and no issue is open")
        return 0
    for what, payload in steps:
        if what == "create":
            sh("gh", "label", "create", LABEL, "-R", repo, "--color", "B60205", "--force",
               "--description", "The nightly iOS UI run is red")
            title, text = payload
            print(sh("gh", "issue", "create", "-R", repo, "--label", LABEL, "--title", title, "--body", text))
        elif what == "comment":
            number, text = payload
            sh("gh", "issue", "comment", str(number), "-R", repo, "--body", text)
            print(f"nightly-report: commented on #{number}")
        elif what == "close":
            number, text = payload
            sh("gh", "issue", "close", str(number), "-R", repo, "--comment", text)
            print(f"nightly-report: closed #{number}")
    return 0


def self_test():
    failures = []

    def expect(what, got, want):
        if got != want:
            failures.append(f"{what}: got {got!r}, want {want!r}")

    log = (
        "2026-10-15T11:40:01Z Test Case '-[FarCoolerUITests.PlanUITests testA]' failed (3.1 seconds).\n"
        "2026-10-15T11:41:01Z Test Case '-[FarCoolerUITests.PlanUITests testB]' failed (3.1 seconds).\n"
        "2026-10-15T11:41:02Z Test Case '-[FarCoolerUITests.PlanUITests testB]' failed (3.1 seconds).\n"
        "2026-10-15T11:42:01Z Test Case '-[FarCoolerUITests.TaskScreenTests testC]' passed (3.1 seconds).\n"
        "2026-10-15T11:43:01Z Test Case '-[FarCoolerUITests.ActionFailureTests testD]' failed (3.1 seconds).\n"
    )
    expect("failures are read, once each, and passes are not",
           failing_tests(log), {"PlanUITests": ["testA", "testB"], "ActionFailureTests": ["testD"]})
    expect("no failures in a clean log", failing_tests("Test Case '-[FarCoolerUITests.X y]' passed"), {})

    env = {"GITHUB_REPOSITORY": "o/r", "GITHUB_RUN_ID": "7", "GITHUB_SHA": "a" * 40}
    job = lambda i, name, conclusion: {"id": i, "name": name, "conclusion": conclusion}
    red = [job(1, "iOS UI (phone)", "failure"), job(2, "iOS UI (shell)", "success"),
           job(3, "iOS UI (agent)", "cancelled"), job(4, OWN_JOB, "failure")]
    steps = plan(env, red, {1: log}, "b" * 40, ["abc123 one", "def456 two"], [])
    expect("a red night opens one issue", [s[0] for s in steps], ["create"])
    title, text = steps[0][1]
    expect("the title names the failing classes' count", title, "Nightly iOS UI: 2 failing")
    for needle in ("`PlanUITests` (iOS UI (phone)): `testA`, `testB`", "`ActionFailureTests`",
                   "iOS UI (agent)` ended as cancelled", "abc123 one", "since the last green night (`bbbbbbbbb`)"):
        expect(f"the body has {needle!r}", needle in text, True)
    expect("the report job is not blamed", "Report" in text, False)
    again = plan(env, red, {1: log}, "b" * 40, [], [42])
    expect("a second red night comments on the open issue", [(s[0], s[1][0]) for s in again], [("comment", 42)])
    expect("an empty range is said", "main has not moved" in again[0][1][1], True)
    green = [job(1, "iOS UI (phone)", "success"), job(4, OWN_JOB, "failure")]
    expect("a green night with no issue does nothing", plan(env, green, {}, None, [], []), [])
    closed = plan(env, green, {}, None, [], [42])
    expect("a green night closes the open issue", [(s[0], s[1][0]) for s in closed], [("close", 42)])
    skipped = [job(1, "iOS UI (phone)", "skipped"), job(2, "iOS", "success")]
    expect("a skipped job is not a failure", plan(env, skipped, {}, None, [], []), [])
    one = plan(env, [job(1, "iOS UI (phone)", "failure")], {1: "Test Case '-[FarCoolerUITests.PlanUITests testA]' failed"}, None, [], [])
    expect("one class is named in the title", one[0][1][0], "Nightly iOS UI: PlanUITests failing")
    expect("no earlier green night is said", "No earlier green nightly" in one[0][1][1], True)
    many = plan(env, red, {1: log}, "b" * 40, ["abc one"], [], 12)
    expect("a long range says how many more", "- and 12 more" in many[0][1][1], True)
    over = plan(env, [job(1, BUDGET_JOB, "failure")], {}, None, [], [])
    expect("a budget overrun is not 'a job did not finish'", over[0][1][0], "Nightly iOS UI: a shard is close to its timeout")
    expect("and the body says what to do", "Move classes between shards" in over[0][1][1], True)
    build = plan(env, [job(1, "iOS", "failure")], {1: "no tests ran"}, None, [], [])
    expect("a build failure files an issue naming the job", ("`iOS` ended as failure" in build[0][1][1], build[0][1][0]),
           (True, "Nightly iOS UI: a job did not finish"))

    for failure in failures:
        print(f"nightly-report self-test: {failure}", file=sys.stderr)
    if failures:
        return 1
    print("nightly-report self-test: ok")
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        sys.exit(self_test())
    if sys.argv[1:]:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    sys.exit(run_live())
