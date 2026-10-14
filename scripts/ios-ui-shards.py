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
    scripts/ios-ui-shards.py smoke        # ... for the per-push smoke set
    scripts/ios-ui-shards.py captures     # ... for the nightly capture classes
    scripts/ios-ui-shards.py --skip       # the -skip-testing targets, all shards
    scripts/ios-ui-shards.py --check      # every class placed, none twice, none stale
    scripts/ios-ui-shards.py --list       # the table, for a report
    scripts/ios-ui-shards.py --self-test  # --check against planted mistakes

Three tiers (ov-534, owner, Oct 14). Every push to main and every pull request
runs the SMOKE set, one job in ci.yml. The NIGHTLY run (nightly.yml, on main,
on a schedule) runs every shard below and the CAPTURES classes. The train gate
runs the classes a diff touches plus SMOKE. Every class is in exactly one of a
shard, LOCAL or CAPTURES; SMOKE names classes (or single methods) that are also
in a shard, so the nightly run still covers them.

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
NIGHTLY = ROOT / ".github/workflows/nightly.yml"

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
# Where they run (ov-534): nightly.yml's `ios-ui` job, one five-entry matrix,
# once a night on main. Until Oct 14 they ran on every push, in two jobs
# because `rust` and `swift` held two of the account's five macOS runners
# (ov-384); with nothing else running at night, five at once is the whole pool.
#
# The classes are not grouped by the harness they launch with; each class
# launches its own app, so only the seconds matter.
SHARDS = {
    "shell": [
        "NativeComposerTests",  # 488 s
        "PhoneTreeUITests",  # 305 s; -phone-harness -phone-plan (ov-300)
        "NativeAnswersUITests",  # 99 s; -native-agent-harness -native-held-ask (ov-370)
        "BoardUnreadUITests",  # 89 s; -phone-harness (ov-113)
        "ReadScopeTests",  # 21 s; -phone-harness (TaskScreenTests.swift)
        "NativeAgentViewTests",  # 440 s Moved from phone3 (Train 114: phone3 at 77%).
    ],
    "agent": [
        "DynamicTypeTests",  # 144 s; -phone-harness
        "PlanSheetAppearanceUITests",  # ~240 s est.; -phone-harness -phone-plan (ov-444)
        "PlanRulingsUITests",  # ~96 s est. (405 s before ov-538); -phone-harness -phone-plan -phone-rulings (ov-304)
        "FirstRunUITests",  # 276 s; -phone-harness (ov-205 lane P, placed by integ-9)
        "ShellColumnCloseTests",  # 115 s; -shell-harness
        "ChangesPullRequestTests",  # 44 s; -changes-layout-harness
    ],
    "phone": [
        "PhoneReopenTests",  # 139 s; -phone-harness
        "PagesUITests",  # 431 s; -phone-harness -phone-plan -phone-pages (ov-285)
        "WorkspaceScreenTests",  # 222 s; -phone-harness
        "TaskScreenTests",  # 203 s; -phone-harness
        "ChangesPatchNoticeTests",  # 64 s; -changes-layout-harness
        "TaskUsageUITests",  # 49 s; -phone-harness (ov-195)
        "RunnerPeopleUITests",  # ~60 s est.; -phone-harness -phone-shared (ov-508)
    ],
    "phone2": [
        "PlanUITests",  # ~119 s est. (372 s before ov-538); -phone-harness -phone-plan (ov-274)
        "ShellGestureTests",  # 318 s; -shell-harness
        "FilesBrowserTests",  # 162 s; -phone-harness (ov-259)
        "WorkspaceChromeTests",  # 66 s; -phone-harness -phone-plan (ov-342)
        "OrchestratorImageDoorTests",  # ~60 s est.; -phone-harness (ov-444)
        "TerminalTaskKeyTests",  # 50 s; -phone-harness -phone-terminal-key (ov-215)
        "TalkToRepUITests",  # ~150 s measured here; -phone-harness -phone-rep (ov-526)
        "TerminalLigatureTests",  # 12 s; -terminal-ligature
        "ShellPaneScrollTests",  # 210 s; -shell-harness; one live method, in SKIP. Moved from phone3 (Train 111: phone3 at 78%)
        "ActionFailureTests",  # 187 s; -phone-harness
    ],
    "phone3": [
        "NativeAgentTrayUITests",  # ~70 s est.; -native-agent-harness -native-agents (ov-453)
        "PlanSectionsUITests",  # ~101 s est. (~430 s before ov-538); -phone-harness -phone-plan (ov-503)
        "ChangesLfsNoticeTests",  # 34 s; -changes-layout-harness -lfs-pointers (ov-199)
        "RunnerReachTests",  # 15 s; seeded -hosts at an address that never answers
        "TunnelRouteOfferTests",  # ~30 s est.; seeded -hosts and -runnerAlternatives (ov-485)
        "HarnessRetryTests",  # 7 s; no app (ov-397)
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

# Capture tools, run by nightly.yml's `captures` job with
# TEST_RUNNER_FARCOOLER_CAPTURE_OUT set (they skip every method without it) and
# uploaded as an artifact. They are phone-harness classes, so CI can run them;
# they sit outside the shards because they assert nothing about the product.
CAPTURES = {
    "PlanSectionsCapturesUITests": "the Plan's sections on an iPhone (ov-503)",
    "PolishSweepUITests": "each shipped screen at its extremes, light and dark (ov-412)",
}

# The per-push smoke set (ov-534): the core flows, as few seconds as will show
# that the app launches and its main screens draw and take input. Each is a
# class or a single `Class/method`, and a shard above holds the class, so the
# nightly run covers the rest of it. The number is the seconds that test took
# in main run 38563769221 (Oct 14, the shard logs: "Test Case ... passed (N
# seconds)"); --check holds the total under SMOKE_BUDGET_S.
#
# Why that budget: the smoke job's overhead before its first test was 252 to
# 452 s in the same run (download, boot, first launch), so 184 s of tests is a
# 7.3 to 10.6 minute job, and the aim is 10. A whole class does not fit for
# most flows (NativeComposerTests is 632 s, PlanUITests 461 s), which is why
# some entries are single methods.
#
# What each covers: launch and first run; a saved runner that cannot be
# reached; the workspace screen; the composer (a slash command, a photo); the
# Plan, and the band that says what needs you; answering a held question and a
# decision, which is Needs You. There is no pairing or join UI test in this
# suite (those flows are tested in AgentKit and the daemon), so none is here.
SMOKE_BUDGET_S = 200
SMOKE = {
    "FirstRunUITests/testOnboardingTeachesWhatARunnerIsFor": 13.0,
    "RunnerReachTests": 15.1,
    "WorkspaceScreenTests/testWorkspaceShowsOrchestratorBoardAndWorktrees": 31.5,
    "NativeComposerTests/testASlashCommandIsSent": 24.2,
    "NativeComposerTests/testAPhotoAloneCanBeSent": 19.1,
    "PlanUITests/testARunnerWithoutBoardPlanShowsNothingNew": 19.6,
    "PlanSectionsUITests/testTheBandSaysNowAndNextAndWhatNeedsYou": 20.3,
    "NativeAnswersUITests/testAQuestionIsAnsweredWithAnOption": 29.8,
    "TaskScreenTests/testAnsweringADecisionRemovesItFromNeedsYou": 11.5,
}

# Live methods inside a shard class.
SKIP = [
    "ShellPaneScrollTests/testAHorizontalSwipeOverTheLiveDiffTurnsThePage",
]

CLASS = re.compile(r"^(?:final )?class (\w+)\s*:\s*XCTestCase", re.M)
MATRIX = re.compile(r"^\s*shard:\s*\[([^\]]*)\]", re.M)


# Planted workflow text, for --self-test: {path: text}.
OVERRIDE = {}


def read(path):
    return OVERRIDE[path] if path in OVERRIDE else path.read_text()


def job_block(job, workflow=WORKFLOW):
    """The text of one top-level job in a workflow file, or None."""
    found = re.search(rf"^  {job}:\n(.*?)(?=^  [\w-]+:\n|\Z)", read(workflow), re.M | re.S)
    return found and found.group(1)


def shards_of(job, workflow=NIGHTLY):
    """The `shard: [...]` matrix of one job, or None if it has not exactly one."""
    block = job_block(job, workflow)
    found = MATRIX.findall(block) if block else []
    if len(found) != 1:
        return None
    return [name.strip() for name in found[0].split(",") if name.strip()]


def classes_on_disk():
    found = {}
    for path in sorted(TESTS.glob("*.swift")):
        for name in CLASS.findall(path.read_text()):
            found[name] = path.name
    return found


def smoke_targets():
    return [f"FarCoolerUITests/{t}" for t in SMOKE]


def check_workflows(problems):
    """What ci.yml (per push) and nightly.yml (nightly) must be true of."""
    # Per push: the smoke set in one job, and no shard matrix at all.
    if re.search(MATRIX, read(WORKFLOW)):
        problems.append(
            f"{WORKFLOW.name} has a `shard: [...]` matrix; the full shards run in "
            f"{NIGHTLY.name} only (ov-534), and per push runs SMOKE"
        )
    smoke_job = job_block("ios-ui")
    if not smoke_job:
        problems.append(f"{WORKFLOW.name} has no `ios-ui` job to run the smoke set")
    else:
        if not re.search(r"^          shard:\s*smoke\s*$", smoke_job, re.M):
            problems.append(f"{WORKFLOW.name}: `ios-ui` must pass `shard: smoke` to the action")
        if not re.search(r"^    name: iOS UI \(smoke\)\s*$", smoke_job, re.M):
            problems.append(f"{WORKFLOW.name}: `ios-ui` must be named `iOS UI (smoke)`")
        found = re.search(r"^    timeout-minutes:\s*(\d+)\s*$", smoke_job, re.M)
        if not found or int(found.group(1)) > 20:
            problems.append(f"{WORKFLOW.name}: `ios-ui` needs a timeout-minutes of 20 or less")
    # Nightly: every shard, in one matrix, named for its shard.
    if not NIGHTLY.exists() and NIGHTLY not in OVERRIDE:
        problems.append(f"{NIGHTLY.name} does not exist")
        return
    text = read(NIGHTLY)
    cron = re.search(r"^  schedule:\n\s+- cron:\s*\"([^\"]+)\"", text, re.M)
    if not cron:
        problems.append(f"{NIGHTLY.name} has no `schedule:` trigger with a quoted cron")
    else:
        fields = cron.group(1).split()
        ok = len(fields) == 5 and fields[0].isdigit() and fields[1].isdigit() and fields[2:] == ["*", "*", "*"]
        if not ok or int(fields[0]) % 15 == 0 or not 8 <= int(fields[1]) <= 13:
            problems.append(
                f"{NIGHTLY.name}: cron `{cron.group(1)}` must be daily, at an off-minute (not :00, :15, :30 "
                "or :45, which GitHub delays) between 08:00 and 13:59 UTC, the early morning in Pacific time"
            )
    if not re.search(r"^  workflow_dispatch:", text, re.M):
        problems.append(f"{NIGHTLY.name} has no `workflow_dispatch:`; a night cannot be run by hand")
    report = job_block("report", NIGHTLY)
    if not report:
        problems.append(f"{NIGHTLY.name} has no `report` job; a red night would tell no one")
    else:
        if not re.search(r"^    if:.*\balways\(\)", report, re.M):
            problems.append(f"{NIGHTLY.name}: `report` needs `always()` in its `if:`, or a red job skips it")
        if not re.search(r"^      issues:\s*write\s*$", report, re.M):
            problems.append(f"{NIGHTLY.name}: `report` needs `issues: write` to open the issue")
        needs = re.search(r"^    needs:\s*\[([^\]]*)\]", report, re.M)
        listed = {n.strip() for n in needs.group(1).split(",")} if needs else set()
        others = set(re.findall(r"^  ([\w-]+):\n", text.split("\njobs:\n", 1)[-1], re.M)) - {"report"}
        if others - listed:
            problems.append(
                f"{NIGHTLY.name}: `report` does not wait for {', '.join(sorted(others - listed))}; "
                "it would report before they finish"
            )
    matrix = shards_of("ios-ui")
    if matrix is None:
        problems.append(f"found no single `shard: [...]` matrix in {NIGHTLY.name}'s `ios-ui` job")
    elif sorted(matrix) != sorted(SHARDS):
        problems.append(
            f"{NIGHTLY.name} runs shards {', '.join(matrix)} but this file has "
            f"{', '.join(SHARDS)}; make the ios-ui matrix match"
        )
    body = job_block("ios-ui", NIGHTLY)
    if body:
        if not re.search(r"^    name: iOS UI \(\$\{\{ matrix\.shard \}\}\)\s*$", body, re.M):
            problems.append(
                f"{NIGHTLY.name}: `ios-ui` must be named `iOS UI (${{{{ matrix.shard }}}})`; the "
                "budget script reads the expanded name, so each job must be named for its shard"
            )
        if not re.search(r"^          shard:\s*\$\{\{ matrix\.shard \}\}\s*$", body, re.M):
            problems.append(
                f"{NIGHTLY.name}: `ios-ui` must pass `shard: ${{{{ matrix.shard }}}}` to the "
                "action, or its jobs run a different shard from the one they are named for"
            )
        if not re.search(r"^    timeout-minutes:\s*\d+\s*$", body, re.M):
            problems.append(f"{NIGHTLY.name}: `ios-ui` has no timeout-minutes, which the budget script reads")
    captures = job_block("captures", NIGHTLY)
    if not captures or not re.search(r"^          shard:\s*captures\s*$", captures, re.M):
        problems.append(f"{NIGHTLY.name}: no `captures` job passing `shard: captures` to the action")


def check():
    problems = []
    placed = [c for shard in SHARDS.values() for c in shard] + list(LOCAL) + list(CAPTURES)
    for name in sorted({c for c in placed if placed.count(c) > 1}):
        problems.append(f"{name} is placed more than once")
    on_disk = classes_on_disk()
    for name, file in on_disk.items():
        if name not in placed:
            problems.append(
                f"{name} ({file}) is in no shard, not in LOCAL and not in CAPTURES; place it in "
                "scripts/ios-ui-shards.py"
            )
    for name in placed:
        if name not in on_disk:
            problems.append(f"{name} is listed but no such XCTestCase class exists")
    in_shards = {c for shard in SHARDS.values() for c in shard}
    for target in SKIP:
        name, _, method = target.partition("/")
        if name not in on_disk:
            problems.append(f"SKIP names {target}, whose class does not exist")
            continue
        source = (TESTS / on_disk[name]).read_text()
        if f"func {method}(" not in source:
            problems.append(f"SKIP names {target}, which no longer exists")
    for target, took in SMOKE.items():
        name, _, method = target.partition("/")
        if name not in on_disk:
            problems.append(f"SMOKE names {target}, whose class does not exist")
            continue
        if name not in in_shards:
            problems.append(
                f"SMOKE names {target}, but {name} is in no shard; the nightly run must cover "
                "everything the smoke set does"
            )
        if method and f"func {method}(" not in (TESTS / on_disk[name]).read_text():
            problems.append(f"SMOKE names {target}, which no longer exists")
        if any(s == target or s.startswith(target + "/") for s in SKIP):
            problems.append(f"SMOKE names {target}, which holds a SKIP method (a live test)")
        if not isinstance(took, (int, float)) or took <= 0:
            problems.append(f"SMOKE {target} has no measured seconds")
    if not SMOKE:
        problems.append("SMOKE is empty; a push would run no UI test at all")
    total = sum(t for t in SMOKE.values() if isinstance(t, (int, float)))
    if total > SMOKE_BUDGET_S:
        problems.append(
            f"SMOKE totals {total:.0f} s of tests, over its {SMOKE_BUDGET_S} s budget (the "
            "job aims at 10 minutes with 4 to 8 of overhead); trim it or measure again"
        )
    if not on_disk:
        problems.append(f"no XCTestCase classes found under {TESTS}")
    check_workflows(problems)
    for problem in problems:
        print(f"ios-ui-shards: {problem}", file=sys.stderr)
    if not problems:
        print(
            f"ios-ui-shards: {len(on_disk)} classes, each placed once; "
            f"smoke {len(SMOKE)} tests, {total:.0f} s"
        )
    return 1 if problems else 0


def self_test():
    """--check against planted mistakes: each must be found, the real tree must not."""
    global SHARDS, SMOKE, CAPTURES
    real = (SHARDS, SMOKE, CAPTURES, dict(LOCAL))
    failures = []

    def run(what, want_in_output, mutate):
        global SHARDS, SMOKE, CAPTURES
        SHARDS = {k: list(v) for k, v in real[0].items()}
        SMOKE, CAPTURES = dict(real[1]), dict(real[2])
        LOCAL.clear()
        LOCAL.update(real[3])
        mutate()
        import contextlib
        import io

        err = io.StringIO()
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
            code = check()
        SHARDS, SMOKE, CAPTURES = real[0], real[1], real[2]
        if want_in_output is None:
            if code != 0:
                failures.append(f"{what}: the real tree failed: {err.getvalue().strip()}")
        elif code == 0 or want_in_output not in err.getvalue():
            failures.append(f"{what}: wanted {want_in_output!r}, got {err.getvalue().strip()!r}")

    run("the real tree passes", None, lambda: None)
    run("a class placed nowhere", "is in no shard", lambda: SHARDS["shell"].remove("ReadScopeTests"))
    run("a class placed twice", "placed more than once", lambda: SHARDS["agent"].append("ReadScopeTests"))
    run("a stale class", "no such XCTestCase class", lambda: SHARDS["agent"].append("NoSuchTests"))
    run("a capture class dropped", "is in no shard", lambda: CAPTURES.pop("PolishSweepUITests"))
    run("a smoke class in no shard", "the nightly run must cover", lambda: (
        SHARDS["phone2"].remove("PlanUITests"), LOCAL.__setitem__("PlanUITests", "x")
    ))
    run("a stale smoke method", "which no longer exists", lambda: SMOKE.__setitem__("RunnerReachTests/testNope", 5.0))
    run("a smoke entry with no seconds", "no measured seconds", lambda: SMOKE.__setitem__("RunnerReachTests", 0))
    run("a smoke set over budget", "over its", lambda: SMOKE.__setitem__("PlanUITests", 400.0))
    run("an empty smoke set", "SMOKE is empty", lambda: SMOKE.clear())
    LOCAL.clear()
    LOCAL.update(real[3])

    def planted(what, want, edit, path=NIGHTLY):
        text = path.read_text()
        changed = edit(text)
        if changed == text:
            failures.append(f"{what}: the planted edit changed nothing")
            return
        OVERRIDE.clear()
        OVERRIDE[path] = changed
        run(what, want, lambda: None)
        OVERRIDE.clear()

    planted("report without issues: write", "issues: write", lambda t: t.replace("      issues: write", "      issues: read"))
    planted("report without always()", "always()", lambda t: t.replace("if: ${{ always() }}", "if: ${{ failure() }}"))
    planted("no workflow_dispatch", "workflow_dispatch", lambda t: t.replace("  workflow_dispatch:\n", ""))
    planted("a cron on the hour", "off-minute", lambda t: t.replace('"23 11 * * *"', '"0 11 * * *"'))
    planted("a cron at noon Pacific", "08:00 and 13:59", lambda t: t.replace('"23 11 * * *"', '"23 20 * * *"'))
    planted("report not waiting for a job", "does not wait for", lambda t: t.replace("needs: [ios, ios-ui, captures, budget]", "needs: [ios, ios-ui]"))
    planted("no captures job", "no `captures` job", lambda t: t.replace("shard: captures", "shard: shell"))
    for failure in failures:
        print(f"ios-ui-shards self-test: {failure}", file=sys.stderr)
    if failures:
        return 1
    print("ios-ui-shards self-test: ok")
    return 0


def main(argv):
    if argv == ["--check"]:
        return check()
    if argv == ["--self-test"]:
        return self_test()
    if argv == ["--skip"]:
        print(" ".join(f"FarCoolerUITests/{t}" for t in SKIP))
        return 0
    if argv == ["smoke"]:
        print(" ".join(smoke_targets()))
        return 0
    if argv == ["captures"]:
        print(" ".join(f"FarCoolerUITests/{c}" for c in CAPTURES))
        return 0
    if argv == ["--list"]:
        for shard, names in SHARDS.items():
            print(f"{shard}: {', '.join(names)}")
        print(f"smoke: {', '.join(SMOKE)}")
        print(f"captures: {', '.join(CAPTURES)}")
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
