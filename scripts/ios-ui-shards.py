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
# Four shards of about 1,200 s of tests each (ov-384). A shard's job costs 440 to
# 570 s on top of its tests, and the budget job (scripts/ios-ui-shard-budget.py)
# goes red at 75% of the timeout (30 minutes), so 1,200 s is about 72% at the
# worst overhead measured, and 1,350 s is the most any shard can hold.
#
# `phone2` runs in its own job (`ios-ui-late` in ci.yml), held until `rust`
# ends; the other three start when `ios` ends. The account has five macOS
# runners and `rust` (macOS) and `swift` hold two of them for 16 and 22 to 25
# minutes, so only three are free at that point, and which of four equal
# shards got a slot was arbitrary. Holding the same one every time keeps the
# order deterministic, but it ends near minute 43 because it starts at 15, so
# the layout alone saves little against the old arbitrary order (see
# .claude/agent/reports/ci-queueing/report.md). Smaller big shards would end
# sooner but the 75% bar forbids bigger ones.
SHARDS = {
    # Seconds are the mean of each class's time in runs 37503739914,
    # 37509877259 and 37523241692 (test start to test end), rounded. Runner
    # variance is large, +/- 25%: PagesUITests took 503 s and 762 s, so
    # rebalance from more than one run when a shard drifts. Shard totals:
    # shell 1,195 s, agent 1,180 s, phone 1,191 s, phone2 1,220 s. AgentFollowTests
    # (60 s) and WorkingShimmerTests (30 s) are estimates: not yet timed.
    "shell": [
        "PlanRulingsUITests",  # -phone-harness -phone-plan -phone-rulings (ov-304); 583 s
        "ShellGestureTests",  # -shell-harness; 290 s
        "ShellPaneScrollTests",  # -shell-harness; one live method, in SKIP; 195 s
        "ShellColumnCloseTests",  # -shell-harness; 119 s
        "TerminalLigatureTests",  # -terminal-ligature; 9 s
    ],
    "agent": [
        "PhoneTreeUITests",  # -phone-harness -phone-plan (ov-300); 298 s
        "ActionFailureTests",  # -phone-harness and -agent-layout-harness; 239 s
        "AgentEmptyStateTests",  # -agent-layout-harness (KeyboardTabStripTests.swift); 146 s
        "AgentTranscriptScrollTests",  # -agent-layout-harness (KeyboardTabStripTests.swift); 109 s
        "AgentFollowTests",  # -agent-layout-harness (ov-383); not yet timed, 2 tests
        "WorkingShimmerTests",  # -agent-layout-harness (ov-382); not yet timed, 1 test
        "AgentDraftTests",  # -agent-layout-harness; 95 s
        "AgentEndedSessionTests",  # -agent-layout-harness (KeyboardTabStripTests.swift); 86 s
        "ComposerKeyboardTests",  # -agent-layout-harness; 36 s
        "DynamicTypeTests",  # -agent-layout-harness; 32 s
        "AgentStoppedTests",  # -agent-layout-harness -stopped; 28 s
        "AgentRetrySendTests",  # -agent-layout-harness; 21 s
    ],
    "phone": [
        "PagesUITests",  # -phone-harness -phone-plan -phone-pages (ov-285); 612 s
        "PlanUITests",  # -phone-harness -phone-plan (ov-274); 395 s
        "PhoneReopenTests",  # -phone-harness; 113 s
        "TaskUsageUITests",  # -phone-harness (ov-195); 47 s
        "ReadScopeTests",  # -phone-harness (TaskScreenTests.swift); 24 s
    ],
    # The late shard: ci.yml's `ios-ui-late`, not a matrix entry.
    "phone2": [
        "FirstRunUITests",  # -phone-harness (ov-205 lane P, placed by integ-9); 213 s
        "FilesBrowserTests",  # -phone-harness (ov-259); 159 s
        "BoardUnreadUITests",  # -phone-harness (ov-113); 112 s
        "ChangesPatchNoticeTests",  # -changes-layout-harness; 80 s
        "TerminalTaskKeyTests",  # -phone-harness -phone-terminal-key (ov-215); 69 s
        "ChangesPullRequestTests",  # -changes-layout-harness; 49 s
        "ChangesLfsNoticeTests",  # -changes-layout-harness -lfs-pointers (ov-199); 48 s
        "WorkspaceChromeTests",  # -phone-harness -phone-plan (ov-342); 68 s
        "RunnerReachTests",  # seeded -hosts at an address that never answers; 11 s
        "TaskScreenTests",  # -phone-harness; 190 s
        "WorkspaceScreenTests",  # -phone-harness; 223 s
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
    if late_job:
        body = late_job.group(1)
        named = re.search(r"^    name: iOS UI \((\w+)\)\s*$", body, re.M)
        passed = re.search(r"^          shard:\s*(\S+)\s*$", body, re.M)
        if not named or not passed or named.group(1) != passed.group(1):
            problems.append(
                f"{WORKFLOW.name}: `ios-ui-late` is named for shard "
                f"{named and named.group(1)} but runs {passed and passed.group(1)}; the budget "
                "script reads the name, so they must be the same shard"
            )
        if not re.search(r"^    if:.*!cancelled\(\)", body, re.M):
            problems.append(
                f"{WORKFLOW.name}: `ios-ui-late` needs `!cancelled()` in its `if:`, or a red "
                "`rust` job skips the UI shard instead of letting it run"
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
