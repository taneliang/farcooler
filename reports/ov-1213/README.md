# ov-1213: waiting engine stack on main

Base: `73974f89898d716c36595eade2511ba5f8df7a8f` (short identity `73974f898`; see `git rev-parse origin/main`).
Remote main was fetched before integration and matched the starting branch.
All twelve tickets are included. No ticket was forced through an unresolved conflict.
The original branches were left untouched. Cherry-picks retain authors, author dates,
and messages; rewritten commits cannot retain their original signatures. The source
stack tips are unsigned. No formatter or prohibited Git cleanup command was run.

## Application and conflicts

| Ticket | Source branch / tip | Conflicts and resolution |
| --- | --- | --- |
| ov-1154 | `ov-1154-engine-stack` / `df098ece0` | Applied shared preview prerequisites, then its engine stack in original order. Truthful phases conflicted in `workflow_runtime.rs`: retained main's places and pool-bearing running effects while adding activity scopes. Failure-log reads conflicted in `wire_commands.rs`: retained places and added logs. Script decisions and automatic starts conflicted in runtime/service constructors and admission: retained main's separate agent/landing/work pools, scheduler state and scheduling calls; decision effects keep their existing no-slot behavior. Retained the empty ov-1142 checkpoint. |
| ov-1155 | `ov-1155-apps-stack` / `6c0eea202` | Board closes and landing diagnostics overlapped engine fields and command matches in CLI, store, reducer, state and tests. Kept close, decision and refresh commands, all optional fields and both regression suites. Combined gate-output fixture diagnostics. App files otherwise applied without conflict. |
| ov-1164 | `ov-1164-engine-waits-fix` / `1230437df` | Runtime clocks, admission wrapper and publication refresh overlapped wait lifetime changes: retained pool reporting, current workspace refresh and scheduling, added wait clocks, retaining the tokio clock for continuous admission and the elapsed real clock for one-shot wait lifetimes. Reducer retained hook counters and decision timing exclusions with stage patience. Cherry-pick infrastructure detection moved into the stack's extracted `pick_member`, retaining batch diagnosis and safe retries. Gate supervision retained bounded stream capture and incoming pre-spawn retry-safe errors. Both test sets retained. Merge `5fffa41cf` has no delta against its second parent, so its main ancestry was not replayed; its author, date and message are retained as an empty checkpoint. An initial ancestry enumeration also included main's adoption commits, producing duplicate conflicts in `workflow_service/adopt.rs` and `reports/ov-1074/report.md`; that attempt was aborted and those already-landed ancestors excluded before applying the unique wait fixes. |
| ov-1146 | `ov-1146-gate-output-rebased` / `d378f70a9` | Already carried as the eight engine-stack commits `a5824b1a8..6cf9ca72e`; not applied twice. `git cherry` plus range-diff identifies changed patch IDs from the earlier stack's execution-tracking adaptations. New-main log-read overlap resolved under ov-1154 above. |
| ov-1153 | `ov-1153-script-hooks-rebased` / `e70363f0d` | Already carried as six engine-stack commits `d5527c5ed..1e27b3719`; retained the stack's optional group-limit adaptations. New-main pool overlap resolved under ov-1154. |
| ov-1147 | `ov-1147-auto-start-rebased` / `2dd82817c` | Already carried as ten engine-stack commits `37885efc7..d60e1f581`; retained runner-wide persisted admission and scheduling regressions. Kept main's effect pools alongside published ticket admission. |
| ov-1142 | `ov-1142-waiting-groups-rebased` / `724cea500` | Already carried as seven engine-stack commits `1f512b0d1..58cb262ba`, including empty checkpoint and reviewed consolidation fixes. Main's ov-1148 join-conflict/rebase behavior retained alongside diagnosed-member handling. |
| ov-1139 | `ov-1139-board-writes-refuse` / `50213f05d` | Already carried by the app stack as `4508639a5` and `f0ef1f35d`. Combined close fields and command matches with engine hook/refresh state; kept both send-back and close regression cases. |
| ov-1145 | `ov-1145-queue-view-rebased` / `313b88374` | Already carried by the app stack as eight commits `2db79fb54..3f6ca190b` plus reviewed fixture adaptations. Combined diagnostics, execution and hook state; retained both queue and batching tests. Mac/iOS Plan-column changes applied without textual conflicts. |
| ov-1152 | `ov-1152-truthful-phases-rebased` / `8439797e4` | Its complete tree delta is already in preview `3d56dac4c` (the only tree differences from that source tip belong to the preceding ov-1151 preview). Did not duplicate its code changes. Retained the five original commit messages, authors and dates, plus the main-merge checkpoint, as empty equivalence commits because the preview already carries their complete content. Combined activity tracking with places, later shared those observations with the lightweight reader. |
| ov-1159 | `ov-1159-light-workflow-get` / `3703f2847` | Three unique implementation commits applied; investigation source already on main. `wire_commands.rs` conflicted with prepared closes, execution snapshots, failure logs and places. Kept all in the extracted reader. Shared activities and places through cloned Exec so the off-inbox read still reports live facts. |
| ov-1203 | `ov-1203-free-build-output` / `d73dccd58` | Added cleanup fields alongside script/admission limits throughout definitions and fixtures; retained both modules. Kept reader extraction with cleanup status, process-log pruning alongside build release, and shared observation fields alongside active-tree tracking. Combined scheduling Tick and CleanupRetry events and constructors. A previous tree-removal assertion now requires the source tree to remain, while still verifying recovered conflict evidence. |

