> Current allocation: implementation released by task decision `3bd232aa` at
> `77250986dcb4b1dbb6b63e06d0b11dc1b92d88a3`. The source-prep sections below are
> preserved historical findings; their shared-file hold is superseded by that
> release. Current files/APIs/bounds are in the implementation allocation at the
> end, and implementation.md describes delivered behavior and limits.

# ov-933: source-preparation contract

Inspected source: `5de71fdeb3c9bdd86f8119b865b4aadc59e45b30`, branch
`ov-933-workflow-notices`. Report only; no product changes, builds, test execution,
native injection, owner data access or acceptance completion is claimed.
Root confirmed this exact inspection base in task decision `a9510757`;
`9305de` was shorthand, not a Git ref. Coordination is through root, not sibling
agent CLI messaging. All baseline file:line references below refer to this pin. Proposed APIs are not
existing receiver guarantees. ov-931 owns Service/config/protocol/Store/migrations;
its current implementation is deliberately not composed or edited here.

### Read-only ov-931 coordination observation

Per root's supplied path, inspected the accepted prep report and early DTO in
`/Users/e-liang/Library/Application Support/com.farcooler.FarCoolerCanary/worktrees/overnight/ov-931-workflow-service`.
Observed HEAD `c40197486572a584f8c85ec2dde423587e0560e6`; that tree remains
unreviewed and may have active edits. This is neither an approved composition
pin nor permission to import its data code. Exact observed file hashes are in
the separate `coordination-sha256.json` manifest.

* Its `reports/ov-931/contract.md:53` assigns the exclusive bootstrap one retained
  runtime and preserves secondary Service opens. `:88` requires an explicit
  native-task binding if any, consistent with not inventing a task for notices.
* `crates/protocol/src/workflow_config.rs:66` exposes closed
  `AdapterConfiguration::Coding(CodingConfig)` with `coding/v1` identity. A
  notification policy describes workflow key points, not a new coding adapter;
  do not add a fake adapter variant or mutate CodingConfig to carry delivery
  authority. Freeze its optional bundle-level placement with ov-931.
* `crates/protocol/src/workflow_commands.rs:18` uses optional `adapter` with
  omission on serialization. `:38` rejects unknown/noncanonical round-tripped
  fields. A future optional notification field must preserve omitted legacy
  command bytes and committed receipt replay; adding a serialized default/null
  would change identity. Test original publication replay after the extension.
* `crates/store/src/workflow_publication.rs:51` selects an immutable bundle and
  commits publication plus receipt together. `:88` defines immutable bundle
  storage. Notification-policy selection must integrate this frozen bundle
  identity or a separately pinned immutable policy chosen in the same enqueue
  transaction, never a second mutable current-policy selector. No schema number
  or migration slot is claimed here. Existing immutable bundle bytes/pins must
  remain readable; append a versioned schema instead of rewriting them.

Requested boundary through root: exact frozen bundle policy placement/omission
semantics, append-only migration slot and transaction hook, ordinary supervisor
wake/admission hook, and native terminal/session admission file allocation.
Source preparation is complete enough for this coordination; no extra broad
planning review or owner product question is implied.

## Existing source and counterexamples

