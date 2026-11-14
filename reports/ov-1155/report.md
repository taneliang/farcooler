# ov-1155: Reviewed app stacks on wave-a-preview

The assigned branch was clean and an ancestor of `wave-a-preview`, so it was
fast-forwarded to `b954ea25f` without replacing the assigned branch. Applied
ov-1139's two commits, then ov-1145's eight commits, in their original order.
No features were added beyond those reviewed stacks.

## Conflict resolutions

### ov-1139, source commit 0ef9ef388

- `crates/cli/src/workflow.rs`: kept both ResolveCancellation and Close commands,
  their help, input validation, and action dispatch.
- `crates/client/src/ffi/route.rs`: retained the unsupported-FFI route entries for
  both WorkflowResolveCancellation and WorkflowClose.
- `crates/daemon/src/rpc.rs`: kept both methods in Control scope and RPC dispatch.
- `crates/protocol/src/workflow_commands.rs`: retained both actions and wire names.
- `docs/workspaces.md`: retained the preview's fold-conflict/review explanation
  and the external-completion documentation.

The second ov-1139 commit applied cleanly.

### ov-1145, source commit 54bf6c946

- `crates/client/src/workflow_presentation/tests.rs`: kept both stored-uncertainty
  explanation coverage and split-group diagnostic presentation coverage.
- `crates/store/src/workflow_overlay/tests.rs`: initialized both `closed` and
  `group_failure`, preserving the preview's `execution` field.
- `crates/workflow/src/reducer.rs`: initialized both close and diagnostic state.
  Kept the preview's split rule (splits do not spend back-edge allowance) and
  marked retained group diagnostics as split before spawning the other half.
- `crates/workflow/src/state.rs`: combined the optional CI failure and detailed
  rejection outcome with the preview's FoldConflict outcome.
- `crates/workflow/tests/groups.rs`: retained both repeated fold-conflict/replay
  tests and split-diagnostic/replay tests.
- `services/web/src/client/app.css`: retained saved-copy recovery styles and the
  landing-group wrapping rule.
- `services/web/src/client/plan-view.ts`: the empty guard requires both an empty
  roster and no landing groups, preserving roster-only and queue-only plans.

### ov-1145, source commit 305d20dd8

- `apps/ios/FarCooler/PhonePlanColumn.swift`: render shared landing groups while
  keeping `hasColumnAgents` in the empty-state guard.
- `apps/ios/FarCoolerUITests/PhonePlanColumnUITests.swift`: retained the long-title
  queue regression and the preview's large-type parameter and longer drag.
- `apps/macos/Sources/FarCooler/Plan/PlanColumnView.swift`: retained the full shared
  view for group rows and the roster-aware empty-state guard.

### ov-1145, source commit a572da1c4

- `services/web/test/browser/plan-render-check.mjs`: kept the preview's golden and
  focused pipeline checks at 360/1440 px, then ran the queue-only check on its
  own page at 320 px, with the ordinary plan hidden. Browser cleanup remains
  in `finally`.
- `services/web/test/browser/plan-render-harness.ts`: kept focused/original
  fixture selection and queue fixture rendering; the queue-only fixture clears
  the roster as well as the pipeline so it exercises the intended empty case.

### Semantic conflicts found by verification

- `crates/client/src/session/workflow/tests/socket.rs`: the imported real-group
  socket fixture used `GroupDef { limit: 8 }`, while the preview's definition
  now uses `Option<u32>`. Changed it to `Some(8)` to retain the original bounded
  group. The first test build failed with E0308 at this exact initializer.

- `test/fixtures/plan-landing-queue.json`: the imported actual-CLI fixture
  predated the preview's execution observations. A pending Run without a live
  claim is now `WaitingSlot`, not `Running`. Regenerated it with the existing
  `UPDATE_PLAN_LANDING_FIXTURE=1` integration-test updater, then reran the
  regression without that flag. Both exited 0. CLI/Session equality and the
  four landing-group rows already passed before the fixture refresh; the
  refresh changes only two run pages' execution-state text and phase.

All remaining source commits applied cleanly. Clean merges retain the preview's
execution observations and recovery commands, the external-close path, queue
presentation, CLI/session enrichment, and CI rerun support.

## Validation

Results below are fresh checks on the integrated branch. Builds use only
this worktree's `target/`; no build artifacts are redirected to `/tmp`.

Completed lightweight checks (November 14):

