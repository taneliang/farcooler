#!/usr/bin/env python3
"""Which tests fail on main's CI, worst first? (ov-697)

    scripts/ci-flake-tally.sh [--runs 30] [--branch main] [--workflow ci.yml] [--repo OWNER/NAME]
    scripts/ci-flake-tally.sh --parse-log FILE [--conclusion failure] [--annotations FILE]
    scripts/ci-flake-tally.sh --report MANIFEST

Reads the last N runs of the CI workflow on main (gh run list), then EVERY
attempt of each run, not just the last: `gh run list` reports only a run's
latest attempt, so a flake that failed once and went green on "Re-run failed
jobs" shows as success there and would never be counted. For each failed job
attempt it reads the failed log (gh run view --job ID --log-failed) and pulls
out the failing tests:

    nextest        FAIL [  7.4s] (3481/5333) crate path::to::test
    swift-testing  ✘ Test "name" failed after 46.8 seconds
    XCTest         the `Failing tests:` list, or Test Case '-[Suite test]' failed
    Gradle         pkg.FooTest > name FAILED
    leak checks    tmux-leak-check: LEAKED farcoolerd 77743

A job attempt that failed with no test in its log is an infrastructure failure
(runner not acquired or lost, DNS or network errors, a timeout, a full disk, a
cancelled job), found from the log text and the job's check-run annotations.
Those are listed apart from the tests. A failure that is neither is listed as
unclassified, never dropped.

Read-only against GitHub: gh run list, gh run view and GET api calls.
--parse-log and --report work from saved files, for scripts/ci-flake-tally-test.sh.
"""
import argparse
import collections
import json
import re
import subprocess
import sys

# `gh run view --log-failed` prints ESC as the two characters ^[ on these
# runners, so both spellings go.
ANSI = re.compile(r"(?:\x1b|\^\[)\[[0-9;]*m")
TS = re.compile(r"^﻿?\d{4}-\d\d-\d\dT[\d:.]+Z ?")

NEXTEST = re.compile(
    r"^(?:TRY \d+ )?(?:FAIL|SIGABRT|SIGSEGV|SIGBUS|TIMEOUT|EXITCODE\S*)\s+\[\s*[\d.]+s\]\s+"
    r"(?:\(\s*[\d/? ]+\)\s+)?(\S+)\s+(\S+)\s*$"
)
CARGO_TEST = re.compile(r"^\s*test (\S+) \.\.\. FAILED\s*$")
PANICKED = re.compile(r"thread '([^']+)'.* panicked at")
SWIFT = re.compile(r'^✘ Test "(.+?)" (failed after|recorded an issue)')
XC_CASE = re.compile(r"^Test Case '-\[(?:\S+\.)?(\S+) (\S+)\]' failed")
GRADLE = re.compile(r"^([\w.$]+) > (.+) FAILED\s*$")
LEAK = re.compile(r"LEAKED ([A-Za-z][\w.-]*(?: [A-Za-z][\w.-]*)*?)(?= \d|$| \()")

# Ordered: the first match names the cause. "Not acquired" and "lost" come from
# annotations, the rest also show in logs.
INFRA = [
    ("not acquired by Runner", r"not acquired by Runner|No runner matching|waiting for a runner"),
    ("runner lost", r"lost communication with the server|runner has received a shutdown signal|The hosted runner encountered an error"),
    ("job timed out", r"exceeded the maximum execution time"),
    ("out of disk", r"No space left on device"),
    ("DNS error", r"Could not resolve host|Couldn't resolve host|Temporary failure in name resolution|Name or service not known"),
    ("network error", r"spurious network error|Connection (?:reset|refused|timed out)|Operation timed out|TLS handshake|Network is unreachable|Bad Gateway|Service Unavailable|Gateway Time-?out"),
]


# A job that failed to build or lint has no tests to name; it is the code, not
# the runner, so it gets its own section rather than "unclassified".
BUILD = re.compile(r"could not compile|error\[E\d+\]|\*\* BUILD FAILED \*\*|FAILURE: Build failed|error: failed to run custom build")


def split_line(raw):
    """(job, text) for one --log-failed line: `job<TAB>step<TAB>timestamp text`."""
    raw = ANSI.sub("", raw.rstrip("\n"))
    parts = raw.split("\t", 2)
    if len(parts) == 3:
        job, text = parts[0], TS.sub("", parts[2])
    else:
        job, text = "", TS.sub("", raw)
    return job, text


