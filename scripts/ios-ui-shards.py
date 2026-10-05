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
WORKFLOW = ROOT / ".github/workflows/ci.yml"

# Measured shard times are in the `ios-ui` job's comment in ci.yml. Rebalance by
# class durations from a CI log ("Test Suite '<Class>' started/passed"), not by
# test count: PlanUITests alone is six minutes.
SHARDS = {
    # Seconds are each class's time in run 37269677267 (agent's from
    # 37267080552), test start to test end. Four shards of 940 to 1,000 s
    # (ov-301). Runner variance is large: the same class has taken 276 s and
    # 442 s, so rebalance from more than one run when a shard drifts.
    "shell": [
        "ShellGestureTests",  # -shell-harness; 442 s
        "ShellPaneScrollTests",  # -shell-harness; one live method, in SKIP; 204 s
        "ShellColumnCloseTests",  # -shell-harness; 106 s
        "TerminalLigatureTests",  # -terminal-ligature; 12 s
        "TaskScreenTests",  # -phone-harness; 152 s
        "ReadScopeTests",  # -phone-harness (TaskScreenTests.swift); 24 s
    ],
    "agent": [
        "AgentEmptyStateTests",  # -agent-layout-harness (KeyboardTabStripTests.swift); 165 s
        "AgentEndedSessionTests",  # -agent-layout-harness (KeyboardTabStripTests.swift); 111 s
        "AgentTranscriptScrollTests",  # -agent-layout-harness (KeyboardTabStripTests.swift); 107 s
        "AgentDraftTests",  # -agent-layout-harness; 125 s
        "AgentRetrySendTests",  # -agent-layout-harness; 25 s
        "AgentStoppedTests",  # -agent-layout-harness -stopped; 27 s
        "ComposerKeyboardTests",  # -agent-layout-harness; 49 s
        "DynamicTypeTests",  # -agent-layout-harness; 36 s
        "ActionFailureTests",  # -phone-harness and -agent-layout-harness; 234 s
        "TaskUsageUITests",  # -phone-harness (ov-195); 67 s
    ],
    "phone": [
        "PagesUITests",  # -phone-harness -phone-plan -phone-pages (ov-285); 650 s
        "WorkspaceScreenTests",  # -phone-harness; 221 s
        "PhoneReopenTests",  # -phone-harness; 129 s
    ],
    "phone2": [
        "PlanUITests",  # -phone-harness -phone-plan (ov-274); 282 s
        "FirstRunUITests",  # -phone-harness (ov-205 lane P, placed by integ-9); 173 s
        "ChangesPatchNoticeTests",  # -changes-layout-harness; 53 s
        "ChangesLfsNoticeTests",  # -changes-layout-harness -lfs-pointers (ov-199); 52 s
        "ChangesPullRequestTests",  # -changes-layout-harness; 44 s
        "RunnerReachTests",  # seeded -hosts at an address that never answers; 9 s
        "TerminalTaskKeyTests",  # -phone-harness -phone-terminal-key (ov-215); 53 s
        "BoardUnreadUITests",  # -phone-harness (ov-113); 127 s
        "FilesBrowserTests",  # -phone-harness (ov-259); 186 s
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
MATRIX = re.compile(r"^\s*shard:\s*\[([^\]]*)\]", re.M)


def shards_in_workflow():
    """The `ios-ui` job's matrix, as ci.yml spells it: a shard named here and
    missing there would be placed, pass --check, and never run."""
    found = MATRIX.findall(WORKFLOW.read_text())
    if len(found) != 1:
        return None
    return [name.strip() for name in found[0].split(",") if name.strip()]


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
    matrix = shards_in_workflow()
    if matrix is None:
        problems.append(f"found no single `shard: [...]` matrix in {WORKFLOW.name}")
    elif sorted(matrix) != sorted(SHARDS):
        problems.append(
            f"{WORKFLOW.name} runs shards {', '.join(matrix)} but this file has "
            f"{', '.join(SHARDS)}; make the ios-ui matrix match"
        )
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