| Gate | Exit | Result |
| --- | --- | --- |
| Web Plan unit tests, three affected files | 0 | 22 passed |
| Web client TypeScript | 0 | No type errors |
| Production web Plan renderer | 0 | Pipeline at 360/1440 px; queue at 320 px |
| `scripts/copy-lint.py` | 0 | Passed |
| `scripts/file-size-budget.py` | 0 | Passed |
| `scripts/visual-tokens-lint.py` | 0 | Passed |
| `scripts/swallow-lint.py` | 0 | Passed |
| Doc-comment check against wave-a-preview | 0 | Passed |
| `git diff --check` | 0 | Passed |
| `scripts/shared-core-lint.py` | 1 | Four failures already present on wave-a-preview |

The in-memory baseline comparison uses the same lint measurement and exclusions
on the tracked preview files. Its four failures exactly match this branch's;
there are zero new violations relative to the preview. See
`shared-core-baseline-comparison.txt`. All four offending files are unchanged
by this integration. Changing that unrelated copy or increasing the lint baseline
would exceed this ticket's conflict-resolution-only scope.

Native preparation and Swift results:

| Gate | Exit | Result |
| --- | --- | --- |
| Local CLI and daemon build, two compiler jobs | 0 | Passed |
| Fresh Mac Rust libraries via `apps/macos/build-vt.sh debug` | 0 | Staged from this worktree's target/debug |
| AgentKit `swift test --filter Plan` | 0 | 178 tests in 33 suites passed |
| Mac selected Plan suites | 0 | 37 tests in 5 suites passed |
| `scripts/build-ios-frameworks.sh` | 0 | Fresh simulator XCFrameworks built |
| Actual CLI fixture regression after regeneration | 0 | Passed without update mode |

The Mac selection was `PlanColumnViewTests|PlanSectionViewTests|PlanViewTests|PlanStaleTests|PlanReconnectingTests`.
Swift scratch directories and all Rust products are under this worktree's
`target/`. Native hook socket/session environment variables were removed from
test children so they cannot redirect fixture traffic to the parent agent.

The first completed Rust selection ran 1,059 tests: 1,058 passed and the one
outdated actual-CLI fixture failed. The fixture was regenerated and its isolated
regression passed, as described above. The clean final selection passed all
1,059 tests with exit code 0, using the same filters after that correction.

The final Rust command selected `farcooler-client`, `farcooler-cli`,
`farcooler-daemon`, `farcooler-protocol`, `farcooler-workflow`, and
`farcooler-store`, with nextest filter:

```text
test(plan) | test(workflow) | test(route) | test(method) | binary(groups) | binary(send_back)
```

Its final run passed 1,059 tests in 297.242 seconds, exit 0. This includes the
real CLI-to-Session-to-Mac fixture regression, workflow close/recovery cases,
execution-state projection, repeated fold conflicts, group diagnostics and replay.
The remaining 4,625 discovered cases were outside this affected-test selection.

The generated iOS project and UI test bundle also passed `build-for-testing`
(exit 0), using the same generic arm64 simulator configuration as CI and
`-derivedDataPath target/ios-derived`, with two build jobs. The initial build
attempt found no generated `project.pbxproj`; running
`apps/ios/generate-project.py` resolved that fresh-checkout setup issue.
The resulting `.xctestrun` is ready for a Plan/smoke run against these freshly
built app and test products.

## Acceptance and remaining blockers

1. Both reviewed stacks are on `wave-a-preview`, in order. Conflict resolutions
   preserve cancellation recovery, external close, fold-conflict recovery,
   split diagnostics, roster layouts and landing groups. The clean Rust,
   shared Swift, Mac and web results above cover those interactions.
2. **Not fully met.** Shared-core lint exits 1 on the same four unchanged
   preview-base violations. The iOS app and UI tests compile successfully,
   but the Plan/smoke UI run did not execute: its machine-wide gate was queued
   for more than an hour on November 14, behind another worktree's gate.
   That gate spent a substantial part of the wait in the Rust shadow harness's
   load guard, then advanced to its workspace tests while retaining the lock.
   Only this ticket's still-waiting wrapper was cancelled (exit 143), after
   checking that it had no test child. No other process or simulator was stopped.
   No iOS UI success is claimed. Changing unrelated feature copy is outside
   the conflict-resolution-only scope. The shared simulator gate was kept in place.
3. Every textual and semantic conflict and its resolution is listed above.

When the shared gate is available, the remaining UI command is:

```sh
export PATH="/opt/homebrew/opt/python@3.14/libexec/bin:/opt/homebrew/bin:$PATH"
XCTESTRUN="$PWD/target/ios-derived/Build/Products/FarCooler_iphonesimulator27.1-arm64.xctestrun" \
  ./scripts/ios-ui-tests.sh FarCoolerUITests/PhonePlanColumnUITests \
  $(./scripts/ios-ui-shards.py smoke)
```

The script retains its device lease, boot-budget and nonzero-executed-test
checks. Raw build/test logs are retained locally in `target/ov-1155-validation/`.
