# Shared Session boundary for the next stacked delta

Status: **proposed, not implemented by the ov-925 wire pin**. The implemented API is `api.md`; Session currently has no workflow methods and the FFI method route explicitly returns None. Do not claim native connectivity from pure presentation fixtures or the CLI trace.

Dependency: stack approved ov-924 schema2 source/evidence `791345ee8` / `72ccadf5c` with reviewed ov-925. Neither tree currently contains the other's module. Preserve `workflow_presentation` and its existing C/JNI/WASM pure projection exports unchanged. No per-platform Intent/Command mapping.

Proposed Rust Session signatures:

```rust
async fn workflow_snapshot(&mut self, workspace: Uuid) -> Result<WorkflowSnapshot, SessionError>;
async fn workflow_prepare_action(
    &mut self, workspace: Uuid, item: &str,
    selected: workflow_presentation::Action,
    identity: WorkflowCommandIdentity, // stable command id + source sequence
) -> Result<PreparedWorkflowAction, SessionError>;
async fn workflow_submit_action(
    &mut self, prepared: &PreparedWorkflowAction,
) -> Result<WorkflowActionOutcome, SessionError>;
```

`WorkflowSnapshot` contains raw State/current configuration/notices plus **server-derived** workspace/epoch-scoped `workflow_presentation::Capabilities` and schema2 Presentation. The daemon's current wire getter returns raw State/config/notices only; the stacked delta must add the authenticated context output. The context is informational, never a request credential; server admission always recomputes its authority.

`workflow_prepare_action` fetches latest snapshot/context, runs the existing shared `workflow_presentation::action` against the exact opaque Action descriptor (kind/label/guard), and performs the single Rust Intent -> closed wire Action conversion. Guard/revision/epoch/run/pin/candidate/evidence stay bound to the inspected observation. A changed snapshot/context returns Stale; never replace selected evidence with unseen newer evidence. Inspection intents return the exact inspected evidence/uncertainty identity locally, not a fabricated operation result. Mutation intents return `PreparedWorkflowAction`, a versioned opaque serializable envelope containing workspace + the exact canonical closed Action. Cancel gets its observed run/pin from that validated snapshot. Review gets its exact observed evidence. No native semantic switch or eligibility calculation.

`workflow_submit_action` sends this exact prepared envelope through existing Session::value/capability negotiation. Daemon reauthenticates and validates revision/epoch/run/pin/evidence and policy; client-side preparation is not authority. On reply loss, preserve and resubmit the same PreparedWorkflowAction across connection replacement/process restoration. **Do not re-prepare against a newer snapshot when retrying**: that would invalidate the original action guard or silently change what the person approved. A new intent gets a new identity; id reuse with changed canonical content must conflict.

Native generic JSON driver names proposed for the stacked delta: `workflow.snapshot`, `workflow.prepare_action`, `workflow.submit_action`, over the existing `farcooler_client_call` Session handle. The schema2 renderer forwards the entire supplied Action to prepare; it treats PreparedWorkflowAction as opaque storage and forwards it to submit/retry. JNI uses the same existing call wrapper; WASM uses the existing Session call route. Export/declaration work must coordinate with ov-926 and use canonical binding conventions, not duplicate transport or Rust policy.

Exact bounded stacked work: (1) approved924 module dependency, (2) additive server-context output and typed Session getter, (3) one shared Intent-to-Action adapter plus prepare/submit structs/methods, (4) existing generic FFI/JNI/WASM call routing, (5) real producer/bridge/socket tests for schema2 action, auth/stale guard/evidence and prepared-request reconnect. No new workflow engine, renderer policy, provider or broad planning gate. That delta needs a source handoff/review against the combined dependencies before integration.
