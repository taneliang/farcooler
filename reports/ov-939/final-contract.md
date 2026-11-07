# Implemented source/API contract — ov-939

Base `82d6a878b3a34b9969d7e4b32bddc14a8a2cc45c`; exact Review-region grant
`8fc7ffd6`. Initial source-first proposal is preserved at signed62b293e and in
source-contract.md, with its compatible transport/CLI corrections. This file
states the implementation that supersedes the historical distinct-variant
proposal. No composition or full-app/runtime approval is implied.

## Authority, reason, budget and durable data

The existing pure/wire Review has an optional `reason`, default/skip-if-none.
A present reason requires passed=false; the reducer validates 1..4096 UTF-8
bytes, nonblank Unicode whitespace, and no controls except tab/CR/LF. Accepted
text retains every byte, including whitespace, Unicode and markup-looking text.
It is displayed as plain text and never enters operation argv/environment,
configuration, principal, executable policy or provider commands.

The existing revision/epoch/source-sequence/duplicate fences and authenticated
decision grant remain authoritative. Reasoned review additionally requires
WaitingReview, the current run, exact retained evidence, candidate/policy and
remaining pinned rework budget. Definition validation owns the earlier
Build/Action rework target; max_reworks (0..2) owns remaining iterations. Stage
attempts are a separate retry budget; no financial/token policy is invented.
Exhausted reasoned requests refuse Limit without failing or advancing the item.

An accepted decision appends bounded Item.rework_history with ID, principal,
observed revision, run, definition pin, complete evidence (including policy),
prior iteration and exact reason. History is data, never a grant. Existing
atomic Store state/events/effects/receipts persist it; no SQL/Store edits.
Default-empty/skip-empty preserves historical state encoding. Old accepted
reasonless Review inputs replay with original semantics and produce no invented
history. They are not presented as reasoned decisions. New RequestRework
selections require reason input; legacy prepared bytes are never upgraded.

Wire Review remains workflow.review in the existing schema1/capability/RPC.
No tag, method, capability or schema number changed. Receipt lookup retains
its original order before revision/incarnation/evidence checks. Exact duplicates
return retained receipts, changed payload under the same ID conflicts, and
refusals that reach Store have durable receipts. Strict round-trip wire parsing
rejects null/nonstring reasons and unknown authority fields. Unsupported
prepared envelope versions explicitly fail.

## Shared selection/form/error/prepared contract

Presentation schema2 and the three-string Action descriptor are unchanged.
RunCard adds omitted-when-empty forms and rework_history. A finite ActionForm
contains action, label, help, submit_label and max_bytes; existing iteration and
reworks_remaining remain Rust-owned. Existing goldens/feature A bytes are not
regenerated. Selection optionally supplies reason separately from the descriptor.

Session preparation re-reads authenticated snapshot/context, checks original
observation and guarded action, then uses shared validate_input/action mapping.
Invalid input yields Validation {error:{code,text}} before identity allocation
or mutation preparation. Ports render that shared text and mark only the
never-submitted preparing marker abandoned; draft remains available. Stale or
revoked observations cannot be validated by changing the draft.

C/JNI/WASM generic workflow_call dispatch stays unchanged; no ABI or native
transport edit is necessary. Session serializes the exact accepted reason once
inside PreparedWorkflowAction.json. Its shared validate method checks schema
and finite mutation vocabulary without changing bytes. Submit and CLI retry
use that validator and forward the original JSON.

Swift shared Mac/iOS, Kotlin and Web render their existing finite native text
controls only when Rust supplies a matching form. They have no local reason
validator, rework/budget/authority switch or semantic copy map. Drafts bind to
run/guard; server text errors preserve them. Controllers retain existing busy,
disconnect, delayed-result, replaced-run and selection fences. Web draft/caret/
focus tests are simulated DOM proof; native accessibility/focus/navigation on
real roots/devices remains unverified.

Swift/Kotlin use a local {action,reason} wrapper within their existing string
port. Web passes reason separately. Each adapter observes the original Action,
retains full selection input in its preparing journal, then persists immutable
prepared bytes before submit. Editing a reason cannot replace an unconfirmed
intent. Restoration/reconnect never prepares again; explicit Retry submits the
retained envelope, and never-submitted marker recovery remains inspection-only.
Account/host/workspace scope and lossless UInt64/decimal IDs remain intact.

New CLI Rework retains explicit inspected run/pin/evidence/identity and uses the
same core reason validator/wire Review. It requires --prepared-file: create-new
0600, no-follow, file+parent sync before connecting/submitting. Existing files
are never overwritten. CLI Retry loads only that regular retained file, checks
the shared Session envelope validator and sends JSON verbatim over the existing
authenticated CLI Link. It does not read a new reason/evidence, allocate identity,
reprepare or rewrite the file. CLI transport remains Link rather than a new
Session connection; server authority/evidence/revision/incarnation fences are
identical. Existing CLI commands/defaults keep their prior behavior.

## Exact regions and verification boundary

Product regions: workflow state/reducer; protocol Review DTO; daemon Review
mapping and directly necessary fixtures; client presentation, Session and
workflow-only tests; CLI workflow producer; shared Apple WorkflowRuns/View/Session;
Android WorkflowRuns/View/Session; Web workflow/view/session and focused tests.
Only mechanical reason:None fields and the preexisting missing notifications:None
socket-fixture constructor change outside new test assertions. No Store,
migration, cutover, bootstrap, notice integration, RPC/allowlist/protobuf edit.
A changed-file/hash/rule inventory accompanies the frozen source.

One inherited socket output names its approval-only fixture "FeatureA"; that
label is not the actual A exercise. Earlier924 prose also called a boolean
review "reasoned";932 next-phase mislabeled A/B. Those originals remain intact.
Actual A = EvidenceInvalidated + InspectLatestEvidence; actual B = reasoned
RequestRework/retained reason/iteration/budget. Tests prove shared semantics and
private fixture transport/reopen behavior, not real app/root/device exercises.

Independent scoped security and ports review must inspect the signed frozen
source and bounded saved evidence before composition. Full app builds/current
C/JNI/WASM root navigation, actual keyboard/AX/device/process lifecycle, both
true feature exercises, final936 composition, owner-copy conservation/cutover
and parent/main gates remain ov-932/parent allocations. No mixed-version rollout,
owner data/config/secret/provider/network/app/display operation is claimed.
