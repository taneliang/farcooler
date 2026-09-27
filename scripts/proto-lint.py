#!/usr/bin/env python3
"""Refuse a wire change that would break a client already in the field.

The wire is additive-only. An App Store review takes days and a daemon update
takes one command, so there will always be an app in the field older than the
machine it is talking to. That app decodes messages by tag number, and it has
no way to learn that a meaning changed.

What this refuses, against the baseline for the channel being built:

  - a removed field          an older client still sends it; a newer one still reads it
  - a reused tag number      the worst one: the same bytes decode as a different thing
  - a changed field type     same bytes, different meaning, no error anywhere
  - a renamed field          JSON-facing tooling keys on the name
  - a new method the daemon dispatches that no capability accounts for

Baselines live in proto/baseline/ and are committed by the promotion workflow
that created the tag. Deliberately NOT derived from git: this repository has no
tags yet, ci.yml checks out at depth 1 so tags are not even fetched, and
version.sh already carries a comment about shallow clones failing silently.

    ./scripts/proto-lint.py                 # against the preview baseline
    ./scripts/proto-lint.py --channel stable
    ./scripts/proto-lint.py --self-test     # the lint's own tests

The two channels here are the two that ship: `promote.yml` writes
`proto/baseline/<channel>.proto` for whichever of preview and stable it just
tagged. They were `beta` and `release` until `2ae5cf3` renamed the channels, and
this file was not renamed with them — so the two names asked for here were names
the promotion workflow never wrote, and every run found no baseline and passed
saying nothing had shipped. A guard that cannot fire, in the one place the
repository has no second opinion: a wire break is invisible until an app in the
field decodes it.

Before a first release the baseline is absent and this exits 0 saying so:
nothing has shipped, so nothing is owed compatibility.
"""

import argparse
import pathlib
import re
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
PROTO = ROOT / "proto" / "farcooler.proto"

# One declaration or brace, found ANYWHERE in the text rather than at the start
# of a line: `message AgentCancel { bytes terminal_id = 1; }` puts the opening,
# the field and the closing on one line, and a line-anchored match saw only the
# opening — so that field, and every other one-line message's, was never parsed.
TOKEN = re.compile(
    r"""
      (?P<open>\b(?:message|enum|oneof)\s+(?P<block>\w+)\s*\{)
    | (?P<close>\})
    | (?P<field>
        \b(?:(?P<qualifier>optional|repeated)\s+)?
        (?P<type>map\s*<\s*[\w.]+\s*,\s*[\w.]+\s*>|[\w.]+)\s+
        (?P<name>\w+)\s*=\s*(?P<tag>\d+)\s*(?:\[[^\]]*\])?\s*;)
    | (?P<value>\b(?P<value_name>[A-Z][A-Z0-9_]*)\s*=\s*(?P<value_tag>\d+)\s*(?:\[[^\]]*\])?\s*;)
    """,
    re.VERBOSE,
)


def parse(text):
    """Every field, keyed by `Message.tag`, plus its name and type.

    A brace-counting walk rather than a real parser: this file is ours, its
    shape is stable, and a lint that needed a protobuf dependency is a lint that
    stops running the first time someone's environment lacks it. The self-test
    counts the real proto's declarations a second way, so a shape this walk
    cannot see fails there rather than passing here.
    """
    # Comments first, so a brace or an `= 1;` in prose is not read as syntax.
    text = "\n".join(line.split("//", 1)[0] for line in text.splitlines())
    fields = {}
    # (kind, name) per open block. A oneof is a block of its own, so its `}`
    # closes it and not the message around it, but it is not a scope: its
    # members share the message's tag numbers, so they key under the message.
    stack = []
    for m in TOKEN.finditer(text):
        if m.group("open"):
            kind = m.group("open").split(None, 1)[0]
            stack.append((kind, m.group("block")))
            continue
        if m.group("close"):
            if stack:
                stack.pop()
            continue
        names = [name for kind, name in stack if kind != "oneof"]
        if not names:
            continue
        scope = ".".join(names)
        if m.group("field"):
            qualifier, type_name = m.group("qualifier"), m.group("type")
            type_name = re.sub(r"\s+", "", type_name).replace(",", ", ")
            # The qualifier is part of the type, not decoration. `repeated
            # string` to `string` leaves the tag and the type name identical
            # while changing what a decoder produces, so dropping it here would
            # let exactly that through — which the self-test caught.
            kind = f"{qualifier} {type_name}" if qualifier else type_name
            fields[f"{scope}.{m.group('tag')}"] = (m.group("name"), kind)
        else:
            fields[f"{scope}.{m.group('value_tag')}"] = (m.group("value_name"), "enum")
    return fields