def parse_log(text, annotations=""):
    """(tests, infra): the failing tests as (framework, name) in first-seen
    order, and the infrastructure causes the log and annotations show."""
    seen = {}

    def add(kind, name):
        seen.setdefault((kind, name), None)

    nextest, fallback = [], []
    swift_failed, swift_issue = [], []
    in_failing_list = False
    blob = [annotations]
    for raw in text.splitlines():
        _, line = split_line(raw)
        blob.append(line)
        body = line.strip()
        if in_failing_list:
            if line[:1] in ("\t", " ") and body:
                add("XCTest", re.sub(r"\(\)$", "", body.replace("/", ".")))
                continue
            in_failing_list = False
        if body == "Failing tests:":
            in_failing_list = True
        elif (m := NEXTEST.match(body)):
            nextest.append(("nextest", f"{m.group(1)} {m.group(2)}"))
        elif (m := SWIFT.match(body)):
            (swift_failed if m.group(2) == "failed after" else swift_issue).append(m.group(1))
        elif (m := XC_CASE.match(body)):
            add("XCTest", f"{m.group(1)}.{m.group(2)}")
        elif (m := GRADLE.match(body)):
            add("Gradle", f"{m.group(1)} > {m.group(2)}")
        elif "leak-check" in body and (m := LEAK.search(body)):
            add("leak", f"LEAKED {m.group(1)}")
        elif (m := CARGO_TEST.match(line)):
            fallback.append(("cargo test", m.group(1)))
        elif (m := PANICKED.search(body)):
            fallback.append(("cargo test", m.group(1)))
    for item in nextest:
        add(*item)
    # A per-test `... FAILED` or a panic names the test without its crate, so
    # it only stands in when nextest printed no FAIL line at all.
    if not nextest:
        for item in fallback:
            add(*item)
    # An issue with no `failed after` line (the run died first) still names it.
    for name in dict.fromkeys(swift_failed + swift_issue):
        add("swift-testing", name)
    joined = "\n".join(blob)
    infra = [label for label, pat in INFRA if re.search(pat, joined)]
    return list(seen), infra


def classify(rec):
    """('test', [(framework, name)]) | ('infra', cause) | ('other', why)."""
    tests, infra = parse_log(rec.get("log", ""), rec.get("annotations", ""))
    if tests:
        return "test", tests
    if infra:
        return "infra", infra[0]
    if BUILD.search(ANSI.sub("", rec.get("log", ""))):
        return "build", "build or lint error"
    if rec["conclusion"] == "cancelled":
        return "infra", "cancelled"
    if rec["conclusion"] == "timed_out":
        return "infra", "job timed out"
    return "other", "no failing test found in the log"


def tally(records):
    tests = collections.defaultdict(list)  # (framework, name) -> [rec]
    infra = collections.defaultdict(list)  # cause -> [rec]
    other, build = [], []
    for rec in records:
        kind, what = classify(rec)
        if kind == "test":
            for key in what:
                tests[key].append(rec)
        elif kind == "infra":
            infra[what].append(rec)
        elif kind == "build":
            build.append(rec)
        else:
            other.append(rec)
    return tests, infra, build, other


def where(rec):
    tail = " (run later went green)" if rec.get("final") == "success" else ""
    return f"{rec['job']}: {rec['url']}{tail}"


def render(records, runs_seen):
    tests, infra, build, other = tally(records)
    out = [f"CI failures on main: {len(records)} failed job attempts across {runs_seen} runs", ""]
    out.append("Tests, worst first")
    if not tests:
        out.append("  none")
    ranked = sorted(tests.items(), key=lambda kv: (-len(kv[1]), kv[0][1]))
    for rank, ((framework, name), recs) in enumerate(ranked, 1):
        green = sum(1 for r in recs if r.get("final") == "success")
        jobs = collections.Counter(r["job"] for r in recs)
        out.append(f"{rank:>3}. {len(recs)}x  {name}  [{framework}]")
        out.append("       jobs: " + ", ".join(f"{j} x{n}" for j, n in jobs.most_common()))
        if green:
            out.append(f"       {green} of these were on runs that went green after a re-run")
        for r in recs[:3]:
            out.append(f"       {r['url']}")
        if len(recs) > 3:
            out.append(f"       and {len(recs) - 3} more")
    out += ["", "Infrastructure, not tests"]
    if not infra:
        out.append("  none")
    for cause, recs in sorted(infra.items(), key=lambda kv: -len(kv[1])):
        out.append(f"  {len(recs)}x  {cause}")
        for r in recs[:5]:
            out.append(f"       {where(r)}")
        if len(recs) > 5:
            out.append(f"       and {len(recs) - 5} more")
    if build:
        out += ["", "Build or lint errors, not tests"]
        for r in build:
            out.append(f"  {where(r)}")
    if other:
        out += ["", "Unclassified (failed, but no test or known infrastructure cause in the log)"]
        for r in other:
            out.append(f"  {where(r)}")
    return "\n".join(out)


