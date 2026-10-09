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
# not by test count: PlanRulingsUITests alone is seven minutes.
#
# Five shards (ruling R-40, after main run 37771611102 went red on the shard
# budget). Each number below is a class's mean test time (suite start to suite
# end in the CI log) over main's last three runs with shards, 37743027620,
# 37760311348 and 37771611102. The five shards total 1,159 to 1,163 s each (5,804
# s in all). A shard's job adds 214 to 476 s on top of its tests (the download,
# the boot, the first launch), 375 s on average, so a shard is about 1,530 s of
# the 2,700 s timeout: 57%, where four shards ran 1,470 to 1,860 s of tests
# and 2,060 to 2,250 s as jobs (the budget job went red at `shell`, 36 min, 80%).
# The runner variance (the same class took 374 s and 557 s) is bigger than any
# split can absorb, so the bar is the 60% this was cut to, not the 75% the
# budget job (scripts/ios-ui-shard-budget.py) enforces.
#
# Where they run (ov-384): the account has five macOS runners. `rust` (macOS)
# and `swift` hold two of them for 13 to 16.5 and 17 to 25 minutes, so when
# `ios` ends only three are free. `shell`, `agent` and `phone`, the `ios-ui`
# matrix, take them. `phone2` and `phone3` are `ios-ui-late`'s two matrix
# entries, held until `rust` ends; with five shards of the same size they end
# near minute 40, and a sixth would only queue.
#
# The classes are not grouped by the harness they launch with; each class
# launches its own app, so only the seconds matter.
SHARDS = {
    "shell": [
        "NativeComposerTests",  # 488 s
        "PhoneTreeUITests",  # 305 s; -phone-harness -phone-plan (ov-300)
        "AgentEndedSessionTests",  # 158 s; -agent-layout-harness (KeyboardTabStripTests.swift)
        "NativeAnswersUITests",  # 99 s; -native-agent-harness -native-held-ask (ov-370)
        "BoardUnreadUITests",  # 89 s; -phone-harness (ov-113)
        "ReadScopeTests",  # 21 s; -phone-harness (TaskScreenTests.swift)
    ],
    "agent": [
        "PlanRulingsUITests",  # 405 s; -phone-harness -phone-plan -phone-rulings (ov-304)
        "FirstRunUITests",  # 276 s; -phone-harness (ov-205 lane P, placed by integ-9)
        "AgentEmptyStateTests",  # 207 s; -agent-layout-harness (KeyboardTabStripTests.swift)
        "ShellColumnCloseTests",  # 115 s; -shell-harness
        "AgentDraftTests",  # 113 s; -agent-layout-harness
        "ChangesPullRequestTests",  # 44 s; -changes-layout-harness
    ],
    "phone": [
        "PagesUITests",  # 431 s; -phone-harness -phone-plan -phone-pages (ov-285)
        "WorkspaceScreenTests",  # 222 s; -phone-harness
        "TaskScreenTests",  # 203 s; -phone-harness
        "AgentTranscriptScrollTests",  # 124 s; -agent-layout-harness (KeyboardTabStripTests.swift)
        "ChangesPatchNoticeTests",  # 64 s; -changes-layout-harness
        "ComposerKeyboardTests",  # 51 s; -agent-layout-harness
        "TaskUsageUITests",  # 49 s; -phone-harness (ov-195)
        "WorkingShimmerTests",  # 16 s; -agent-layout-harness (ov-382)
    ],
    # The late shards: ci.yml's `ios-ui-late`, a two-entry matrix (`phone2`, `phone3`),
    # not part of the `ios-ui` matrix. They start when `rust` ends.
    "phone2": [
        "PlanUITests",  # 372 s; -phone-harness -phone-plan (ov-274)
        "ShellGestureTests",  # 318 s; -shell-harness
        "FilesBrowserTests",  # 162 s; -phone-harness (ov-259)
        "PhoneReopenTests",  # 139 s; -phone-harness
        "WorkspaceChromeTests",  # 66 s; -phone-harness -phone-plan (ov-342)
        "PlanSheetAppearanceUITests",  # ~240 s est.; -phone-harness -phone-plan (ov-444)
        "OrchestratorImageDoorTests",  # ~60 s est.; -phone-harness (ov-444)
        "TerminalTaskKeyTests",  # 50 s; -phone-harness -phone-terminal-key (ov-215)
        "AgentRetrySendTests",  # 22 s; -agent-layout-harness
        "AgentStoppedTests",  # 22 s; -agent-layout-harness -stopped
        "TerminalLigatureTests",  # 12 s; -terminal-ligature
    ],
    "phone3": [
        "NativeAgentViewTests",  # 440 s
        "ShellPaneScrollTests",  # 210 s; -shell-harness; one live method, in SKIP
        "ActionFailureTests",  # 187 s; -phone-harness and -agent-layout-harness
        "DynamicTypeTests",  # 144 s; -agent-layout-harness
        "AgentFollowTests",  # 124 s; -agent-layout-harness (ov-383)
        "ChangesLfsNoticeTests",  # 34 s; -changes-layout-harness -lfs-pointers (ov-199)
        "RunnerReachTests",  # 15 s; seeded -hosts at an address that never answers
        "HarnessRetryTests",  # 7 s; no app (ov-397)
    ],
}

