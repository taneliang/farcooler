# Workflow presentation API v2

Source: `crates/client/src/workflow_presentation.rs`. Pure `project(&State,
&Capabilities) -> Result<Presentation, Error>` and `action(&State, &Capabilities,
item, &Action) -> Result<Intent, Error>`. One finite JSON `Request`/`Response`,
schema 2. `json(&str)` serializes exactly the same response for C, JNI, WASM.

Exports: C `farcooler_client_workflow_presentation`, JNI
`Java_com_farcooler_core_NativeClient_nativeWorkflowPresentation`, browser
`workflowPresentation` (Rust `farcooler_web::workflow::presentation`). C uses
existing guarded/null/size-query/short-buffer untouched conventions; JNI uses
existing string/two-call helper. No handle or retained allocation. Error values
are stable enum words in the versioned response; malformed input is Invalid.

Capabilities are explicit workspace/epoch-scoped inputs FROM an authenticated
host adapter. JSON is a local DTO, never a credential. This pure API performs
no transport or execution. The future transport must authenticate, source the
capabilities independently of caller JSON, re-read latest State, validate the
action, and submit the resulting command through reducer/store authorization.
The digest is a stale-action guard, not an authentication token.

Cards preserve immutable run/definition/policy/candidate/latest evidence,
current StageCard and explicit GateCard (Approval/Review + principal), plus ALL
stage kinds and progression policies, iteration, attempt,
fence, pending effect, reasons and finite action descriptors. EvidenceInvalidated
means retained evidence differs from current candidate or policy. It never
erases the receipt. InspectLatestEvidence returns that precise retained evidence,
even if invalidated. It does not imply a remote artifact has been fetched.
Rework eligibility follows the definition bound and reviewer principal; stale
or invalidated review evidence cannot authorize a review/rework intent.
Exhausted rework exposes RejectReview, producing the reducer rejection/Failed
intent rather than forcing approval. Uncertainty offers inspection, not invented
reconciliation authority. Snapshot/context hashing is once per projection, then
item-bound, so producing cards does not serialize the whole snapshot per item.

Native screens, Kotlin declarations/consumers, browser JS consumers,
authenticated workflow transport and full release/UI parity remain pending.

## Shared display copy (schema 2)

The revised schema requires `Request.schema = 2` and returns `Response.schema = 2`.
The workflow snapshot itself still uses workflow State schema 1. Bare-enum
presentation schema 1 requests are rejected; consumers must use this revised
contract before integrating. No transport/Session/proto contract is changed.

`RunCard.label` and `phase_label` are directly displayable. `StageCard.label` and
`progression_label` include finite stage copy and configured approval copy.
`GateCard.label`/`text` describe the already-derived gate and its principal.
`RunCard.reasons` is now a list of `ReasonCard { kind, label, text }`. Every
`Action` includes `label` alongside its unchanged `kind`/`guard` identity fields.
Shells fill existing text/button slots from these strings and forward the whole
supplied Action descriptor unchanged; they do not switch on the semantic IDs
for copy or eligibility. Semantic IDs remain for inspection/testing/dispatch.

Feature A therefore supplies **Evidence Invalidated**, **The latest evidence no
longer matches the current candidate or policy.**, and **Inspect Latest Evidence…**
from Rust alone. `workflow_presentation_generic_display_slots` exercises a
renderer with no kind/phase/stage switch and forwards the supplied inspection
button through the shared JSON API. All five golden fixture responses carry the
same copy through C and Web, and JNI delegates the same C implementation.

This is finite workflow copy in `workflow_presentation/display.rs`, not a new UI
component system, plugin registry, localization framework or platform reducer.
No tone abstraction is needed to display the new text in existing generic slots.
