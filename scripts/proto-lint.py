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
    ./scripts/proto-lint.py --channel canary
    ./scripts/proto-lint.py --self-test     # the lint's own tests
    ./scripts/proto-lint.py --compare OLD NEW   # two files, wire rules only

`--compare` is for scripts/canary-baseline.sh, which checks the proto Canary
shipped against the baseline it is about to replace. Without that check a
break that shipped became the baseline, and every later lint passed against it.

The two channels here are the two that ship: `promote.yml` writes
`proto/baseline/<channel>.proto` for whichever of preview and stable it just
tagged. They were `beta` and `release` until `2ae5cf3` renamed the channels, and
this file was not renamed with them — so the two names asked for here were names
the promotion workflow never wrote, and every run found no baseline and passed
saying nothing had shipped. A guard that cannot fire, in the one place the
repository has no second opinion: a wire break is invisible until an app in the
field decodes it.

Canary is the third, and ships more than the other two together: every push to
main reaches the owner's phones and Macs, so a field renumbered between two
pushes is a Canary phone and a newer Canary daemon disagreeing with no error.
`canary.yml` writes `proto/baseline/canary.proto` after each successful ship,
through scripts/canary-baseline.sh, with a first comment line naming the commit
it came from. The parser strips comments, so that line is invisible here.

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