| Concern | Actual seam / limit |
| --- | --- |
| Producer | `crates/workflow/src/reducer.rs:155` produces `item:Phase` for every changed item, including changes within the same phase. `state.rs:108` stores `Vec<String>`, not structured key points. Parsing that prose would lose identity and is excluded. |
| Atomic commit | `crates/store/src/workflow.rs:46` and `:205` commit admitted input, event, state, effects and command receipt together. This transaction is the notification-selection seam; post-commit reconstruction from current item state would incorrectly attribute old candidate/run/policy. |
| Existing notice ID | `crates/store/src/workflow.rs:76` hashes `(notice/v1,workspace,revision,index)`; it does not persist delivery state or preserve structured event/item/run identity in its result. Equal text across revisions already has different IDs; do not replace this with text deduplication. |
| History endpoint | `crates/daemon/src/workflow_runtime.rs:175` only delegates to Store. `workflow_runtime/wire_commands.rs:18` includes all notices after revision zero in Get. This is inspection, not native delivery or a bounded producer. |
| Actual ordinary producer gap | `crates/daemon/src/rpc.rs:201` labels the embedding fixture-only; `:857` requires an injected handle. `main.rs:439` creates ordinary RpcFactory without injection. Runtime's owned loop already exists at `workflow_runtime.rs:335`; ov-931 is composing it into ordinary bootstrap. ov-933 must attach to that owner, not fork a second engine loop. |
| Caller authority | `workflow_runtime/admission.rs:27` captures server Session, peer/access generation, kernel PID/incarnation, workspace, seat, terminal/login/lead. `:88` rechecks at consumption after all awaits and before synchronous Store work. `:122` rejects terminal-row/resource-version, login or lead changes. No caller-supplied actor/Grant can replace this proof. |
| Orchestrator lookup | `service.rs:4808` explicitly calls `live_orchestrator` a display view that can be stale. `:4841` obtains fresh seat facts using rows then inventory refresh. Both require final row/version/epoch/session/liveness validation at admission; neither snapshot grants permission across awaits. Seat changes already serialize through `orchestrator_locks` (`:1951`, `:3511`). |
| Native message admission | `crates/store/src/messages.rs:54` atomically inserts task note + message wake, returns a new note UUID, and honors wake_on_answer. It requires a native task UUID; workflow item IDs are strings (`workflow/src/state.rs:36`). There is no established universal item-to-task mapping. Calling this after workflow commit creates an unbound second transaction and lost-reply duplicate risk. |
| Existing recipient routing | `watch/answer_wake/messages.rs:362` resolves a `None` recipient to the current orchestrator, or waits offline. It does not itself persist a terminal epoch binding. `:131` explicitly warns sender attribution is not a security boundary. |
| Legacy coalescing | `store/src/messages.rs:90` replaces/coalesces unclaimed runner notices by task. This loses distinct engine events; `send_runner_notice` is not the engine notice producer. |
| Native mailbox receipt | `crates/agent-hooks/src/mailbox.rs:109` writes/syncs a temp file then renames into new/. `:122` exposes Absent/Waiting/Taken; `:157` renames new/ to taken/ before reading bytes. Taken proves local handoff only, not provider consumption or a human ACK. No directory fsync or durable receiver deduplication ledger exists here. Re-dropping a name already in taken/ can recreate new/ and deliver twice. |
| Existing wake mailbox route | `watch/answer_wake/deliver.rs:151` remembers pane/session in a route file, observes Taken and retracts Waiting before fallback. `:228` best-effort route persistence ignores write errors. It is not sufficient as a durable engine delivery receipt. |
| Provider inbox limit | `crates/cli/src/hook_inbox.rs:12` states no socket receipt exists. `:112` emits auth and user JSON lines; `:125` confirms by wrapped-text substring in transcript JSON; `:162` collapses send/timeout failures into Failed and polls. This cannot justify identity-based engine receipt or blind fallback after possible acceptance. No new provider integration or transcript polling is proposed. |
| Engine progress | `workflow_runtime.rs:216` pumps effects without notice state. `:335` waits on inbox/shutdown/actual lease deadlines when idle. Preserve this behavior; informational delivery must not become eligibility, source sequence, lease, or workflow approval. |
| Intentional approval | `workflow_runtime/wire_commands.rs:64` submits authenticated principal and exact observed candidate/run/pin; reducer `:58` requires WaitingApproval and exact tuple. An informational receipt must never invoke Approve/Review or construct a Decision grant. |

## Finite proposed data/API contract for ov-931 coordination

This is the requested concrete implementation boundary, pending root allocation
and ov-931 frozen schema. Method names are proposed internal APIs, not wire API.

1. Inside the existing accepted workflow transaction, select structured key
   points from old/new state plus admitted input. Persist a notification row with
   schema, notice ID, event ID, workspace, workflow epoch, committed revision,
   item, run, stage index, iteration, attempt, candidate (explicit null allowed),
   definition pin, execution policy pin, notification-policy pin and kind.
   Text is presentation only. Use a versioned digest of event/workspace/revision/
   item/kind/ordinal for identity and retain the exact payload hash for collision
   rejection. Repeated same event cannot create another row; equal text from a
   new event must survive. Retain the selection policy used at commit.
