# ov-1238: linear engine train

Base: `bfe99f5248d5a18c4743ca5c1b94f573e416d6c2` (the `origin/main` read at start).

Applied the 22 queued branches in the ticket’s exact order, then the six waiting branches below. Every applied source commit was cherry-picked with `-x`; no merge commits were picked. Empty and already-applied ancestry was skipped. No waiting ticket was left out. Conflict resolutions preserve the source tickets’ behavior; follow-up source commits retain their own corrections.

No `cargo fmt`, `swift-format`, stash, reset, clean, checkout-index, amend, or `git add -A` was used. All tests run as foreground commands.

## Waiting tickets

| Branch | Source tip | Non-merge commits in source range | Applied here | Skipped ancestry / empty |
| --- | --- | ---: | ---: | ---: |
| `ov-1213-engine-stack-rebased` | `1ad4ce5ce9369da7e1f777b794bcfb0d532a8a7b` | 95 | 82 | 13 |
| `ov-1231-close-as-landed` | `b65637ec409eb5288466bf65a21f901b7a8a99fd` | 5 | 5 | 0 |
| `ov-1235-brief-v2` | `ef0d539ea67f8012d80f88295e1cf4c6e08981c1` | 5 | 4 | 1 |
| `ov-1236-yield-manual-pause` | `be96ac6348e86344e65787a6c7f0455eae9f813e` | 14 | 14 | 0 |
| `ov-1233-hook-boundary` | `f99817b2d13a55eb2772b9cccbe4ec442a384fba` | 19 | 14 | 5 |
| `ov-1178-message-agents-fix` | `0641d37ebc4fe79ba56798c3c92cd2481136dd07` | 6 | 6 | 0 |

Counts are per source branch; shared ancestry appears in multiple source ranges but is applied once. ov-1233’s re-keyed steering ancestors `74ee81497`, `d589f4ac1`, `32c354d7a`, and `39529ec71` have stable patch IDs identical to queued ov-1205 ancestry. `96d00e086` differs from queued `93c644379` only in parent context; its changes are already present. These are skipped rather than replayed over later fixes.

## Conflict ledger

Each row records one source pick that stopped with conflicts. The duplicate steering attempts were skipped in full after comparison; all other resolutions were committed in the source pick that introduced them.

