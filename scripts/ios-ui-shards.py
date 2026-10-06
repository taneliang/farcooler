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

# Rebalance by class durations from a CI log ("Test Suite '<Class>' started/passed"),
# not by test count: PlanRulingsUITests alone is eleven minutes.
#
# The shards are NOT the same size, on purpose (ov-384). The account has five
# macOS runners; `rust` (macOS) and `swift` hold two of them for 16 and 25
# minutes, so when `ios` ends only three are free. `shell`, `agent` and `phone`
# start then and are the big ones, about 1,350 s of tests each. `phone2` runs
# in its own job (`ios-ui-late` in ci.yml), held until `rust` ends, and is the
# small one, about 750 s, so that all four end together near 35 minutes. Four
# equal shards started the fourth at 16 minutes and ended it at 43 to 52.
# Move a class between the big three to level them; change `phone2` only with
# that in mind. A shard's job costs about 450 s on top of its tests, and the
# budget job (scripts/ios-ui-shard-budget.py) goes red at 75% of the timeout,
# so about 1,350 s of tests is the most a big shard can hold.
SHARDS = {
    # Seconds are the mean of each class's time in runs 37503739914 and
    # 37509877259 (test start to test end), rounded. Runner variance is large,
    # +/- 25%: the same class took 343 s and 275 s, so rebalance from more than
    # one run when a shard drifts. Shard totals: shell 1,375 s, agent 1,366 s,
    # phone 1,320 s, phone2 740 s.
    "shell": [
        "PlanRulingsUITests",  # -phone-harness -phone-plan -phone-rulings (ov-304); 646 s
        "ShellGestureTests",  # -shell-harness; 309 s
        "ShellPaneScrollTests",  # -shell-harness; one live method, in SKIP; 205 s
        "ShellColumnCloseTests",  # -shell-harness; 132 s
        "TaskUsageUITests",  # -phone-harness (ov-195); 49 s
        "ReadScopeTests",  # -phone-harness (TaskScreenTests.swift); 25 s
        "TerminalLigatureTests",  # -terminal-ligature; 9 s
    ],
    "agent": [
        "PhoneTreeUITests",  # -phone-harness -phone-plan (ov-300); 327 s
        "ActionFailureTests",  # -phone-harness and -agent-layout-harness; 276 s
        "TaskScreenTests",  # -phone-harness; 196 s
        "AgentEmptyStateTests",  # -agent-layout-harness (KeyboardTabStripTests.swift); 146 s
        "AgentTranscriptScrollTests",  # -agent-layout-harness (KeyboardTabStripTests.swift); 114 s
        "AgentFollowTests",  # -agent-layout-harness (ov-383); not yet timed, 2 tests
        "WorkingShimmerTests",  # -agent-layout-harness (ov-382); not yet timed, 1 test
        "AgentDraftTests",  # -agent-layout-harness; 102 s
        "AgentEndedSessionTests",  # -agent-layout-harness (KeyboardTabStripTests.swift); 89 s
        "ComposerKeyboardTests",  # -agent-layout-harness; 38 s
        "DynamicTypeTests",  # -agent-layout-harness; 33 s
        "AgentStoppedTests",  # -agent-layout-harness -stopped; 25 s
        "AgentRetrySendTests",  # -agent-layout-harness; 20 s
    ],
    "phone": [
        "PagesUITests",  # -phone-harness -phone-plan -phone-pages (ov-285); 536 s
        "PlanUITests",  # -phone-harness -phone-plan (ov-274); 398 s
        "WorkspaceScreenTests",  # -phone-harness; 215 s
        "PhoneReopenTests",  # -phone-harness; 110 s
        "WorkspaceChromeTests",  # -phone-harness -phone-plan (ov-342); 61 s
    ],
    # The late shard: ci.yml's `ios-ui-late`, not a matrix entry.
    "phone2": [
        "FirstRunUITests",  # -phone-harness (ov-205 lane P, placed by integ-9); 206 s
        "FilesBrowserTests",  # -phone-harness (ov-259); 158 s
        "BoardUnreadUITests",  # -phone-harness (ov-113); 117 s
        "ChangesPatchNoticeTests",  # -changes-layout-harness; 82 s
        "TerminalTaskKeyTests",  # -phone-harness -phone-terminal-key (ov-215); 72 s
        "ChangesPullRequestTests",  # -changes-layout-harness; 51 s
        "ChangesLfsNoticeTests",  # -changes-layout-harness -lfs-pointers (ov-199); 44 s
        "RunnerReachTests",  # seeded -hosts at an address that never answers; 10 s
    ],
}

# Run locally only, each with the reason CI cannot.
LOCAL = {
    "KeyboardTabStripTests": "needs a real iPhone; skips on any simulator",
    "NewTerminalTests": "needs the demo runner",
    "PadWorkspaceUITests": "needs an iPad simulator (fc-lanes-ipad); CI's shards run on an iPhone",
    "TerminalPermissionTests": "needs the demo runner",
    "TerminalScrollTests": "needs the demo runner",
}

# Live methods inside a shard class.
SKIP = [
    "ShellPaneScrollTests/testAHorizontalSwipeOverTheLiveDiffTurnsThePage",
]

CLASS = re.compile(r"^(?:final )?class (\w+)\s*:\s*XCTestCase", re.M)
MATRIX = re.compile(r"^\s*shard:\s*\[([^\]]*)\]", re.M)
LATE = re.compile(r"^    name: iOS UI \((\w+)\)\s*$", re.M)


def shards_in_workflow():
    """The shards ci.yml runs: the `ios-ui` matrix plus every job named for one
    shard outright (`ios-ui-late`). A shard named here and missing there would
    be placed, pass --check, and never run."""
    found = MATRIX.findall(WORKFLOW.read_text())
    if len(found) != 1:
        return None
    names = [name.strip() for name in found[0].split(",") if name.strip()]
    return names + LATE.findall(WORKFLOW.read_text())


def timeouts_in_workflow():
    """`timeout-minutes` of `ios-ui` and `ios-ui-late`, in that order, or None
    for one that is missing. The budget script reads the first only, so the
    second must not be allowed a longer one."""
    text = WORKFLOW.read_text()
    out = []
    for job in ("ios-ui", "ios-ui-late"):
        block = re.search(rf"^  {job}:\n(.*?)(?=^  [\w-]+:\n|\Z)", text, re.M | re.S)
        found = block and re.search(r"^    timeout-minutes:\s*(\d+)\s*$", block.group(1), re.M)
        out.append(int(found.group(1)) if found else None)
    return out


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
    late_job = re.search(r"^  ios-ui-late:\n(.*?)(?=^  [\w-]+:\n|\Z)", WORKFLOW.read_text(), re.M | re.S)
    if not late_job or not re.search(r"^    needs:\s*\[[^\]]*\brust\b", late_job.group(1), re.M):
        problems.append(
            f"{WORKFLOW.name}: `ios-ui-late` must `needs: rust`, which holds it until a macOS "
            "runner frees (ov-384); without it the small shard may take a slot a big one waits for"
        )
    first, late = timeouts_in_workflow()
    if first is None or late is None or first != late:
        problems.append(
            f"{WORKFLOW.name}: `ios-ui` has timeout-minutes {first} and `ios-ui-late` {late}; "
            "they must both exist and match, because the budget script reads only `ios-ui`'s"
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