2. Proposed optional versioned notification policy belongs in ov-931's immutable
   configuration bundle, with backward-compatible omission semantics agreed by
   that writer. Finite key-point set: entry into WaitingApproval, WaitingReview,
   Blocked, Uncertain, Done, Failed, Cancelled; stage/run/iteration advancement
   as a separate selectable `stage_changed` point. No arbitrary prose filters,
   custom scripts, mandatory stage names/order or new workflow gate. Emit phase
   entries on actual phase entry; stage_changed on changed structured stage/run/
   iteration, not every item field update. Multiple selected kinds receive
   distinct identities. Proposed default: approval/review/uncertain/terminal
   phase entries; exact default and field shape must freeze with ov-931.
3. Proposed Store APIs: `pending_notices(workspace, after, limit)`;
   `claim_notice(id, payload_hash, expected_delivery_version, target_binding)`;
   `record_notice_outcome(attempt_id, expected_version, evidence)`;
   `notice_delivery(id)`. All mutating calls compare exact identity/version and
   are internal daemon operations, not caller grants. Pending enumeration is
   indexed, bounded (proposed 32 per pass), ordered by revision + ordinal + ID.
   Atomic outbox insertion plus indexed pending state is sufficient; do not add
   a separate producer watermark. Delivery rows remain individually addressable;
   enumeration never implies acceptance. No full-history rescan on Get/idle.
4. Append-only schema owned by ov-931: immutable notification payload rows;
   per-notice delivery state/version; immutable attempt identity and exact target
   binding/evidence. Do not mutate the meaning of existing event/effect/
   command receipts. A dedicated workflow-notice outbox is deliberately finite,
   not a framework for unrelated native notices. No migration/backfill sends
   old live backlog; rollout selection starts at an explicit committed boundary.
5. Actual normal producer will be the sole Service-owned runtime accepted
   transition path supplied by ov-931. Commit makes rows durable; a wake merely
   requests bounded draining. Startup recovery, new committed selected rows,
   authoritative orchestrator/session changes and native receipt events wake
   delivery. Use existing lifecycle ownership and cancel/join semantics. No
   idle polling timer and no informational receipt wait in the effect pump.
6. Proposed internal native adapter: `admit_notice(notice, target_proof)` and
   `observe_notice(attempt_id, target_binding)`, returning NotAdmitted,
   Admitted(native identity), HandedOff(native evidence), or Unknown(reason).
   Target proof is daemon-created, non-serializable authority, never a caller
   terminal UUID. Bind workspace, terminal UUID/resource version/epoch/lease,
   session ID, current lead/login and current native endpoint identity. Recheck
   after every await immediately before native admission, serializing against
   seat/session replacement through the existing owning seam. A stale proof
   must fail before bytes. Merely resolving `live_orchestrator` is insufficient.
7. A native task-backed wake may only be used if ov-931 provides an authenticated
   engine-item/native-task binding and atomic deterministic native admission.
   Do not synthesize a task or treat arbitrary item text as UUID. Otherwise use
   the existing terminal/session inbox seam directly through a bounded workflow
   adapter, retaining native route ownership. If a harness has no safe native
   admission/identity receipt, leave pending/unsupported visibly rather than
   adding a provider SDK, screen parsing or generic fallback.

## Delivery/recovery state machine and guarantee

### Accepted first native path (root decision `0d36fcbb`)

Keep the default decision/uncertain/terminal key points and optional stage-change
selection. First support the owner's existing Codex/native route; unsupported
endpoints stay explicit. No extra producer watermark or message framework.

Baseline `watch/answer_wake/deliver.rs:315` proves the agent, finds the current
rollout/thread and native Codex program, then invokes
`codex queue --thread <thread> --message=<text>` with the discovered home. This
is an existing native CLI seam, not a new SDK. It currently returns only
`Option<Queued>`: `:334` combines timeout, spawn/status errors and nonzero exit
into None. The caller at `:296` then falls back to typing. A workflow adapter
must retain the preadmission/possibly-sent distinction and cannot inherit this
fallback. Success proves local native command handoff, not provider consumption.
The queue interface supplies no stable receiver deduplication receipt.

`watch/answer_wake/queued.rs:108` starts a detached rollout/text watcher with a
two-second interval (`:46`, `:119`). Do not reuse that polling watcher as the
workflow notice receipt or claim that its text match identifies an event. The
new finite adapter retains exact structured notice/attempt/target identity,
uses owned bounded native execution and final current authority checks, records
honest handoff/Unknown, and never blindly retries a possibly sent invocation.
Likewise the scheduled check-in Enter path in `compose/codex.rs:337` has a
durable admission fence, but its screen checks at `:309` and `:326` are outside
this task's permitted structured delivery design. No screen-based check-in flow
or legacy notice scheduler is imported. These are source-backed limitations of
the first concrete native path, not a request for broader redesign.

