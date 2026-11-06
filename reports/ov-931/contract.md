# ov-931 source-preparation contract

Prepared 2026-11-07 on `ov-931-workflow-service`, source HEAD
`5de71fdeb`. This is report-only preparation under task decisions
`7f7a4725` and `a8ff1e3b`, not activation or implementation evidence.
Read parent ov-921 and ov-931 through the supported host CLI. No builds,
product changes, app updates, owner-data operations or prerequisite imports.

## Evidence and corrected assumptions

* `crates/daemon/src/service.rs:1906,2092`: Service retains its root, shared
  Store and process/session owners; `open_in` also serves secondary processes.
  `main.rs:227,334,439,455` opens the ordinary Service, starts the watcher and
  check-in scheduler, constructs the socket factory, and joins the scheduler.
  It currently owns no workflow runtime. Starting an executor in every
  `Service::open_in` would create competing writers from stdio/stream opens.
* `rpc.rs:202,241,264,846`: only `with_workflow_fixture` attaches a handle;
  ordinary factory starts with `None`. The existing authenticated Session,
  scope/caller gates and `workflow_runtime/admission.rs` are the authority
  boundary, including consumption-time recheck after the inbox wait. Do not
  introduce wire grants or select a principal from publication text.
* `workflow_runtime.rs:117,335`: current runtime creates/opens a private Git
  fixture and separate workflow.sqlite; its loop already drains a bounded
  inbox, pumps durable work and sleeps until a lease deadline or command.
  This is an existing wake loop, not a missing mechanism to rebuild.
* `workflow_runtime/wire_commands.rs:47,56` fixture-validates publication
  and enqueue; `protocol/src/workflow_commands.rs:19` and
  `cli/src/workflow.rs:28` publish only a Definition. CLI already bounds reads
  and refuses nonregular/symlink inputs. Existing file bounds must extend to
  adapter files and receipts, rather than assuming a trusted file is bounded.
* `store/src/workflow_publication.rs:9,33` has immutable definition records,
  mutable workspace selection and atomic publication/receipt transactions.
  `workflow.rs:46` atomically stores state/events/effects. Reuse this model;
  a workspace's current definition pin alone does not pin adapter config.
* Read-only component inspection: ov-928 branches both resolve to
  `b15f90bb44c7641b48c524dd94a4233a94093f52`; ov-929 resolves to
  `c6d27220984cc25e1ff899cb96edb1d6e7eb65cc`. These are observations,
  **not permission to compose b15**. In b15, `coding.rs:145` requires generated
  stage IDs/order, and `workflow_runtime.rs:146` loads one `coding.json` at
  startup. Neither meets flexible definitions and immutable per-item config.
  The helper at `coding.rs:125` remains an optional archetype only.
* `daemon/tests/workflow_commands_over_socket.rs:15` explicitly constructs a
  separate fixture runtime and injects it into a factory. It proves transport
  and receipt behavior, not ordinary Service/bootstrap ownership. Its Drop
  aborts tasks; that cannot count as graceful shutdown/join evidence.
* `service.rs:3597,3631`, `agent_panes.rs:44,117` and `task_starts.rs` are
  concrete legacy task-dispatch/worker entry points. Check-in/answer delivery
  is not itself a workflow execution scheduler. Item/task mapping and mutation
  guards must be explicit before claiming these paths cannot compete.

## Bounded implementation contract after the reviewed composition arrives

1. Retain one workflow supervisor in Service, with its authenticated command
   handle and owned shutdown/join state. Start it only from the daemon's
   existing exclusive bootstrap, before serving workflow commands; recover
   durable work before accepting competing effects. Ordinary RpcFactory uses
   this retained handle. Secondary Service opens do not start a runtime or
   seize a writer lock; existing stdio forwarding must reach the daemon owner
   rather than quietly constructing another executor. Use an extracted common
   bootstrap seam for isolated tests, not a replacement fixture factory.
   Startup failure must not advertise a usable execution path. Stop admissions,
   signal the owner, terminate/reap owned processes within deadlines and join
   before releasing execution ownership; unprovable external outcome remains
   uncertain. Do not turn cancellation/shutdown into a successful receipt.
2. Extend the existing set-workflow file to publish a versioned bundle containing
   the pure definition and exact existing trusted-local adapter configuration.
   Keep shared wire DTOs free of daemon dependencies. Pure definition pin stays
   the definition identity; bundle/config pin includes schema, adapter versions
   and canonical serialized config. Atomically persist immutable bundle bytes,
   workspace selection and command receipt in the existing Store transaction.
   Never persist a filesystem pathname as the sole configuration version.
   Enqueue selects an inspected bundle pin and records its binding with the
   item. Effect dispatch/reconciliation/rework resolves that binding, never
   the workspace's newer selection. Unknown/missing/corrupt pins refuse closed.
   Validate before *fresh* admission, retaining cached-receipt lookup before
   mutable current-state validation. Replayed committed commands remain stable.
