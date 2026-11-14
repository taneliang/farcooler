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

### Semantic conflict found by the Rust test build

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

Results will be recorded here after the affected gates finish. Builds use only
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
