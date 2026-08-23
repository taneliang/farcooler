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
  - a new method or payload field no capability accounts for

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

ROOT = pathlib.Path(__file__).resolve().parent.parent
PROTO = ROOT / "proto" / "farcooler.proto"

# A field line: `optional string display_path = 5;`, `repeated Foo bar = 1;`.
FIELD = re.compile(
    r"^\s*(?:(optional|repeated)\s+)?([\w.]+)\s+(\w+)\s*=\s*(\d+)\s*(?:\[[^\]]*\])?\s*;"
)
MESSAGE = re.compile(r"^\s*(?:message|enum)\s+(\w+)\s*\{")
ENUM_VALUE = re.compile(r"^\s*([A-Z][A-Z0-9_]*)\s*=\s*(\d+)\s*(?:\[[^\]]*\])?\s*;")


def parse(text):
    """Every field, keyed by `Message.tag`, plus its name and type.

    A brace-counting walk rather than a real parser: this file is ours, its
    shape is stable, and a lint that needed a protobuf dependency is a lint that
    stops running the first time someone's environment lacks it.
    """
    fields = {}
    stack = []
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("//"):
            continue
        opened = MESSAGE.match(line)
        if opened:
            stack.append(opened.group(1))
            continue
        if stripped.startswith("}"):
            if stack:
                stack.pop()
            continue
        if not stack:
            continue
        scope = stack[-1]
        m = FIELD.match(line)
        if m:
            qualifier, type_name, name, tag = m.groups()
            # The qualifier is part of the type, not decoration. `repeated
            # string` to `string` leaves the tag and the type name identical
            # while changing what a decoder produces, so dropping it here would
            # let exactly that through — which the self-test caught.
            kind = f"{qualifier} {type_name}".strip() if qualifier else type_name
            fields[f"{scope}.{tag}"] = (name, kind)
            continue
        e = ENUM_VALUE.match(line)
        if e:
            name, tag = e.groups()
            fields[f"{scope}.{tag}"] = (name, "enum")
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


def capability_problems():
    """Methods the daemon dispatches that no capability accounts for.

    Reads the daemon's own scope table, which is the list of methods that
    actually exist, and checks each against the capability table. A method with
    no capability cannot be asked for by a client that checks first, so it would
    ship as a feature nobody can discover.
    """
    scope_table = (ROOT / "crates" / "daemon" / "src" / "rpc.rs").read_text()
    cap_table = (ROOT / "crates" / "protocol" / "src" / "lib.rs").read_text()

    # Method names are string literals in `required_scope`'s match arms.
    start = scope_table.find("fn required_scope")
    end = scope_table.find("\n}", start)
    methods = set(re.findall(r'"([a-z_]+\.[a-z_.]+)"', scope_table[start:end]))

    cap_start = cap_table.find("pub fn for_method")
    cap_end = cap_table.find("\n    }", cap_start)
    covered = set(re.findall(r'"([a-z_]+\.[a-z_.]+)"', cap_table[cap_start:cap_end]))
    prefixes = re.findall(r'm\.starts_with\("([a-z_]+\.)"\)', cap_table[cap_start:cap_end])

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

    for f in failures:
        print(f"FAIL: {f}", file=sys.stderr)
    print(f"{len(cases) + 1 - len(failures)} passed, {len(failures)} failed")
    return 1 if failures else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--channel", default="preview", choices=["preview", "stable"])
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    problems = capability_problems()

    baseline_path = ROOT / "proto" / "baseline" / f"{args.channel}.proto"
    if not baseline_path.exists():
        print(f"no {args.channel} baseline yet — nothing has shipped, so nothing is owed compatibility")
    else:
        problems += compare(parse(baseline_path.read_text()), parse(PROTO.read_text()))

    if problems:
        print(f"\n{len(problems)} wire compatibility problem(s):\n", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        print(
            "\nThe wire is additive-only. See docs/releasing.md.",
            file=sys.stderr,
        )
        return 1
    print(f"proto is compatible with the {args.channel} baseline")
    return 0


if __name__ == "__main__":
    sys.exit(main())