def compare(baseline, current):
    """What changed, as a list of sentences a person can act on."""
    problems = []
    for key, (name, kind) in sorted(baseline.items()):
        scope, tag = key.rsplit(".", 1)
        if key not in current:
            problems.append(
                f"{scope} tag {tag} ({name}) was removed. "
                f"Reserve it instead — a client in the field still sends it."
            )
            continue
        now_name, now_kind = current[key]
        if now_kind != kind:
            problems.append(
                f"{scope} tag {tag} ({name}) changed type from {kind} to {now_kind}. "
                f"The same bytes would decode as a different thing."
            )
        if now_name != name:
            problems.append(
                f"{scope} tag {tag} was renamed from {name} to {now_name}. "
                f"Add a new field instead."
            )
    return problems


def capability_problems(scope_table=None, cap_table=None):
    """Methods the daemon dispatches that no capability accounts for.

    Reads the daemon's own scope table, which is the list of methods that
    actually exist, and checks each against the capability table. A method with
    no capability cannot be asked for by a client that checks first, so it would
    ship as a feature nobody can discover.

    Both tables can be passed in, so the self-test can hand it a method the
    real tables do not have.
    """
    if scope_table is None:
        scope_table = (ROOT / "crates" / "daemon" / "src" / "rpc.rs").read_text()
    if cap_table is None:
        cap_table = (ROOT / "crates" / "protocol" / "src" / "lib.rs").read_text()

    # Method names are string literals in `required_scope`'s match arms. Digits
    # included: `v2.anything` or `host.get_v2` is as much a method as any other,
    # and a pattern without 0-9 simply never saw one.
    method = r'"([a-z0-9_]+\.[a-z0-9_.]+)"'
    start = scope_table.find("fn required_scope")
    end = scope_table.find("\n}", start)
    methods = set(re.findall(method, scope_table[start:end])) if start >= 0 else set()

    cap_start = cap_table.find("pub fn for_method")
    cap_end = cap_table.find("\n    }", cap_start)
    cap_body = cap_table[cap_start:cap_end] if cap_start >= 0 else ""
    covered = set(re.findall(method, cap_body))
    prefixes = re.findall(r'm\.starts_with\("([a-z0-9_]+\.)"\)', cap_body)

    # A slice that found nothing checks nothing and would pass. Say so instead.
    if not methods or not covered:
        return [
            "found no methods in `required_scope` or no capabilities in "
            "`capability::for_method`; the tables moved and this check went blind."
        ]

    missing = sorted(
        m
        for m in methods - covered
        if not any(m.startswith(p) for p in prefixes)
    )
    return [
        f"`{m}` is dispatched by the daemon but names no capability. "
        f"Add it to `capability::for_method`, or a client cannot discover it."
        for m in missing
    ]


def lint(channel, baseline_dir):
    """The problems, and the line to print if there are none."""
    problems = capability_problems()
    baseline_path = baseline_dir / f"{channel}.proto"
    if not baseline_path.exists():
        return problems, (
            f"no {channel} baseline yet — nothing has shipped, so nothing was compared "
            f"(only the capability table was checked)"
        )
    problems += compare(parse(baseline_path.read_text()), parse(PROTO.read_text()))
    return problems, f"proto is compatible with the {channel} baseline"