The ov-1150 preview becomes empty on main and was skipped; ov-1148's preview
is patch-equivalent to main and was skipped. Other shared preview prerequisites
were applied once. `git cherry` was checked against every requested branch;
remaining positive patch IDs in already-consolidated tickets were compared with
the corresponding preserved stack commits, rather than replaying older versions
of those same changes over their reviewed adaptations.

## Validation

- Shared Swift Plan selection: 178 tests in 33 suites passed.
- Web Plan, API and recovery selection: 68 tests in 7 files passed.
- Relay ceremony-reply and web-vault selection: 35 tests in 2 files passed.
- Mac linking libraries rebuilt successfully from this branch with `apps/macos/build-vt.sh`.
- The initial Rust compile identified the combined output supervisor's `io::Error` return needing conversion to `ProcessFailure`. The follow-up ov-1213 commit also adapts the old cap/Running test fixtures to places and preserves the continuous scheduling clock. No feature behavior was added.
- Mac Plan selection (`PlanColumnViewTests|PlanSectionViewTests|PlanViewTests|PlanStaleTests|PlanReconnectingTests`): 37 tests in 5 suites passed.
- Process-guard prerequisite regression: 10 tests passed.
- Shared Swift conversation-cache prerequisite regression: 1 test passed.
- First complete Rust selection: 2,000 passed, 5 failed out of 2,005. Four failures identified combined fixture assumptions: preserved gate exit 1, provisions use the work pool, one-shot pumps drain bounded provision waves, and a hook retry deadline is measured from its observed expiry (including dispatch elapsed time). These are reconciled in ov-1213 test-only commits. The idle-cleanup case hit its initial five-second timeout while host load exceeded 100; the unchanged case then passed twice serially. All five previously failing regressions passed together in a serial nextest run (5/5).
- Workflow (188), CLI (695), store (658), and affected client Plan/presentation (69) cases all passed in the complete Rust selection. Final daemon selection: 447 passed, zero failures, including all 395 workflow-runtime cases plus affected file-watch and summary tests.
- The three ov-1159 status-offload regressions passed separately (3/3).
- iOS PhonePlanColumn: exit 0, 13 collected, 1 optional screenshot-capture skip, 12 actually executed, zero failures (699.821 seconds). This includes the long-title landing queue and largest accessibility text-size cases. All three simulator XCFrameworks were built from this worktree before Xcode built and tested the app on the leased iPhone 17 simulator (SDK 27.1). The job waited for the repository's machine-wide heavy-job lock and retained its device lease and nonzero-execution check.
- Remote main was fetched again after all validation and still resolved to `73974f898`; it remains an ancestor of this branch. No changes to `docs/jobs-to-be-done.md` or
`docs/superpowers/`. `git diff --check origin/main...HEAD` passed; the final worktree is clean after committing this report.

