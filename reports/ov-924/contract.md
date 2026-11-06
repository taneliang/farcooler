# Workflow presentation API v1

Source: `crates/client/src/workflow_presentation.rs`. Pure `project(&State,
&Capabilities) -> Result<Presentation, Error>` and `action(&State, &Capabilities,
item, &Action) -> Result<Intent, Error>`. One finite JSON `Request`/`Response`,
schema 1. `json(&str)` serializes exactly the same response for C, JNI, WASM.

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
