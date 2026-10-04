#!/usr/bin/env python3
"""Which iOS UI test classes CI runs, in which shard, and which stay local.

Every XCTestCase class in apps/ios/FarCoolerUITests is named here exactly once,
and `--check` fails if one is not. That is the point of the file: CI selects
tests with `-only-testing:FarCoolerUITests/<Class>`, which takes a CLASS, and a
file can hold several. The first version of the `ios-ui` job listed
`KeyboardTabStripTests`, whose one test always skips on a simulator, and so
silently left out the three runnerless agent classes in the same file, and
`ReadScopeTests` beside `TaskScreenTests` (ov-127 review). A class that exists
and is in no list is now a red build, not a hole.

    scripts/ios-ui-shards.py shell        # the -only-testing targets for a shard
    scripts/ios-ui-shards.py --skip       # the -skip-testing targets, all shards
    scripts/ios-ui-shards.py --check      # every class placed, none twice, none stale
    scripts/ios-ui-shards.py --list       # the table, for a report

A shard class must need no runner: it launches on an in-app harness or a
fixture flag. A live method inside one goes in SKIP. A live test that slips in
anyway skips with "NO LIVE RUNNER:", which scripts/ios-ui-tests.sh turns red.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
TESTS = ROOT / "apps/ios/FarCoolerUITests"

# Measured shard times are in the `ios-ui` job's comment in ci.yml.
SHARDS = {
    "shell": [
        "ShellGestureTests",  # -shell-harness
        "ShellColumnCloseTests",  # -shell-harness
        "ShellPaneScrollTests",  # -shell-harness; one live method, in SKIP
        "TerminalLigatureTests",  # -terminal-ligature
        "AgentTranscriptScrollTests",  # -agent-layout-harness (KeyboardTabStripTests.swift)
        "AgentEmptyStateTests",  # -agent-layout-harness (KeyboardTabStripTests.swift)
        "AgentEndedSessionTests",  # -agent-layout-harness (KeyboardTabStripTests.swift)
    ],
    "phone": [
        "WorkspaceScreenTests",  # -phone-harness
        "BoardUnreadUITests",  # -phone-harness (ov-113)
        "TaskScreenTests",  # -phone-harness
        "ReadScopeTests",  # -phone-harness (TaskScreenTests.swift)
        "TaskUsageUITests",  # -phone-harness (ov-195, landed after this lane branched)
        "PhoneReopenTests",  # -phone-harness
        "FilesBrowserTests",  # -phone-harness (ov-259)
        "AgentDraftTests",  # -agent-layout-harness
        "ChangesPullRequestTests",  # -changes-layout-harness
        "ChangesPatchNoticeTests",  # -changes-layout-harness
        "AgentRetrySendTests",  # -agent-layout-harness
        "AgentStoppedTests",  # -agent-layout-harness -stopped
        "ActionFailureTests",  # -phone-harness and -agent-layout-harness
        "ComposerKeyboardTests",  # -agent-layout-harness
        "DynamicTypeTests",  # -agent-layout-harness
        "RunnerReachTests",  # seeded -hosts at an address that never answers
        "FirstRunUITests",  # -phone-harness (ov-205 lane P, placed by integ-9)
    ],
}

# Run locally only, each with the reason CI cannot.
LOCAL = {
    "KeyboardTabStripTests": "needs a real iPhone; skips on any simulator",
    "NewTerminalTests": "needs the demo runner",
    "TerminalPermissionTests": "needs the demo runner",
    "TerminalScrollTests": "needs the demo runner",
}

# Live methods inside a shard class.
SKIP = [
    "ShellPaneScrollTests/testAHorizontalSwipeOverTheLiveDiffTurnsThePage",
]

CLASS = re.compile(r"^(?:final )?class (\w+)\s*:\s*XCTestCase", re.M)


def classes_on_disk():
    found = {}
    for path in sorted(TESTS.glob("*.swift")):
        for name in CLASS.findall(path.read_text()):
            found[name] = path.name
    return found


def check():
    problems = []
    placed = [c for shard in SHARDS.values() for c in shard] + list(LOCAL)
    for name in sorted({c for c in placed if placed.count(c) > 1}):
        problems.append(f"{name} is placed more than once")
    on_disk = classes_on_disk()
    for name, file in on_disk.items():
        if name not in placed:
            problems.append(
                f"{name} ({file}) is in no shard and not in LOCAL; place it in "
                "scripts/ios-ui-shards.py"
            )
    for name in placed:
        if name not in on_disk:
            problems.append(f"{name} is listed but no such XCTestCase class exists")
    for target in SKIP:
        name, _, method = target.partition("/")
        if name not in on_disk:
            problems.append(f"SKIP names {target}, whose class does not exist")
            continue
        source = (TESTS / on_disk[name]).read_text()
        if f"func {method}(" not in source:
            problems.append(f"SKIP names {target}, which no longer exists")
    if not on_disk:
        problems.append(f"no XCTestCase classes found under {TESTS}")
    for problem in problems:
        print(f"ios-ui-shards: {problem}", file=sys.stderr)
    if not problems:
        print(f"ios-ui-shards: {len(on_disk)} classes, each placed once")
    return 1 if problems else 0


def main(argv):
    if argv == ["--check"]:
        return check()
    if argv == ["--skip"]:
        print(" ".join(f"FarCoolerUITests/{t}" for t in SKIP))
        return 0
    if argv == ["--list"]:
        for shard, names in SHARDS.items():
            print(f"{shard}: {', '.join(names)}")
        for name, why in LOCAL.items():
            print(f"local: {name} ({why})")
        print(f"skipped: {', '.join(SKIP)}")
        return 0
    if len(argv) == 1 and argv[0] in SHARDS:
        print(" ".join(f"FarCoolerUITests/{c}" for c in SHARDS[argv[0]]))
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
