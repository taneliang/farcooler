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


def _clean(text):
    """The source with comments, strings and char literals blanked, so braces and keywords inside them are inert."""
    out, i, n = [], 0, len(text)
    while i < n:
        two = text[i:i + 2]
        if two == "//":
            j = text.find("\n", i)
            i = n if j < 0 else j
        elif two == "/*":
            depth, i = 1, i + 2
            while i < n and depth:
                if text[i:i + 2] == "/*":
                    depth, i = depth + 1, i + 2
                elif text[i:i + 2] == "*/":
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
        elif (raw := re.match(r'b?r(#*)"', text[i:i + 12])) and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            end = text.find('"' + raw.group(1), i + raw.end())
            i = n if end < 0 else end + 1 + len(raw.group(1))
        elif text[i] == '"':
            i += 1
            while i < n and text[i] != '"':
                i += 2 if text[i] == "\\" else 1
            i += 1
        elif (ch := re.match(r"'(?:\\.[^']*|[^'\\])'", text[i:i + 12])):
            i += ch.end()
        else:
            out.append(text[i])
            i += 1
    return "".join(out)


_ITEM = re.compile(
    r"(?P<mod>\bmod\s+(?P<mname>\w+)\s*\{)"
    r"|(?P<fn>(?P<attrs>(?:#\[[^\]]*\]\s*)+)(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+(?P<fname>\w+))"
    r"|(?P<open>\{)|(?P<close>\})")
_TEST_ATTR = re.compile(r"#\[\s*(?:[\w:]+::)?(?:test|rstest|test_case)\s*(?:\(|\])")


def inline_tests(text):
    """Module-relative names (`a::b::name`) of the #[test] functions in one file's text."""
    names, stack = [], []
    for m in _ITEM.finditer(_clean(text)):
        if m["mod"]:
            stack.append(m["mname"])
        elif m["open"]:
            stack.append(None)
        elif m["close"]:
            if stack:
                stack.pop()
        elif m["fn"] and any(_TEST_ATTR.match(a) for a in re.findall(r"#\[[^\]]*\]", m["attrs"])):
            names.append("::".join([x for x in stack if x] + [m["fname"]]))
    return names


def _module(parts):
    """The module path of a source file given its path parts below src/ (or tests/)."""
    parts = list(parts)
    parts[-1] = parts[-1].removesuffix(".rs")
    if parts[-1] in ("lib", "main", "mod"):
        parts.pop()
    return parts


def real_tests(root):
    """Every (binary id, full test name) in the repo's crates, read from source the way nextest names them.

    Static, so the always-on lint needs no build: a unit test is `<package>` (lib) or
    `<package>::bin/<name>` plus its module path; an integration test is `<package>::<file>`.
    """
    found = set()
    for manifest in sorted(root.glob("crates/*/Cargo.toml")):
        crate, text = manifest.parent, manifest.read_text()
        package = re.search(r'^\[package\][^\[]*?^name\s*=\s*"([^"]+)"', text, re.S | re.M)
        if not package:
            continue
        package = package.group(1)
        has_lib = (crate / "src/lib.rs").exists()
        declared = re.findall(r'^\[\[bin\]\]\s*\nname\s*=\s*"([^"]+)"(?:\s*\npath\s*=\s*"([^"]+)")?', text, re.M)
        main_bin = next((n for n, path in declared if path in ("", "src/main.rs")), package)
        for path in sorted((crate / "src").rglob("*.rs")) if (crate / "src").exists() else []:
            rel = path.relative_to(crate / "src").parts
            if rel[0] == "bin":
                stem = rel[1].removesuffix(".rs")
                binary, module = f"{package}::bin/{next((n for n, p in declared if p == 'src/' + '/'.join(rel)), stem)}", _module(rel[2:] or ["main"])
                if len(rel) == 2:
                    module = []
            elif has_lib:
                binary, module = package, _module(rel)
            else:
                binary, module = f"{package}::bin/{main_bin}", _module(rel)
            for name in inline_tests(path.read_text(errors="replace")):
                found.add((binary, "::".join(module + [name])))
        for path in sorted((crate / "tests").rglob("*.rs")) if (crate / "tests").exists() else []:
            rel = path.relative_to(crate / "tests").parts
            module = _module(rel[1:]) if len(rel) > 1 and rel[-1] != "main.rs" else []
            if len(rel) > 1 and rel[-1] == "main.rs":
                module = []
            for name in inline_tests(path.read_text(errors="replace")):
                found.add((f"{package}::{rel[0].removesuffix('.rs')}", "::".join(module + [name])))
    return found


def unknown(entries, root):
    """Entries whose exact binary id and test name match no real test, so a typo or a rename fails the lint."""
    real = real_tests(root)
    return [f"line {number}: no test `{test}` in binary `{binary}` (check the binary id, module path and #[test])"
            for binary, test, _ticket, _expiry, number in entries if (binary, test) not in real]


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