# Each channel with a baseline, and the workflow that writes it. The self-test
# checks each workflow still names its channel, so a rename cannot again leave
# this asking for a file nothing writes.
CHANNELS = {
    "preview": ".github/workflows/promote.yml",
    "stable": ".github/workflows/promote.yml",
    "canary": ".github/workflows/canary.yml",
}

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

    Every method is one row of `methods!` in the protocol crate,
    `Variant = "name" => CAPABILITY,`, and the daemon's `required_scope` reads
    `Method::parse`, so the compiler already refuses a row without a
    capability, or naming a constant `capability` lacks. What it does not
    refuse, and this does:

      - a capability missing from `capability::ALL`, which the daemon
        advertises from: its methods exist, and no client can discover them
      - one wire name on two rows, where the second is only a warning
      - a `required_scope` that names methods itself again instead of going
        through `Method::parse`, so a method can be dispatched outside the
        table, which is the drift the table was made to end

    Both files can be passed in, so the self-test can hand it rows the real
    tables do not have.
    """
    if scope_table is None:
        scope_table = (ROOT / "crates" / "daemon" / "src" / "rpc.rs").read_text()
    if cap_table is None:
        cap_table = (ROOT / "crates" / "protocol" / "src" / "lib.rs").read_text()

    # The invocation, not the macro_rules! that defines it.
    invocation = re.search(r"^\s*methods!\s*\{(.*?)^\s*\}", cap_table, re.M | re.S)
    # Digits included: `v2.anything` or `host.get_v2` is as much a method as
    # any other, and a pattern without 0-9 simply never saw one.
    rows = re.findall(r'(\w+)\s*=\s*"([a-z0-9_]+\.[a-z0-9_.]+)"\s*=>\s*(\w+)\s*,', invocation.group(1)) if invocation else []
    module = re.search(r"pub mod capability \{(.*?)\n\}", cap_table, re.S)
    constants = set(re.findall(r"pub const ([A-Z][A-Z0-9_]*): &str\b", module.group(1))) if module else set()
    advertised = re.search(r"pub const ALL: &\[&str\]\s*=\s*&\[(.*?)\]", module.group(1), re.S) if module else None
    start = scope_table.find("fn required_scope")
    end = scope_table.find("\n}", start)
    scope_body = scope_table[start:end] if start >= 0 else ""

    # A slice that found nothing checks nothing and would pass. Say so instead.
    if not rows or not constants or not advertised or not scope_body:
        return [
            "found no methods in `methods!`, no capabilities in `capability`, no "
            "`capability::ALL` or no `required_scope`; the tables moved and this check went blind."
        ]

    problems = []
    if "Method::parse" not in scope_body or re.search(r'"[a-z0-9_]+\.[a-z0-9_.]+"', scope_body):
        problems.append(
            "`required_scope` does not read `Method::parse`, so the daemon can dispatch a method "
            "outside `methods!` that names no capability."
        )
    listed = set(re.findall(r"\b[A-Z][A-Z0-9_]*\b", advertised.group(1)))
    seen = set()
    for variant, name, capability in rows:
        if name in seen:
            problems.append(f"`{name}` is on two rows of `methods!`; the second is never reached.")
        seen.add(name)
        if capability not in constants:
            problems.append(f"`{name}` names capability `{capability}`, which `capability` does not declare.")
        elif capability not in listed:
            problems.append(
                f"`{name}` belongs to `{capability}`, which is not in `capability::ALL`, "
                f"so no client can discover it."
            )
    return problems


def workflow_jobs(text):
    """The `jobs:` of a workflow: each job's top-level keys, raw, by name.

    Not a YAML parser, because a lint that needs PyYAML stops running wherever
    it is missing (it is, on this repository's Macs). It reads the shape
    workflows here are written in: two-space indentation, jobs at indent 2,
    their keys at indent 4. A value is everything to the next key at indent 4,
    so a block list or a multi-line `if` is kept whole. Comments are dropped.
    """
    jobs, job, key, in_jobs = {}, None, None, False
    for line in text.splitlines():
        bare = line.split(" #", 1)[0].rstrip() if not line.lstrip().startswith("#") else ""
        if not bare:
            continue
        indent = len(bare) - len(bare.lstrip())
        if indent == 0:
            in_jobs = bare == "jobs:"
            continue
        if not in_jobs:
            continue
        if indent == 2 and bare.endswith(":"):
            job, key = bare.strip()[:-1], None
            jobs[job] = {}
        elif indent == 4 and job and ":" in bare:
            key, value = bare.strip().split(":", 1)
            jobs[job][key] = value.strip()
        elif job and key:
            jobs[job][key] += "\n" + bare.strip()
    return jobs


def needs_of(value):
    """`needs: a`, `needs: [a, b]` or a block list, as a set of job names."""
    return set(re.findall(r"[\w-]+", value or ""))


def canary_gate_problems(text):
    """How a wire break could still ship in canary.yml, as sentences.

    Every job but the lint must reach it through `needs`, and nothing may
    undo that: a status function in an `if` (always(), cancelled(),
    failure()) runs a job after its need failed, and continue-on-error or
    `|| true` in the lint job lets it fail and still count as passed.
    """
    jobs = workflow_jobs(text)
    gates = [j for j, keys in jobs.items() if "proto-lint.py --channel canary" in keys.get("steps", "")]
    if len(gates) != 1:
        return [f"expected one job running `proto-lint.py --channel canary`, found {gates}"]
    gate = gates[0]
    problems = []
    body = jobs[gate]
    if "continue-on-error" in body or "continue-on-error" in body.get("steps", ""):
        problems.append(f"`{gate}` has continue-on-error, so a failed lint still passes")
    if re.search(r"\|\|\s*(true|:|exit 0)", body.get("steps", "")):
        problems.append(f"`{gate}` swallows a lint failure with `|| true`")
    if "if" in body or re.search(r"^\s*(-\s*)?if:", body.get("steps", ""), re.M):
        problems.append(f"`{gate}` has an `if`, so the lint can be skipped")
    for job, keys in jobs.items():
        if job == gate:
            continue
        if re.search(r"\b(always|cancelled|failure)\(\)", keys.get("if", "")):
            problems.append(f"`{job}`'s `if` uses a status function, so it can run after the lint failed")
        seen, todo = set(), [job]
        while todo:
            for need in needs_of(jobs.get(todo.pop(), {}).get("needs")) - seen:
                seen.add(need)
                todo.append(need)
        if gate not in seen:
            problems.append(f"`{job}` does not need `{gate}`, directly or through another job, so a wire break can ship")
    return problems


def timeout_problems(text):
    """The jobs in a workflow that could hang until GitHub's six-hour cap.

    A job with no `timeout-minutes` runs for six hours before GitHub kills it,
    holding a runner (a macOS one bills at ten times Linux) and, in Canary and
    CI, the concurrency group behind it. A job that calls a reusable workflow
    through `uses:` cannot take one; the called workflow's jobs carry it.
    """
    return [
        f"`{job}` has no timeout-minutes, so a hang runs for six hours"
        for job, keys in workflow_jobs(text).items()
        if "uses" not in keys and not re.fullmatch(r"[1-9]\d*", keys.get("timeout-minutes", ""))
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

    # The capability check. Fixtures in the shape of the real tables: a method
    # whose capability is undeclared, one whose capability is never advertised,
    # one name on two rows, and digits in names, which a pattern without 0-9
    # once never saw.
    scope_fixture = (
        "fn required_scope(method: &str) -> Option<Scope> {\n"
        "    Method::parse(method).map(scope_of)\n"
        "}\n"
    )
    cap_fixture = (
        "pub mod capability {\n"
        '    pub const WORKTREES: &str = "workspaces";\n'
        '    pub const HIDDEN: &str = "hidden";\n'
        "    pub const ALL: &[&str] =\n"
        "        &[\n"
        "            WORKTREES,\n"
        "        ];\n"
        "}\n"
        "pub mod method {\n"
        "    macro_rules! methods {\n"
        "        ($($variant:ident = $name:literal => $capability:ident,)*) => {};\n"
        "    }\n"
        "    methods! {\n"
        '        HostGet = "host.get" => WORKTREES,\n'
        '        HostGetV2 = "host.get_v2" => MISSING,\n'
        '        V2BrandNew = "v2.brand_new" => HIDDEN,\n'
        '        HostGetAgain = "host.get" => WORKTREES,\n'
        "    }\n"
        "}\n"
    )
    flagged = capability_problems(scope_fixture, cap_fixture)
    for method, why in [("host.get_v2", "does not declare"), ("v2.brand_new", "not in `capability::ALL`"), ("host.get", "two rows")]:
        count += 1
        if not any(f"`{method}`" in p and why in p for p in flagged):
            failures.append(f"{method} ({why}) was not flagged: {flagged}")
    count += 1
    clean = cap_fixture.replace('        HostGetV2 = "host.get_v2" => MISSING,\n', "").replace(
        '        V2BrandNew = "v2.brand_new" => HIDDEN,\n', "").replace('        HostGetAgain = "host.get" => WORKTREES,\n', "")
    if capability_problems(scope_fixture, clean):
        failures.append(f"a clean table was flagged: {capability_problems(scope_fixture, clean)}")
    # A scope table that names methods itself again bypasses the table.
    count += 1
    hand_typed = 'fn required_scope(method: &str) -> Option<Scope> {\n    match method { "host.get" => Some(Scope::Read), _ => None }\n}\n'
    if not capability_problems(hand_typed, clean):
        failures.append("a required_scope that matches method names itself passed")
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

    # Every channel asked for here is one a workflow writes. `beta` and
    # `release` were asked for long after nothing wrote them, and passed.
    writers = {
        "preview": lambda text: re.search(r"options:\s*\[[^\]]*\bpreview\b", text),
        "stable": lambda text: re.search(r"options:\s*\[[^\]]*\bstable\b", text),
        "canary": lambda text: "scripts/canary-baseline.sh" in text,
    }
    for channel, workflow in CHANNELS.items():
        count += 1
        path = ROOT / workflow
        text = path.read_text() if path.exists() else ""
        if channel not in writers or not writers[channel](text):
            failures.append(f"nothing in {workflow} writes the {channel} baseline")

    # Canary ships every push to main, so its ship jobs must wait for this lint
    # in their own workflow. CI's `wire` job is no gate: canary.yml never waits
    # for it, and CI cancels a run when the next push lands.
    count += 1
    canary = (ROOT / CHANNELS["canary"]).read_text()
    for problem in canary_gate_problems(canary):
        failures.append(f"canary.yml: {problem}")

    # And the gate check itself, against the ways a gate is lost while every
    # `needs` line still reads correctly, plus a valid spelling it must accept.
    gate_cases = [
        ("ios's needs removed", "    runs-on: xcode-27\n    needs: wire\n", "    runs-on: xcode-27\n", 1),
        ("macos's needs removed", "    needs: linux\n", "", 1),
        ("always() on ios", "    if: vars.CANARY_TESTFLIGHT == 'true'\n", "    if: always() && vars.CANARY_TESTFLIGHT == 'true'\n", 1),
        ("!cancelled() on linux", "  linux:\n    needs: wire\n", "  linux:\n    needs: wire\n    if: ${{ !cancelled() }}\n", 1),
        ("|| true on the lint", "--channel canary\n", "--channel canary || true\n", 1),
        ("continue-on-error on the lint job", "  wire:\n    name: Wire compatibility\n", "  wire:\n    name: Wire compatibility\n    continue-on-error: true\n", 1),
        ("continue-on-error on a lint step", "      - run: ./scripts/proto-lint.py --channel canary\n", "      - run: ./scripts/proto-lint.py --channel canary\n        continue-on-error: true\n", 1),
        ("needs as a block list", "  linux:\n    needs: wire\n", "  linux:\n    needs:\n      - wire\n", 0),
        ("needs as a flow list", "    needs: linux\n", "    needs: [wire, linux]\n", 0),
    ]
    for what, old, new, want in gate_cases:
        count += 1
        if old not in canary:
            failures.append(f"gate case {what!r}: canary.yml no longer contains {old!r}")
            continue
        got = canary_gate_problems(canary.replace(old, new, 1))
        if bool(got) != bool(want):
            failures.append(f"gate case {what!r}: expected {'a problem' if want else 'none'}, got {got}")

    # Every job in every workflow has a timeout. Without one, a hang holds a
    # runner until GitHub kills it at six hours.
    workflows = sorted((ROOT / ".github" / "workflows").glob("*.yml"))
    count += 1
    if not workflows:
        failures.append("no workflows found under .github/workflows")
    for path in workflows:
        for problem in timeout_problems(path.read_text()):
            failures.append(f"{path.name}: {problem}")

    # And the timeout check itself: a planted job with none, one with an empty
    # value, and a reusable-workflow call, which cannot take one.
    planted = canary.replace("\njobs:\n", "\njobs:\n  planted:\n    runs-on: ubuntu-latest\n    steps:\n      - run: sleep 99999\n", 1)
    timeout_cases = [
        ("a job without timeout-minutes", planted, 1),
        ("an empty timeout-minutes", canary.replace("    timeout-minutes: 5\n", "    timeout-minutes:\n", 1), 1),
        ("the real canary.yml", canary, 0),
    ]
    for what, text, want in timeout_cases:
        count += 1
        if text == canary and want:
            failures.append(f"timeout case {what!r}: the mutation no longer applies to canary.yml")
            continue
        got = timeout_problems(text)
        if len(got) != want:
            failures.append(f"timeout case {what!r}: expected {want} problem(s), got {got}")

    for f in failures:
        print(f"FAIL: {f}", file=sys.stderr)
    print(f"{count - len(failures)} passed, {len(failures)} failed")
    return 1 if failures else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--channel", default="preview", choices=sorted(CHANNELS))
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--compare", nargs=2, metavar=("OLD", "NEW"), type=pathlib.Path)
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    if args.compare:
        old, new = args.compare
        problems = compare(parse(old.read_text()), parse(new.read_text()))
        verdict = f"{new} is compatible with {old}"
    else:
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