| Waiting ticket | Source commit | Conflicting files | Resolution |
| --- | --- | --- | --- |
| ov-1213 | `74c879376` | `crates/client/src/ffi/route.rs`<br>`crates/daemon/src/rpc.rs`<br>`crates/daemon/src/workflow_runtime/coding_native.rs` | Preserved cancellation resolution routing alongside pause/resume; combined active steering clock/control and freeze cleanup with lost-session detection, startup refusal and failure settlement; kept both native probe modes. |
| ov-1213 | `1d9078787` | `scripts/lane-gate.py` | Use process_guard supervisor for step lifetimes and propagate its return code; retains fail-fast result handling while supervising orphan cleanup on parent death and failure. |
| ov-1213 | `a6f53e2b7` | `apps/android/app/src/main/java/com/farcooler/net/ConnectionTeammate.kt`<br>`apps/ios/FarCooler/ConnectionTeammate.swift` | Retained already-applied ceremony authority implementations using shared Rust phrase wrappers rather than duplicate older inline platform copy. |
| ov-1213 | `847d03fb3` | `crates/daemon/src/workflow_runtime/coding_native.rs` | Kept frozen-process quiescence and drained the native output fanout before receipt hashing. |
| ov-1213 | `ee0c8405b` | `crates/workflow/src/reducer.rs` | Kept original wait/Push deadline reuse while excluding script decision effects from reused stage deadlines. |
| ov-1213 | `814e43b2d` | `crates/daemon/src/workflow_runtime/coding_verbs_tests.rs` | Retained both independent regressions: gate-during-CI/restart and automatic queue/build capacity. |
| ov-1213 | `2a4cebe24` | `crates/client/src/ffi/route.rs`<br>`crates/daemon/src/people_rulings.rs`<br>`crates/daemon/src/rpc.rs` | Unioned WorkflowClose with existing WorkflowPause/WorkflowResume in client routes, Control scope and engine member ruling. |
| ov-1213 | `66201c5a2` | `apps/android/app/src/main/java/com/farcooler/net/ConnectionTeammate.kt`<br>`apps/ios/FarCooler/ConnectionTeammate.swift` | Skipped obsolete inline platform-copy annotations where the queued ceremony branch already delegates this phrase to shared Rust copy. |
| ov-1213 | `7e52f392a` | `reports/ov-1157/landing-rca.md` | Retained the queued ov-1157 RCA, which includes the same analysis plus corrected distinct-main-run concurrency guidance; duplicate documentation pick becomes empty. |
| ov-1213 | `6f375e866` | `crates/daemon/src/workflow_runtime/coding_execute.rs`<br>`crates/workflow/src/reducer.rs`<br>`crates/workflow/tests/groups.rs` | Restored missing gate tree before Spotlight exclusion; retained parallel light-review/check execution with Run I/O SafeFailure retry. Preserved pre-send Push deadline/resumption and fresh AwaitBase/AwaitCi patience; tested all three paths. |
| ov-1213 | `6aedbd804` | `crates/workflow/src/reducer.rs`<br>`crates/workflow/tests/groups.rs` | Applied the ticket’s follow-up correction to preserve stage-entry patience across downtime, retaining pre-send Push alongside base/CI in the regression. |
| ov-1213 | `1d91dd5b7` | `crates/daemon/src/workflow_runtime/coding_execute.rs` | Placed Spotlight exclusion inside restored-tree preparation; preserved preparation SafeFailure retries and concurrent review/check execution using the prepared command. |
| ov-1213 | `cdac26156` | `crates/daemon/src/workflow_runtime/coding_execute.rs` | Retained parallel light review/check branch and the source gate-quiescence retry contract comment; quiescence implementation applied cleanly. |
| ov-1213 | `d8d3c986a` | `crates/daemon/src/workflow_runtime/coding_verbs.rs` | Removed obsolete unpark_build call per output-release ticket while keeping Spotlight exclusion on the new fold tree. |
| ov-1231 | `ddf2d135e` | `crates/store/src/workflow.rs`<br>`crates/workflow/src/reducer.rs` | Added Land alongside Close/Decide/RefreshGroups in reducer and store targeting, preserving scripted decisions and group refresh handling. |
| ov-1231 | `fdd9118cc` | `crates/client/src/ffi/route.rs`<br>`crates/daemon/src/people_rulings.rs`<br>`crates/daemon/src/rpc.rs`<br>`crates/daemon/src/workflow_runtime/wire_commands.rs` | Unioned landed RPC/action targeting with Close, ResolveCancellation and pause/resume methods, retaining all existing scopes and rulings. |
| ov-1231 | `a4a2481cf` | `crates/store/src/workflow_overlay/tests.rs`<br>`crates/workflow/src/reducer.rs`<br>`crates/workflow/src/state.rs` | Added landed field to Item alongside execution, closed, group_failure and script-hook state; initialized it in overlapping Item literals without losing existing fields. |
| ov-1231 | `435116e17` | `crates/daemon/src/workflow_runtime.rs` | Performed both external-close verification and landed remote verification before the final authority recheck; retained Prepared tree/close evidence. |
| ov-1235 | `17b7d67c9` | `crates/daemon/src/workflow_runtime/coding_execute.rs`<br>`crates/daemon/src/workflow_runtime/coding_native.rs` | Retained Codex hook override while making launch extras mutable for file briefs; kept output recovery and failure-bearing settlement verification alongside native last-message extraction. |
| ov-1235 | `3b1e5058d` | `crates/daemon/src/workflow_runtime/coding_native.rs` | Added Cursor last-message availability note while keeping failure-bearing native settlement return type. |
| ov-1236 | `54d6a5577` | `crates/daemon/src/workflow_runtime.rs`<br>`crates/daemon/src/workflow_runtime/coding_execute.rs`<br>`crates/daemon/src/workflow_runtime/coding_tests.rs`<br>`crates/daemon/src/workflow_runtime/fold_trees.rs`<br>`crates/daemon/src/workflow_runtime/service_owner.rs` | Combined builder resource controls with retained gate recovery/parallel light review, native log paths and Codex hooks. Kept output-only cleanup (ov-1203), adding builder-cache cleanup without restoring deleted source-tree pruning; merged test controls into Exec initializers. |
| ov-1236 | `1721eea1e` | `crates/daemon/src/workflow_runtime/coding_execute.rs` | Kept resource lease alive through both parallel consumers, using effect-bound target preparation and checking low disk before mutation. |
| ov-1236 | `f6ddc3e19` | `crates/daemon/src/workflow_runtime.rs`<br>`crates/daemon/src/workflow_runtime/coding_execute.rs`<br>`crates/daemon/src/workflow_runtime/fold_trees.rs`<br>`crates/daemon/src/workflow_runtime/service_owner.rs` | Applied source follow-up removing shared target/cache and leases while retaining nice/low-disk controls, per-run target release, output-only cleanup, gate recovery and concurrent review checks. |
| ov-1236 | `954deab40` | `crates/daemon/src/effect_runner.rs`<br>`crates/daemon/src/workflow_runtime.rs`<br>`crates/daemon/src/workflow_runtime/coding_execute.rs`<br>`crates/daemon/src/workflow_runtime/coding_native.rs`<br>`crates/daemon/src/workflow_runtime/service_owner.rs` | Combined yielding process supervision with retry-safe ProcessFailure, fallible PID identity and execution presence. Paused children do not count as exited; native fanout capture remains. Added Yielding to all Exec constructors and adapted identity comparison to fallible reads. |
| ov-1236 | `be96ac634` | `crates/daemon/src/workflow_runtime/coding_native.rs` | Passed the yielding builder registration into native control for manual-pause ownership while retaining execution-session presence tracking. |
| ov-1233 | `df3c6b682` | `crates/cli/src/workflow.rs`<br>`crates/daemon/src/rpc.rs`<br>`crates/daemon/src/rpc/engine_caller.rs`<br>`crates/daemon/src/workflow_runtime/admission.rs`<br>`crates/daemon/src/workflow_runtime/wire_commands.rs`<br>`crates/store/src/workflow.rs` | Combined audited orchestrator publication/rollback with compact read path and --full CLI, preserving close tests and receipt projection. Exposed publication metadata through Exec read_get; retained owner-only lost-cancellation resolution alongside publication grant fences. |
| ov-1233 | `f0a24e93b` | `crates/store/src/workflow_projection/tests.rs` | Retained external-close projection regression and added informational publication-notice regression. |
| ov-1233 | `605a994c6` | `crates/daemon/src/rpc.rs` | Used extracted publication announcement helper while preserving early compact-summary return. |
| ov-1233 | `74ee81497` | `.config/nextest.toml`<br>`crates/cli/src/message.rs`<br>`crates/client/src/ffi/route.rs`<br>`crates/daemon/src/engine_steering.rs`<br>`crates/daemon/src/people_rulings.rs`<br>`crates/daemon/src/rpc.rs`<br>`crates/daemon/src/watch/answer_wake/messages.rs`<br>`crates/daemon/src/workflow_runtime.rs`<br>`crates/daemon/src/workflow_runtime/coding_native.rs`<br>`crates/daemon/src/workflow_runtime/native_control.rs`<br>`crates/daemon/src/workflow_runtime/native_messages.rs`<br>`docs/engine-steering.md`<br>`reports/ov-1188/verification.md` | Skipped whole pick: stable patch-id 3ed37d62a18238caa271624823006c7678e94192 exactly matches queued f6e3fc548. Replaying older steering would duplicate already integrated behavior. |
| ov-1233 | `96d00e086` | `crates/daemon/src/workflow_runtime/coding_execute.rs`<br>`crates/daemon/src/workflow_runtime/coding_tests.rs`<br>`crates/daemon/src/workflow_runtime/native_messages.rs`<br>`docs/engine-steering.md`<br>`reports/ov-1205/verification.md` | Skipped re-keyed ov-1205 ancestry: compared full patches with queued 93c644379; only parent context around hook publication differs, all added behavior already applied. |
| ov-1178 | `a4166caf4` | `crates/daemon/src/watch/answer_wake/messages.rs` | Prioritized engine_steering registration for native flagged/fenced delivery, retaining the watch_view recipient fallback for managed panes without a steering registration. The expanded messaging tests caught and verified this fallback. |
| ov-1178 | `cb2841920` | `crates/daemon/src/workflow_runtime/coding_native.rs` | Retained native output capture and yielding registration; published watch access for all tickets owned by the effect. |
| ov-1178 | `134c88115` | `crates/daemon/src/workflow_runtime/coding_native.rs` | Retained active-clock/session validation and integrated Claude context reply after steering followup, with exactly one reply per hook; avoids writing an empty reply before a gating direction. |
| ov-1178 | `47fa02c88` | `crates/daemon/src/hook_install/native_host.rs` | Retained Codex engine-hook implementation/tests and added Claude synchronous all-tool context regression in a separate test module. |
| ov-1178 | `9d8e8b623` | `crates/daemon/src/hook_install/native_host.rs`<br>`crates/daemon/src/workflow_runtime/coding_native.rs` | Combined strict active-steering registration validation and Claude synchronous context grammar; retained Claude-specific refusal plus existing steering refusal for other agents. Kept both hook tests. |