def gh(*args):
    done = subprocess.run(["gh", *args], capture_output=True, text=True)
    if done.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args)}: {done.stderr.strip()}")
    return done.stdout


def gh_soft(*args):
    """Logs and annotations can be gone (expired, or never written for a job
    that was never picked up); that is an empty answer, not a failed tally."""
    try:
        return gh(*args)
    except RuntimeError:
        return ""


def live_records(repo, workflow, branch, limit):
    runs = json.loads(gh("run", "list", "-R", repo, "--workflow", workflow, "--branch", branch,
                         "--limit", str(limit), "--json", "databaseId,attempt,conclusion,status"))
    runs = [r for r in runs if r["status"] == "completed"]
    records = []
    for run in runs:
        rid = run["databaseId"]
        for attempt in range(1, (run["attempt"] or 1) + 1):
            body = gh_soft("api", f"repos/{repo}/actions/runs/{rid}/attempts/{attempt}/jobs", "--paginate",
                           "--jq", '.jobs[] | select(.conclusion=="failure" or .conclusion=="cancelled" or .conclusion=="timed_out")'
                                   ' | {id, name, conclusion, html_url} | tojson')
            jobs = [json.loads(chunk) for chunk in body.splitlines()]
            if not jobs:
                continue
            # Per attempt, not `--job ID`: a job id's log is its latest
            # attempt's, so a flake that a re-run fixed would read as green.
            logs = collections.defaultdict(list)
            for line in gh_soft("run", "view", "-R", repo, str(rid), "--attempt", str(attempt),
                                "--log-failed").splitlines():
                logs[line.split("\t", 1)[0]].append(line)
            for job in jobs:
                log = "\n".join(logs.get(job["name"], []))
                rec = {"run": rid, "attempt": attempt, "job": job["name"], "conclusion": job["conclusion"],
                       "url": job["html_url"], "final": run["conclusion"], "log": log, "annotations": ""}
                if not parse_log(log)[0]:
                    rec["annotations"] = gh_soft("api", f"repos/{repo}/check-runs/{job['id']}/annotations",
                                                 "--jq", ".[].message")
                records.append(rec)
    return records, len(runs)


def manifest_records(path):
    """Lines of `run<TAB>attempt<TAB>job<TAB>conclusion<TAB>final<TAB>log-file`,
    log paths relative to the manifest."""
    import os
    base = os.path.dirname(os.path.abspath(path))
    records, runs = [], set()
    for line in open(path, encoding="utf-8"):
        if not line.strip() or line.startswith("#"):
            continue
        run, attempt, job, conclusion, final, logfile = line.rstrip("\n").split("\t")
        runs.add(run)
        records.append({"run": run, "attempt": attempt, "job": job, "conclusion": conclusion,
                        "final": final, "url": f"https://example.invalid/runs/{run}/attempts/{attempt}",
                        "log": open(os.path.join(base, logfile), encoding="utf-8").read(), "annotations": ""})
    return records, len(runs)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--runs", type=int, default=30)
    ap.add_argument("--branch", default="main")
    ap.add_argument("--workflow", default="ci.yml")
    ap.add_argument("--repo")
    ap.add_argument("--parse-log", metavar="FILE")
    ap.add_argument("--conclusion", default="failure")
    ap.add_argument("--annotations", metavar="FILE")
    ap.add_argument("--report", metavar="MANIFEST")
    args = ap.parse_args()

    if args.parse_log:
        notes = open(args.annotations, encoding="utf-8").read() if args.annotations else ""
        rec = {"log": open(args.parse_log, encoding="utf-8").read(), "annotations": notes,
               "conclusion": args.conclusion}
        kind, what = classify(rec)
        if kind == "test":
            for framework, name in what:
                print(f"test\t{framework}\t{name}")
        else:
            print(f"{kind}\t{what}")
        return 0
    if args.report:
        records, runs = manifest_records(args.report)
        print(render(records, runs))
        return 0
    repo = args.repo or json.loads(gh("repo", "view", "--json", "nameWithOwner"))["nameWithOwner"]
    records, runs = live_records(repo, args.workflow, args.branch, args.runs)
    print(render(records, runs))
    return 0


if __name__ == "__main__":
    sys.exit(main())