3. Validate pure definitions and finite supported adapters separately. Match
   operation versions, kind, arguments, check index/reviewer identity and config
   pin; validate actual repository/base, harness/settings/hooks, command/check,
   landing ref/config and CI repository/workflow/event identities. Use bounded
   nonblocking regular-file reads and identity checks from corrected ov-928.
   Perform mutable resource checks again immediately before an effect. Accept
   renamed stage IDs, multiple checks, any/every-stage approval and valid
   configured rework destinations. Require only semantic safety prerequisites
   for the requested operations (e.g. candidate/evidence for landing), not
   equality with the generated pipeline, mandatory review, or Build-only rework.
   Fast-ship definitions remain possible within adapter safety constraints.
4. Before any effect, record durable engine ownership of the exact workspace
   item and its board task mapping, if any. Enqueue must reject an active legacy
   worker for that mapping; legacy dispatch/worker-start paths must reject an
   engine-owned task. Both checks and admission need one serialized durable
   boundary, so concurrent legacy/engine admission cannot both win. Receipt
   replay preserves ownership. Do not invent a per-workspace dual-engine switch
   or migrate existing owner tasks in this slice. Arbitrary workflow item strings
   must not be treated as task UUIDs without validation. Keep no-Git tracker
   execution available without requiring a coding configuration or repository.
5. Routine work advances on durable events/deadlines with no orchestrator or
   informational ACK requirement. Approval gates are intentional dependencies;
   they resume only on the existing valid principal/run/pin/candidate decision.
   Preserve uncertain-effect reconciliation and single-writer resource fencing
   across restart and config changes.

## Planned source files and regressions

| Area | Planned files | Required evidence |
| --- | --- | --- |
| Owner lifecycle | `crates/daemon/src/service.rs`, `main.rs`, new small Service workflow supervisor module, `rpc.rs` (or reviewed ov-929 extraction) | Ordinary factory/socket starts enabled, one owner across secondary opens, startup refusal, wake/deadline recovery, joined shutdown and restart |
| Publication vocabulary | `crates/protocol/src/workflow_commands.rs`, `crates/cli/src/workflow.rs` | Real CLI file parses bundle, unknown/oversize/alias/FIFO inputs refuse, no caller authority/result fields |
| Immutable storage | `crates/store/src/workflow_publication.rs`, migration registry and new appended migration, narrow ownership storage module if needed | Atomic rollback of config/selection/receipt, immutable bytes, A item/effect remains A after B publication/restart/rework, same ID different payload conflict |
| Dispatch resolution | `crates/daemon/src/workflow_runtime.rs`, `workflow_runtime/{wire_commands,coding,coding_execute,coding_native,coding_landing}.rs` from reviewed union | Resolve item-bound config in execution and reconciliation; altered identities refuse before effects; no current-global fallback |
| Legacy boundary | `crates/daemon/src/service.rs`, `agent_panes.rs`, `task_starts.rs` and exact mutation seam identified during implementation | Concurrent legacy/engine claims have one winner; restart retains claim; unrelated task behavior retained |
| Ordinary proof | new `crates/daemon/tests/workflow_service_over_socket.rs` plus bootstrap unit coverage and CLI producer regression | Actual authenticated Session through ordinary factory publishes/enqueues/reads/approves; coding check/review/fail/rework/land/CI completes offline with zero ACK; auto and configured gate modes; no-Git retained |

Implementation must inspect the final reviewed tree before editing: names/line
numbers above refer to this prep snapshot or explicitly named b15 read-only
inspection, and are not a proposed patch against an unreviewed component.

## Composition and verification boundary

Wait for orchestrator's exact corrected ov-928 security disposition from the
same reviewer and exact composition pin including final ov-930/929 and approved
core/runtime/transport. Preserve frozen signed originals. Record supplied full
commit IDs, signatures, ordered composition, tree/blob hashes, overlapping-file
resolutions and exact reviewed-byte equivalence in a signed final union manifest.
Current prep records no such approved union and leaves all acceptance unchecked.

After authorization, use the existing normal heavy FIFO with one isolated writer,
jobs=6, profile 000, offline dependencies, load5 <20, 10 GiB reserve, owned
deadlines/reap and provenance-bound warm target. Do not create a cold duplicate
target or clean existing targets. Target protocol/CLI/store/runtime/socket/bootstrap
regressions and repository-required lint/size checks. Include meaningful negative
mutations (ordinary handle absent, current-config fallback, ownership guard absent,
receipt read blocking/identity expansion) and restored runs on the same source;
then independent security review of the exact signed composition. No tests/builds
were run during preparation. Main CI/platform gates, owner-data rehearsal and
whole-app cutover remain ov-921 obligations. Commit normally; no fmt, stash,
reset, clean, amend, add-A, installed activation or provider/push actions.

The remaining blocker is prerequisite source review/composition from the
orchestrator, not an unanswered owner product choice. Keep ov-931 in progress
during preparation; do not claim task completion or move to review on this report.
