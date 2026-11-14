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

## Initial validation

Results below were checks on the initial integrated branch, before fix round 1. Builds use only
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

## Initial acceptance status (before fix round 1)

The first build integrated both stacks and documented all conflicts, but did
not complete acceptance 2: shared-core lint failed on four preview violations,
and the iOS UI command was cancelled while queued. The review returned the
build for those two items. The completed resolution and fresh results follow.

## Fix round 1: acceptance 2

The review finding on the ticket requires an executed, passing iOS Plan/smoke
run and a zero exit code from shared-core lint. The reviewed stacks and their
conflict resolutions above are retained.

### Shared-core lint resolution

Reproduced all four reported ratchet failures. Each new string describes
platform-owned enrollment or browser recovery state:

- iOS and Android `ConnectionTeammate`: the device authentication prompt for
  confirming browser enrollment, not a projection of runner workflow data.
- Web `app.ts`: the browser's passkey recovery-copy deletion and sign-out UI.
- Web `saved-copies.ts`: management of account-owned browser recovery ciphertext
  and passkey credential IDs at the relay, independent of a connected runner.

Used the lint's documented `platform copy: <reason>` annotation on those lines
and recorded just these four files in the baseline's `exempt` map. No ratchet,
Rust-copy, wire-word, or twin allowance was increased. The existing strings,
control flow, wire types and rendered screens are unchanged. This resolves the
preview integration's lint failures within the requested fix; it adds no feature.
The lint's baseline-growth gate explicitly permits reviewed platform exemptions.

Fresh checks in this round:

| Gate | Exit | Result |
| --- | --- | --- |
| `scripts/shared-core-lint.py` | 0 | All four failures resolved |
| Shared-core baseline not grown vs `wave-a-preview` | 0 | Feature-copy ratchet unchanged |
| Shared-core lint self-test | 0 | Violation detection still passes |
| Copy, file-size, visual-token, swallow lints | 0 | Passed |
| Doc-comment check against `wave-a-preview` | 0 | Passed |
| Web recovery and Plan unit tests | 0 | 49 tests across 5 files passed |
| Web client TypeScript | 0 | Passed |
| Mac selected Plan suites | 0 | 37 tests across 5 suites passed |
| Affected Rust selection | 0 | All 1,059 selected tests passed |
| iOS PhonePlanColumn plus smoke UI tests | 0 | 20 actually ran and passed; 1 optional capture skipped |

The web selection was `saved-copies.test.ts`, `sign-out.test.ts`,
`vault.test.ts`, `plan.test.ts`, and `plan-column.test.ts`. Mac used the same
five Plan suites as the initial validation. Initial sandboxed web test execution
could not start its Workers runtime; the permitted rerun above completed.
Raw logs for this round are in `target/ov-1155-validation/*-fix-round1.log`.

The fresh Rust run used the same six packages and filter as the initial final
run above, through `scripts/heavy --light`, with two compiler jobs and the
native hook/session environment removed. It passed all 1,059 selected tests in
123.478 seconds after compilation (4,625 cases outside the selection).

### Executed iOS Plan and smoke tests

The shared gate became available; the queued command ran to completion against
this worktree's `target/ios-derived` app and UI test bundle. The existing gate,
device lease, boot budget, test timeouts and nonzero-executed-test check were
retained. No other worktree's process or simulator was stopped.

```sh
export PATH="/opt/homebrew/opt/python@3.14/libexec/bin:/opt/homebrew/bin:$PATH"
XCTESTRUN="$PWD/target/ios-derived/Build/Products/FarCooler_iphonesimulator27.1-arm64.xctestrun" \
  ./scripts/ios-ui-tests.sh FarCoolerUITests/PhonePlanColumnUITests \
  $(./scripts/ios-ui-shards.py smoke)
```

Exit code **0**. The leased device was iPhone 17,
`47C77A22-0EDF-4349-B2A1-F678935955AB`. On November 14 the run completed at
12:46:45 local time in 792.393 seconds: 21 collected, 1 skipped, **20 actually
ran**, zero failures. `PhonePlanColumnUITests` passed its 12 executable cases,
including the long-title landing queue and largest accessibility type size;
its `testCaptures` case requires explicit `FC_CAPTURES=1` and was skipped as
designed. All eight smoke cases executed and passed. The wrapper printed
`OK: 20 test(s) executed.` after `TEST EXECUTE SUCCEEDED`.

See `ios-plan-smoke.txt` for the retained per-case verdicts. The full log is
`target/ov-1155-validation/ios-fix-round1.log`.

### Final acceptance

1. **Met:** both reviewed stacks remain integrated on top of `wave-a-preview`,
   preserving the conflict resolutions documented above.
2. **Met:** affected Rust (1,059), Mac Plan (37), web (49), TypeScript, shared-core
   and copy lint pass by exit code. The initial shared Swift Plan check (178)
   remains valid; this round changes only comments and the lint exemption map.
   The iOS Plan and smoke UI selection executed and passed with exit code 0.
3. **Met:** every integration conflict and its resolution is listed above,
   together with the semantic fixture fixes and this round's lint resolution.
