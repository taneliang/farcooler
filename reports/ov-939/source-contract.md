# ov-939 reasoned rework: source contract before implementation

Base: `82d6a878b3a34b9969d7e4b32bddc14a8a2cc45c`.
Branch: `ov-939-reasoned-rework`; one writer. This document is an implementation
proposal and seam inventory, not implementation or verification evidence.
No product source has changed. Protocol/daemon regions below require an exact
manager handoff before edits. Store/cutover regions remain exclusively ov-936.

## Observed source and framing corrections

* `crates/client/src/workflow_presentation.rs`: `RequestRework` is available
  below `max_reworks`, but maps to `Command::Review { passed: false }` without
  input. `Selection` contains only item/action; `Action` has kind/label/guard.
  `RunCard` already exposes iteration and remaining reworks.
* `crates/workflow/src/reducer.rs`: `Review` authenticates the decision grant,
  validates candidate/policy evidence, then `finish(false)` follows the pinned
  rework edge or fails. It does not retain a user reason. Exact latest evidence
  is checked by daemon admission, but the pure review path only checks evidence
  validity/candidate/policy, not equality to the retained latest evidence.
* `crates/workflow/src/definition.rs`: the existing finite budget is
  `max_reworks` (0..2), with a validated earlier Build/Action target. Stage
  retry attempts are a separate budget. No monetary or token budget exists in
  this slice; do not invent one or shift budget policy into renderers.
* `crates/client/src/session/workflow.rs`: authenticated context binds guards;
  prepare re-reads current state, authenticates selection and allocates identity;
  submit sends retained `PreparedWorkflowAction.json` unchanged. The mutation
  mapping currently has no reasoned variant.
* `crates/protocol/src/workflow_commands.rs` and daemon
  `workflow_runtime/wire_commands.rs`: closed JSON vocabulary/parser and
  authenticated principal-to-grant mapping; durable receipt lookup precedes
  revision/epoch/configuration checks. They must preserve this order.
* `crates/store/src/workflow.rs` and `workflow_publication.rs` persist entire
  Input/state JSON with atomic events/effects/receipts. Reason persistence can
  use those existing paths; no SQL migration, table or Store implementation
  change is proposed. This is a source observation, not restart/durability proof.
* Existing Swift/Kotlin/Web ports journal the displayed action and immutable
  prepared payload, keyed to account/host/workspace, with explicit retained
  retry. Their observation keys currently include the exact descriptor. User
  input must be separate from the descriptor or these keys cease to match.
* Swift `WorkflowAction` decodes `[String:String]`; a nested form added inside
  Action would break it. Put optional form data alongside RunCard actions and
  add explicit local DTO decoding. Retain the complete original Action.
* `crates/cli/src/workflow.rs`: existing CLI Review produces closed wire JSON
  directly using explicit identity fields. It does not use Session prepare.
  Preserve that authenticated CLI transport and exact-argument retry contract;
  do not claim all CLI mutations already use prepared Session envelopes.
* `reports/ov-924/handoff.md` calls a reasonless boolean review a "reasoned
  rework intent". `reports/ov-932/rebind-prep/next-phase.md` step 5 labels approval
  as A and evidence inspection as B. Preserve these historical reports, but
  supersede those claims here: A = EvidenceInvalidated + InspectLatestEvidence;
  B = reasoned RequestRework with durable reason and finite iteration budget.
  Neither real-app exercise is verified by this source allocation.

## Proposed exact shared semantics

1. Add a distinct `Command::RequestRework { item, run, evidence, reason }` and
   matching closed wire `Action::RequestRework { mutation, item, run, pin,
   evidence, reason }`. Use the existing workflow review RPC method/envelope;
   no protobuf tag, capability, RPC method or SQL version allocation. **Held
   pending manager region handoff**, including confirmation of this mapping.
   Keep historical Review serialization and semantics byte-for-byte unchanged;
   never infer or fabricate a reason for legacy failed-review events.
2. Shared Rust validates reason text: nonblank by Unicode whitespace, at most
   4096 UTF-8 bytes, no control characters except tab/CR/LF. Retain the accepted
   text exactly, including leading/trailing whitespace and Unicode. This is
   borrowed user text: display as text only; never parse as commands, environment,
   principal, configuration, provider instructions or executable operation data.
3. Reducer validates current input revision/epoch/source sequence and authenticated
   decision principal; WaitingReview/run, exact latest Evidence and current
   candidate/policy; valid definition target and `iteration < max_reworks`.
   Exhaustion refuses with Limit without changing state, rather than turning a
   RequestRework into RejectReview. Existing AcceptReview/RejectReview behavior
   stays on Review. Validation also protects direct pure-engine callers.
4. Append an accepted rework record to Item containing command ID, authenticated
   principal, observed revision, run, definition/policy pins, full Evidence,
   prior iteration and exact reason. Use a default-empty, skip-if-empty additive
   field so old snapshots serialize identically and old events replay without
   fabricated records. History is bounded by the existing max of two accepted
   reworks; legacy iterations may have no reason records. Validate records as
   data without treating them as authority. Persist through existing atomic
   Store state/event transaction; prove restart/replay using focused external
   tests, without modifying ov-936 Store files.
