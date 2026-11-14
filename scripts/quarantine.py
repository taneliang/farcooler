#!/usr/bin/env python3
"""A checked-in quarantine list for flaky Rust tests (ov-1229).

    scripts/quarantine.py --lint                       # the list is well formed, live and matches real tests
    scripts/quarantine.py run -- cargo nextest run ... # nextest, with listed tests rerun up to twice

`.config/quarantine.txt` lists a test, the ticket that tracks its fix and an
expiry date (format in that file, notes in docs/ci/quarantine.md). `run` wraps a
`cargo nextest run --profile ci ...` command, in CI and in the lane gate alike:
a listed test that fails is rerun, up to twice, and if a rerun passes it is
reported as "flaky (quarantined)" and the run stays green. A listed test that
fails every rerun still fails the run, and so does any test that is not listed:
only the listed ones get a rerun. With an empty list the command runs unchanged.

Nothing here opens a PR, a branch or a ticket. Adding a line is an ordinary edit.
"""
import datetime as dt
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent
LIST = ROOT / ".config/quarantine.txt"
RERUNS = 2
TICKET = re.compile(r"^ov-\d+$")


def parse(text, today):
    """(entries, problems) for the file's text. An entry is (binary, test, ticket, expiry, line number)."""
    entries, problems, seen = [], [], set()
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        fields = line.split()
        if len(fields) != 4:
            problems.append(f"line {number}: want `<binary-id> <test-name> <ticket> <expiry>`, got {len(fields)} field(s)")
            continue
        binary, test, ticket, expiry = fields
        if not TICKET.match(ticket):
            problems.append(f"line {number}: ticket `{ticket}` is not like ov-123")
            continue
        try:
            day = dt.date.fromisoformat(expiry)
        except ValueError:
            problems.append(f"line {number}: expiry `{expiry}` is not a YYYY-MM-DD date")
            continue
        if (binary, test) in seen:
            problems.append(f"line {number}: {binary} {test} is listed twice")
            continue
        seen.add((binary, test))
        if day <= today:
            problems.append(f"line {number}: {binary} {test} expired on {expiry} ({ticket}): fix it and remove the line")
        entries.append((binary, test, ticket, expiry, number))
    return entries, problems


def sources(root):
    """Every .rs file under the repo's Rust trees, as (path, text)."""
    for sub in ("crates", "test", "services", "apps"):
        for path in (root / sub).rglob("*.rs"):
            if "target" not in path.parts:
                yield path, path.read_text(errors="replace")


def unknown(entries, root):
    """Entries whose name matches no test: a cheap static check, so the always-on lint needs no build.

    The test's last path segment must be a `fn` somewhere, and an integration
    test's binary (`package::file`) must have a file or directory by that name.
    """
    texts = list(sources(root))
    problems = []
    for binary, test, _ticket, _expiry, number in entries:
        leaf = test.split("::")[-1]
        if not any(re.search(rf"\bfn\s+{re.escape(leaf)}\b", text) for _p, text in texts):
            problems.append(f"line {number}: no test function `{leaf}` exists ({binary} {test})")
        if "::" in binary:
            stem = binary.split("::", 1)[1]
            if not any(p.stem == stem or stem in p.parts for p, _t in texts):
                problems.append(f"line {number}: no test binary `{stem}` exists ({binary} {test})")
    return problems


def lint(path=LIST, root=ROOT, today=None):
    today = today or dt.datetime.now(dt.timezone.utc).date()
    entries, problems = parse(path.read_text(), today)
    return problems + unknown(entries, root)


def nextest_config(base, entries, report):
    """The repo's nextest config plus a rerun override per listed test and a JUnit report."""
    text = base + "\n[profile.ci.junit]\npath = " + json.dumps(str(report)) + "\n"
    for binary, test, *_ in entries:
        # Exact regexes, so a name is never read as a filter expression.
        exact = lambda s: "^" + re.escape(s).replace("/", r"\/") + "$"
        text += ("\n[[profile.ci.overrides]]\nfilter = " + json.dumps(f"binary_id(/{exact(binary)}/) and test(/{exact(test)}/)")
                 + f"\nretries = {RERUNS}\n")
    return text


def flaky(report):
    """(binary, test) of each case that failed, then passed on a rerun."""
    found = set()
    for suite in ET.parse(report).getroot().iter("testsuite"):
        for case in suite.findall("testcase"):
            if (case.find("failure") is None and case.find("error") is None
                    and any(case.find(t) is not None for t in ("flakyFailure", "flakyError"))):
                found.add((suite.attrib["name"], case.attrib["name"]))
    return sorted(found)


def run(command, base=None, entries=None, say=print):
    """Run a nextest command with the listed tests rerun; return its exit code."""
    base = base if base is not None else (ROOT / ".config/nextest.toml").read_text()
    if entries is None:
        entries, problems = parse(LIST.read_text(), dt.datetime.now(dt.timezone.utc).date())
        if problems:
            raise ValueError("; ".join(problems))
    if not entries:
        return subprocess.call(command)
    if command[:3] != ["cargo", "nextest", "run"] or command[command.index("--profile") + 1:][:1] != ["ci"]:
        raise ValueError("expected cargo nextest run --profile ci ...")
    with tempfile.TemporaryDirectory(prefix="quarantine-") as tmp:
        report, config = pathlib.Path(tmp, "junit.xml"), pathlib.Path(tmp, "nextest.toml")
        config.write_text(nextest_config(base, entries, report))
        rc = subprocess.call([*command, "--config-file", str(config)])
        listed = {(b, t) for b, t, *_ in entries}
        tickets = {(b, t): k for b, t, k, *_ in entries}
        for binary, test in (flaky(report) if report.exists() else []):
            # JUnit names the suite by binary id and the case by test name.
            if (binary, test) in listed:
                say(f"flaky (quarantined): {binary} {test} failed, then passed on a rerun ({tickets[(binary, test)]})")
        return rc


def main(argv):
    if argv[:1] == ["--lint"]:
        problems = lint()
        for problem in problems:
            print(f"quarantine: {problem}", file=sys.stderr)
        return 1 if problems else 0
    if argv[:1] == ["run"]:
        command = argv[1:]
        command = command[1:] if command[:1] == ["--"] else command
        try:
            return run(command)
        except ValueError as error:
            print(f"quarantine: {error}", file=sys.stderr)
            return 1
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