All four acceptance items are met: the twelve tickets are included on current main, the requested test selections pass, the per-ticket conflict table is above, and no ticket was omitted or forced through an unresolved conflict.

## Fix round 1 audit

Read the ticket again through the installed Canary CLI, including its full JSON
history. The history records the return from review to in progress but contains
no reviewer note or requested correction. The workflow evidence read was refused
with `scope-denied: known caller required`.

Fetched `origin/main` again: it remains `73974f898`, an ancestor of this branch.
Rechecked `git cherry` for all twelve recorded source tips. Every remaining
non-equivalent patch has its original subject represented on this branch except
the already-empty ov-1150 prerequisite described above. Main's `places.rs` is
unchanged, and the prohibited documentation paths have no delta.

The fresh Rust selection passed 2,004 of 2,005 tests. The only failure was
`interrupted_ci_later_green_finishes_on_the_pushed_sha`: the resumed pending CI
poll reached Failed instead of Done with only 400 ms of patience, while host
load exceeded 40. The unchanged test passed alone. The combined fixture now
allows 5 seconds for its real Python polls and directly asserts that CI's
creation timestamp follows the one-second push. This preserves the clock
regression check independently of subprocess speed. Production behavior is
unchanged.

The strengthened test passed with the current clock, then failed at the new
timestamp assertion when the finished-effect observation temporarily used the
old frozen sweep time (`created_ms: 5`). The temporary production mutation was
removed before final validation.

Fresh validation:

- Final daemon workflow-runtime selection: 395/395 passed, including the
  repaired fixture, in 168.138 seconds with six test threads. The original
  selection passed all 1,610 other cases: workflow 188, CLI 695, store 658,
  and client Plan/presentation 69. All 2,005 selected cases therefore passed
  across the initial run and affected-module rerun.
- Shared Swift Plan selection: 178 tests in 33 suites passed.
- Mac Plan selection: 37 tests in 5 suites passed.
- The prior iOS log was checked directly: 13 collected, 1 skipped, 12 executed,
  zero failures. No app or production files changed in this round.
- `git diff --check origin/main...HEAD` passed.
- The doc-comment and file-size checks passed.

## Reproduction

The Rust commands clear this dispatched agent's native hook and workflow-run
variables so CLI hook fixtures cannot route to the live agent's socket:

```sh
PATH=/opt/homebrew/bin:/usr/bin:$PATH env \
  -u FARCOOLER_NATIVE_HOOK_SOCKET -u FARCOOLER_NATIVE_SESSION \
  -u FARCOOLER_WORKFLOW_FENCE -u FARCOOLER_WORKFLOW_ITEM \
  -u FARCOOLER_WORKFLOW_MODEL -u FARCOOLER_WORKFLOW_RUN \
  cargo nextest run -p farcooler-workflow -p farcooler-store \
    -p farcooler-cli -p farcooler-daemon -p farcooler-client \
    -E '(not package(farcooler-daemon) and not package(farcooler-client)) or (package(farcooler-daemon) and test(workflow_runtime)) or (package(farcooler-client) and (test(plan_view) or test(workflow_presentation)))' \
    --test-threads 6
```

Final daemon selection, under the same environment cleanup:

```sh
cargo nextest run -p farcooler-daemon --lib \
  -E 'test(workflow_runtime) or test(fs_watch) or test(rpc::status_work) or test(workflow_summary)' \
  --test-threads 2
```

Platform commands:

```sh
swift test --package-path apps/shared/AgentKit --filter Plan
apps/macos/build-vt.sh
caffeinate -d swift test --package-path apps/macos \
  --filter 'PlanColumnViewTests|PlanSectionViewTests|PlanViewTests|PlanStaleTests|PlanReconnectingTests'
./scripts/heavy -- /bin/bash -c './scripts/build-ios-frameworks.sh && ./scripts/ios-ui-tests.sh FarCoolerUITests/PhonePlanColumnUITests'
```