`pending -> prepared -> admitted -> handed_off`; `prepared/admitted -> unknown`
on any possible side effect without durable evidence. Persist prepared attempt
and target binding before bytes. Accepted local admission is not model receipt,
model response or intentional approval. No exactly-once claim.

* Before native admission, no target or a conclusively refused stale target
  leaves pending. A replacement target can receive only a never-admitted notice
  or one whose old admission was conclusively retracted before handoff.
* Same notice/attempt/payload retries return existing admission facts. This
  requires concrete native deduplication/admission evidence; neither current
  `send_message` nor mailbox `drop_message` alone provides it. Do not advertise
  this guarantee until implemented and independently reviewed.
* Restart after prepared means potentially sent, even if producer got no reply.
  Observe exact durable native identity/route first. Waiting/Taken are evidence
  within that original target binding, not permission to resend. Missing files,
  read errors, session replacement, or an unavailable endpoint are Unknown,
  not evidence of nonacceptance. Persist/display reason; never silently retry
  or fallback into another epoch. A local mailbox take/read crash remains an
  explicitly limited handoff guarantee.
* Reconcile by matching exact native identity plus pinned destination/evidence;
  otherwise remain Unknown. Inspection is read-only. No owner decision is needed
  merely to leave ambiguous delivery visible; defining a resend override is
  outside this slice. Retain evidence until terminal outcome is durably recorded;
  cleanup cannot manufacture absence proof.
* Proposed finite delivery budget: 32 notices/pass and one outstanding admitted
  attempt per notice. Backlog remains durable rather than dropped/coalesced;
  unavailable/unknown items do not starve later independent notices. Retry only
  on explicit endpoint lifecycle/reconciliation events after proven nonadmission.
  Exact byte/deadline limits must use the allocated native seam's existing bounds
  and be frozen before builds. No unowned subprocess/thread, idle retries, or
  informational ACK deadline can block engine dispatch/check/review/rework/CI.

## Planned file ownership and review evidence

ov-933 proposes a dedicated `workflow_runtime/notices.rs` and dedicated tests
after allocation. ov-931 remains sole writer of `service.rs`, `main.rs`, `rpc.rs`,
runtime integration surfaces, protocol/config and Store/migrations. Selection
needs pure reducer/state and Store-transaction hooks: request explicit ownership
transfer or have ov-931 add the agreed hook, never concurrent shared edits.
Existing mailbox/route code may need narrowly allocated admission hardening;
that is new reviewable source, not an inherited receiver guarantee. No product
file list is authorized by publication of this report alone.

Required private evidence after review/allocation:

| Scenario | Required observation |
| --- | --- |
| Commit rollback / command replay | No orphan notice, cursor or native effect; identical accepted event has one row. |
| Equal-new events | Same rendered text, different event/revision produces distinct native identities/bytes. |
| Offline / restart backlog | Ordinary bootstrap creates selected notices while orchestrator absent; reopened private Store drains same IDs on authoritative target availability. |
| Target replacement / ABA | Replace role, lead/login, terminal epoch/session or process across admission await; zero stale-target bytes; old unknown does not move to new target. |
| Crash / reply loss | Fail before bytes, after bytes, after native acceptance, after receipt before producer reply; original identity observed, no blind duplicate; unprovable acceptance remains visibly unknown. |
| Native receipt limit | Actual isolated native mailbox/socket/producer bytes, including take-before-read crash and missing receipt; no fabricated provider exactly-once proof. |
| Engine independence | Ordinary configured engine advances dispatch/check/review/rework/landing/CI offline with no informational ACK. Separately prove intentional approval refuses stale/unauthenticated candidate/run/pin. |
| Policy/bounds | Omitted and explicit policy, immutable old-policy selection after reconfiguration, finite supported kinds, >32 backlog and unsupported endpoint without effect starvation or idle history polling. |
| Negative/restored | Mutate each authority/identity/atomicity guard independently, preserve actual failing evidence and restored pass on exact source; tests must cross real producer/admission boundaries. |