5. Add a finite optional rework form beside actions on RunCard, containing Rust
   label/help/submit labels, field key, byte bound and remaining count. Rust
   supplies typed invalid-reason/unavailable/stale/error data. Selection adds
   optional reason input, separate from its exact original Action. No generic
   UI toolkit, dynamic renderer language or local semantic-policy table.
6. Authenticated Session prepare accepts that optional input, reprojects and
   validates the original guarded Action against current authenticated state,
   invokes the shared action mapper and serializes the reasoned command once.
   C/JNI/WASM continue forwarding the existing generic Session call. IDs and
   revision/epoch retain their existing lossless string/UInt64 handling.
7. Swift shared Mac/iOS, Kotlin and Web use existing native multiline controls
   for this finite optional form. Drafts are scoped to displayed workspace,
   run and guard. Session journal entries retain the input separately; observation
   lookup uses original Action, while intent identity includes submitted input.
   Prepared retries never read current draft or call prepare again. Failed
   prepare has no submit; ambiguous prepare keeps its never-submitted marker.
   Form errors preserve the draft and focus; stale/replaced observations demand
   fresh inspection. Busy/disconnect, selection tickets and restored retry
   attribution retain their existing behavior. No client decides eligibility.
8. CLI adds an explicit reasoned rework subcommand with required reason and
   inspected run/pin/evidence/identity. Same shared reason validator and wire
   variant; existing text/JSON output and explicit unchanged retry arguments.
   Do not repurpose Review flags and alter old command meaning.
9. Historical prepared schema1 bytes remain accepted by their existing parser,
   submitted verbatim with no silent reprepare. Unsupported envelope versions
   explicitly fail. Old review bytes are never upgraded to reasoned requests.
   A historical accepted failed Review remains legacy behavior; new displayed
   RequestRework always produces the reasoned variant.

## File/rule ownership inventory

| Region | Proposed edit / rule | Allocation |
| --- | --- | --- |
| `crates/workflow/src/{state,reducer,lib}.rs`, focused tests | Reason validation, distinct command, bounded retained history, shared eligibility | ov-939; publish contract first |
| `crates/client/src/workflow_presentation{.rs,/display.rs,/tests.rs}`, testdata | Optional finite form, shared input/action/error contract, common fixtures | ov-939 |
| `crates/client/src/session/workflow{.rs,/tests.rs,/tests/socket.rs}` | Authenticated preparation, reason input and immutable retry proof | ov-939 |
| `crates/protocol/src/workflow_commands.rs` | One JSON Action variant + method mapping + strict parser tests | **Manager handoff required** |
| `crates/daemon/src/workflow_runtime/wire_commands.rs` | One authenticated variant mapping + current run/pin/evidence checks and receipt ordering; focused tests | **Manager handoff required** |
| New focused `crates/daemon/tests/*` if allocated | Owned offline runtime/store restart/retry proof only | Confirm exact test file with manager |
| `crates/cli/src/workflow.rs` | Explicit reasoned CLI producer and parser fixtures | ov-939, dependent on handoff |
| `apps/shared/AgentKit/Sources/AgentKit/Workflow{Runs,RunsView,Session}.swift`, corresponding focused tests | Shared Mac/iOS finite form, draft/errors, separate journal input | ov-939 |
| `apps/android/app/src/main/java/com/farcooler/ui/Workflow{Runs,RunsView,Session}.kt`, corresponding focused tests | Same finite form and journal input | ov-939 |
| `services/web/src/client/workflow{,-view,-session}.ts`, focused tests | Same finite form, text-only rendering, focus, separate input and exact retries | ov-939 |
| `crates/{android,web}/src/workflow.rs`, presentation bridge tests | Only if generic forwarding/schema test needs an actual adjustment | Inspect before change; no unnecessary ABI changes |
| `crates/store/**`, migrations, ownership/cutover, bootstrap, notices, daemon RPC/allowlist/protobuf | **No edits**; consume reviewed existing APIs | Exclusive ov-936 / manager |
| `reports/ov-939/**` | Plans, unchanged original failures, bounded checks, source/evidence hashes and review handoff | ov-939 |

## Review and release boundary

Freeze a normally signed source/API commit with changed-file/rule inventory,
then save bounded evidence and hash manifests. Ask orchestrator to allocate
independent scoped security and ports review of that exact pin/evidence, and
stop product changes for the same review. Do not compose before acceptance.
No reviewer or delegation is assumed from publishing this document.

All acceptance items remain open at this preparation pin. Actual full app/root,
C/JNI/WASM navigation, accessibility/device/keyboard/lifecycle proof, both true
feature exercises and composition with final ov-936 belong to ov-932/parent
allocation. No app, simulator, owner DB/config, provider, network, live receipt
or external-send action is authorized here.