def self_test():
    """The lint's own tests, so it is not dormant until the first release.

    Without these, every check here is unexercised until a baseline exists —
    which is exactly when a broken lint is least likely to be noticed.
    """
    base = parse(
        """
        message Foo {
          string alpha = 1;
          repeated string beta = 2;
        }
        """
    )
    cases = [
        ("a removed field", "message Foo {\n  string alpha = 1;\n}", "was removed"),
        (
            "a reused tag",
            "message Foo {\n  string alpha = 1;\n  repeated string gamma = 2;\n}",
            "renamed",
        ),
        (
            "a changed type",
            "message Foo {\n  string alpha = 1;\n  string beta = 2;\n}",
            "changed type",
        ),
        (
            "a renamed field",
            "message Foo {\n  string renamed = 1;\n  repeated string beta = 2;\n}",
            "renamed",
        ),
    ]
    failures = []
    for what, text, expected in cases:
        problems = compare(base, parse(text))
        if not any(expected in p for p in problems):
            failures.append(f"{what}: expected a problem mentioning {expected!r}, got {problems}")

    # And the case that must NOT complain: adding a field is the whole point.
    added = parse(
        "message Foo {\n  string alpha = 1;\n  repeated string beta = 2;\n  string added = 3;\n}"
    )
    if compare(base, added):
        failures.append("adding a field must be allowed; that is what additive-only means")

    # Shapes the line-at-a-time parser could not see. Each one is a real shape
    # in proto/farcooler.proto, and each base was parsed to nothing (or to the
    # wrong scope), so removing, retyping or renumbering it compared clean.
    # A renumber shows up as the old tag removed.
    shapes = [
        (
            "a one-line message",
            "message AgentCancel { bytes terminal_id = 1; }",
            [
                ("removed", "message AgentCancel {}", "was removed"),
                ("retyped", "message AgentCancel { string terminal_id = 1; }", "changed type"),
                ("renumbered", "message AgentCancel { bytes terminal_id = 2; }", "was removed"),
            ],
        ),
        (
            "a map field",
            "message Launch {\n  string name = 1;\n  map<string, string> env = 4;\n}",
            [
                ("removed", "message Launch {\n  string name = 1;\n}", "was removed"),
                (
                    "retyped",
                    "message Launch {\n  string name = 1;\n  map<string, bytes> env = 4;\n}",
                    "changed type",
                ),
                (
                    "renumbered",
                    "message Launch {\n  string name = 1;\n  map<string, string> env = 5;\n}",
                    "was removed",
                ),
            ],
        ),
        (
            "a oneof member",
            "message Envelope {\n  oneof body {\n    Hello hello = 1;\n    Bye bye = 2;\n  }\n}",
            [
                ("removed", "message Envelope {\n  oneof body {\n    Hello hello = 1;\n  }\n}", "was removed"),
                (
                    "retyped",
                    "message Envelope {\n  oneof body {\n    Hello hello = 1;\n    Hello bye = 2;\n  }\n}",
                    "changed type",
                ),
                (
                    "renumbered",
                    "message Envelope {\n  oneof body {\n    Hello hello = 1;\n    Bye bye = 3;\n  }\n}",
                    "was removed",
                ),
            ],
        ),
        (
            # The oneof's closing brace used to pop the MESSAGE, so everything
            # after it in the same message fell outside any scope.
            "a field after a oneof",
            "message Envelope {\n  oneof body {\n    Hello hello = 1;\n  }\n  string trailer = 2;\n}",
            [
                ("removed", "message Envelope {\n  oneof body {\n    Hello hello = 1;\n  }\n}", "was removed"),
                (
                    "retyped",
                    "message Envelope {\n  oneof body {\n    Hello hello = 1;\n  }\n  bytes trailer = 2;\n}",
                    "changed type",
                ),
                (
                    "renumbered",
                    "message Envelope {\n  oneof body {\n    Hello hello = 1;\n  }\n  string trailer = 3;\n}",
                    "was removed",
                ),
            ],
        ),
        (
            "a one-line enum",
            "enum Tone { TONE_UNSPECIFIED = 0; TONE_LOUD = 1; }",
            [
                ("removed", "enum Tone { TONE_UNSPECIFIED = 0; }", "was removed"),
                ("renamed", "enum Tone { TONE_UNSPECIFIED = 0; TONE_QUIET = 1; }", "renamed"),
                ("renumbered", "enum Tone { TONE_UNSPECIFIED = 0; TONE_LOUD = 2; }", "was removed"),
            ],
        ),
    ]
    count = len(cases) + 1
    for shape, base_text, mutations in shapes:
        shape_base = parse(base_text)
        for how, text, expected in mutations:
            count += 1
            problems = compare(shape_base, parse(text))
            if not any(expected in p for p in problems):
                failures.append(
                    f"{shape} {how}: expected a problem mentioning {expected!r}, got {problems}"
                )

    # The real file, counted a second way. Every declaration that carries a
    # number ends `= N;` (or `= N [options];`), whatever shape surrounds it, so
    # a shape the parser skips makes the two counts differ. Comments are
    # stripped first because prose may say `= 1;` too.
    count += 1
    real = PROTO.read_text()
    uncommented = "\n".join(line.split("//", 1)[0] for line in real.splitlines())
    declared = len(re.findall(r"=\s*\d+\s*(?:\[[^\]]*\])?\s*;", uncommented))
    parsed = len(parse(real))
    if parsed != declared:
        failures.append(
            f"proto/farcooler.proto declares {declared} numbered fields and enum values "
            f"but the parser found {parsed}; some shape is invisible to it"
        )

    # The capability check, which had no test at all. A method whose name has a
    # digit was never extracted from the scope table, so it could ship with no
    # capability and pass.
    scope_fixture = (
        "fn required_scope(method: &str) -> Option<Scope> {\n"
        "    Some(match method {\n"
        '        "host.get" | "zzz.brand_new" | "v2.brand_new" | "host.get_v2" => Scope::Read,\n'
        "        _ => return None,\n"
        "    })\n"
        "}\n"
    )
    cap_fixture = (
        "    pub fn for_method(method: &str) -> Option<&'static str> {\n"
        "        Some(match method {\n"
        '            "host.get" => WORKTREES,\n'
        "            _ => return None,\n"
        "        })\n"
        "    }\n"
    )
    flagged = capability_problems(scope_fixture, cap_fixture)
    for method in ["zzz.brand_new", "v2.brand_new", "host.get_v2"]:
        count += 1
        if not any(f"`{method}`" in p for p in flagged):
            failures.append(f"a method with no capability, {method}, was not flagged: {flagged}")
    count += 1
    if any("`host.get`" in p for p in flagged):
        failures.append(f"host.get has a capability and was flagged anyway: {flagged}")

    # Tables the slicing cannot find must fail, not check nothing and pass.
    count += 1
    if not capability_problems("", ""):
        failures.append("empty scope and capability tables passed the capability check")

    # With no baseline nothing was compared, and the last line is the one
    # people quote. It used to say "compatible with the baseline" regardless.
    count += 1
    with tempfile.TemporaryDirectory() as empty:
        _, verdict = lint("preview", pathlib.Path(empty))
    if "compatible" in verdict or "nothing was compared" not in verdict:
        failures.append(f"with no baseline the verdict must say nothing was compared: {verdict!r}")

    for f in failures:
        print(f"FAIL: {f}", file=sys.stderr)
    print(f"{count - len(failures)} passed, {len(failures)} failed")
    return 1 if failures else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--channel", default="preview", choices=["preview", "stable"])
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    problems, verdict = lint(args.channel, ROOT / "proto" / "baseline")
    if problems:
        print(f"\n{len(problems)} wire compatibility problem(s):\n", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        print(
            "\nThe wire is additive-only. See docs/releasing.md.",
            file=sys.stderr,
        )
        return 1
    print(verdict)
    return 0


if __name__ == "__main__":
    sys.exit(main())