# Run locally only, each with the reason CI cannot.
LOCAL = {
    "ComposerWidthUITests": "needs an iPad simulator (fc-lanes-ipad); CI's shards run on an iPhone",
    "KeyboardTabStripTests": "needs a real iPhone; skips on any simulator",
    "NewTerminalTests": "needs the demo runner",
    "PadWorkspaceUITests": "needs an iPad simulator (fc-lanes-ipad); CI's shards run on an iPhone",
    "PolishSweepUITests": "a capture tool for the polish sweep (ov-412); skips unless TEST_RUNNER_FARCOOLER_CAPTURE_OUT is set",
    "TerminalPermissionTests": "needs the demo runner",
    "TerminalScrollTests": "needs the demo runner",
}

# Live methods inside a shard class.
SKIP = [
    "ShellPaneScrollTests/testAHorizontalSwipeOverTheLiveDiffTurnsThePage",
]

CLASS = re.compile(r"^(?:final )?class (\w+)\s*:\s*XCTestCase", re.M)
MATRIX = re.compile(r"^\s*shard:\s*\[([^\]]*)\]", re.M)


def job_block(job):
    """The text of one top-level job in ci.yml, or None."""
    found = re.search(rf"^  {job}:\n(.*?)(?=^  [\w-]+:\n|\Z)", WORKFLOW.read_text(), re.M | re.S)
    return found and found.group(1)


def shards_of(job):
    """The `shard: [...]` matrix of one job, or None if it has not exactly one."""
    block = job_block(job)
    found = MATRIX.findall(block) if block else []
    if len(found) != 1:
        return None
    return [name.strip() for name in found[0].split(",") if name.strip()]


def shards_in_workflow():
    """The shards ci.yml runs: the matrices of `ios-ui` and `ios-ui-late`. A
    shard named here and missing there would be placed, pass --check, and never
    run."""
    first, late = shards_of("ios-ui"), shards_of("ios-ui-late")
    if first is None or late is None:
        return None
    return first + late


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
    late_body = job_block("ios-ui-late")
    if not late_body or not re.search(r"^    needs:\s*\[[^\]]*\brust\b", late_body, re.M):
        problems.append(
            f"{WORKFLOW.name}: `ios-ui-late` must `needs: rust`, which holds it until a macOS "
            "runner frees (ov-384); without it the late shards may take a slot a big one waits for"
        )
    if late_body:
        if not re.search(r"^    name: iOS UI \(\$\{\{ matrix\.shard \}\}\)\s*$", late_body, re.M):
            problems.append(
                f"{WORKFLOW.name}: `ios-ui-late` must be named `iOS UI (${{{{ matrix.shard }}}})`; the "
                "budget script reads the expanded name, so each job must be named for its shard"
            )
        if not re.search(r"^          shard:\s*\$\{\{ matrix\.shard \}\}\s*$", late_body, re.M):
            problems.append(
                f"{WORKFLOW.name}: `ios-ui-late` must pass `shard: ${{{{ matrix.shard }}}}` to the "
                "action, or its jobs run a different shard from the one they are named for"
            )
        if not re.search(r"^    if:.*!cancelled\(\)", late_body, re.M):
            problems.append(
                f"{WORKFLOW.name}: `ios-ui-late` needs `!cancelled()` in its `if:`, or a red "
                "`rust` job skips the UI shards instead of letting them run"
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