## Integration checks

The full required selection compiles, including combined `CodingConfig`, `Action`, `Definition`, `Item` and native settlement literals. Compatibility fixes add absent publication reasons and definition defaults, retain fallible process identity in yielding, and accept the added native failure field in receipt patterns.

The first foreground run exposed these semantic overlaps, resolved without dropping tickets:

| Tickets | Observed conflict | Resolution |
| --- | --- | --- |
| ov-1213 / ov-1236 | The old target sweep ran before the durable cleanup, losing byte counts and bypassing open-file retry. | A retained cleanup intent owns its item’s policy and deletion; the old sweep remains a fallback for items with no durable intent. |
| queued ov-1209 / ov-1213 | A split-half deadline fixture expected its base effect only after the previous CI; pipelining now emits it after the previous push. | Collect the emitted effect across push and CI, retaining the assertion that its deadline starts at the half’s real admission time. |
| ov-1236 / ov-1213 | The capacity fixture waited for a builder to execute while a landing gate deliberately paused it. | The gate now waits for simultaneous admission to separate pools, then checks the admitted builder executes after yielding. Both capacity and pause behavior remain required. |
| ov-1178 / queued ov-1205 | The native delivery fixture had separate watcher/runtime stores; explicit direction also now fences a tool instead of printing informational context. | Share the runner store in the fixture. Explicit direction denies the pending tool; informational runner acceptance notices return context. Both are consumed once through the authenticated hook before Stop. |

CLI hook fixtures inherit neither this task’s private native hook socket nor its workflow bindings on the final run: those session variables override their scratch sockets. `PATH` selects the working system Python for repository scripts. Final test results follow below.

## Queued prefix

Applied in order: `ov-1157-ci-main-verdict`, `ov-1151-mac-perf-rebased`, `ov-1008-uikit-transcript-spike`, `ov-1149-signed-replies-offline`, `ov-1137-fs-watch-flake`, `ov-1162-remote-reconnect`, `ov-1165-ci-cache`, `ov-1172-definition-phone-approval`, `ov-1186-shared-cache-spike`, `ov-1200-vanished-server-flake`, `ov-1205-steer-codex`, `ov-1207-compose-held-flake`, `ov-1211-spotlight-exclusion`, `ov-1216-gate-fail-fast`, `ov-1209-gate-during-ci`, `ov-1184-ci-android-15`, `ov-1219-crate-split`, `ov-1183-ci-swift-15`, `ov-1215-timing-from-receipts`, `ov-1225-light-review-bypass`, `ov-1230-job-time-matrix`, `ov-1223-pipeline-design`.