Future checks require the task's exclusive normal heavy FIFO, native Python,
jobs=6/profile=000/offline, load5<20, >10GiB reserve, existing warm-target
provenance and owned bounded supervisor/reaping. No checks/builds run in prep.
Before implementation: root composition/schema allocation and ov-931 API freeze.
Before acceptance: signed exact source/evidence freeze plus independent security
review, actual isolated ordinary producer/native evidence and all regressions.
Current acceptance items 1–4 remain open; this report supplies the early contract
and identifies concrete gaps rather than representing delivery as implemented.

## Implementation allocation (2026-11-07)

Released source is `77250986dcb4b1dbb6b63e06d0b11dc1b92d88a3`.
Read independent review at root reports/ov-933/contract-security-review.md.
Own protocol `workflow_config.rs`/`workflow_commands.rs`, CLI `workflow.rs`,
Store `workflow_notices.rs`, `workflow.rs`, `workflow_publication.rs`,
`migrate.rs`, `lib.rs`, daemon dedicated notice module and ordinary Service
owner/runtime notification wakes; update affected constructors/tests only.
The optional `notifications` bundle field omits None and uses a new bundle
pin domain only when present. Old immutable bundle rows are unchanged; a new
append-only policy table attaches only newly published pins. Selection runs
inside `apply_in` after accepted transition, never migrating old events.
Store APIs enumerate at most32 pending records after a key, prepare one exact
attempt before spawn and finish it once; recovery changes prepared to Unknown.
Get inspects bounded structured delivery records. This is independent of legacy
history and all effect/approval eligibility. Exact native admission authority
and endpoint bounds will be documented from current writers before native edits.

Native allocation/bounds frozen before checks: dedicated daemon workflow_notices
uses current unended orchestrator, unique live pane/tty, bounded owned ps/lsof
(discovery owner reused, visibility only), unique foreground-held main rollout,
thread/home metadata and kernel process incarnation. First path refuses member
leads/logins and competing targets. Store NativeTarget has no deserializer;
final Immediate transaction rechecks resource_version/epoch/lease/session,
current role/workspace, owner login/lead absence and stop/remove reservations,
then executes synchronous native recheck+spawn without awaits. This covers all
DB target writers, not only the seat lock. Admission linearizes at spawn; later
replacement cannot recall already admitted bytes. No eternal-current claim.
Maximum structured message16KiB, discovery15s, queue5s, one owned process group,
32-row indexed pages with cooperative yield and event wakes. Any post-spawn
nonzero/timeout/cancel is Unknown. Existing native queue argv only, no text
confirmation/fallback. Additional narrow native files: answer_wake codex_turn
visibility, lead_change/check_in_target lifecycle wakes, Store store.rs and
workspaces.rs target/session wakers, Store Cargo.toml/Cargo.lock test dependency
and affected tests. No RPC hook was necessary: actual Store mutation seams wake
without read traffic. A dedicated callback carries no payload or grant; only
notify_one, never acknowledgment or external delivery. Target attempt evidence
retains bounded exact structured terminal/epoch/lease/session/generation and
pane/process/thread/home/program facts, not merely their digest. Unknown cannot
be rebound or resent. Generation triggers are append-only in the next registry
entry and skip unassigned NULL workspaces. Member endpoints remain explicitly
unsupported by the first owner-Codex route.


Current v1 default is exactly WaitingApproval, WaitingReview, Uncertain, Done,
Failed, Cancelled. Blocked and StageChanged are optional selections. Omission
permanently names this v1 default; changing future defaults must introduce a
new policy version rather than retarget admitted bundles. A discovered native
rollout passes existing log_join::scan_head with nofollow/nonblocking open and
regular/single-link opened-descriptor validation, preserving its64-line/1MiB
bounds. That narrowly allocated metadata-read hardening prevents discovered
FIFOs/devices/symlinks from defeating the native discovery deadline.


Root allocation `039254bf`/`c2486954` adds only
`crates/tmux/src/inventory.rs` to the final native boundary. Its retained watch
publishes health and relevant tagged identity/TTY/PID/death/foreground-command
hint changes, excluding output/title/geometry and treating no hint as authority.
TaggedPane Eq ignores stamp.pid, so comparison explicitly includes it.
Publication wakes after all inventory locks release. The existing notice owner
subscribes before initial recovery/draining and selects watch changes alongside
its Store wake and shutdown; no new task, timer or producer loop. The owner never
refreshes inventory itself, preventing refresh/self-wake cycles. A watch version
is a coalesced hint, not a workflow cursor, informational ACK or grant.
