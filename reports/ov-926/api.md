# Thin workflow renderer seam (pending ov-927 integration after ov-925 review)

Baseline: reviewed schema 2 source791345ee8, evidence72ccadf5c. Each shell
consumes `Presentation` directly, not State or user-provided Capabilities.

* Swift `WorkflowPort.read(host, workspace) async throws -> String` returns
  schema-2 Presentation JSON (not Response). `dispatch(host, workspace, item,
  actionJSON) async throws -> String` forwards the complete Action object.
  `WorkflowPort.id` rotates per authenticated connection; inject nil on
  disconnect, triggering display invalidation automatically.
  Inject `.environment(\.workflowPort, port)` above workspace navigation.
* Kotlin `WorkflowPort` has the same suspend read/dispatch signatures.
  Inject `LocalWorkflowPort` above the workspace route; replace it with the
  new session adapter on reconnect and nil on disconnect.
* Web `WorkflowPort.read(workspace) -> Promise<WorkflowPresentation>` and
  `dispatch(workspace, item, action) -> Promise<string>`; `installWorkflowPort`
  installs a getter resolving the CURRENT runner/session, never a retained
  disconnected session. Numeric u64 fields must be decoded losslessly by the
  adapter (string/bigint at the JS edge); action descriptor fields are opaque.

Dispatch returns inspection identity/result text for an existing selectable
text slot; no artifact fetch is implied. Errors reject/throw, then invalidate
local readiness and refresh. Authenticated adapter MUST independently source
capabilities, re-read latest State, call shared Rust action, and map resulting
Intent through ONE Rust command adapter into authorized transport. Digest is
only a stale guard. Shells never create Grant/commands/receipts/ACKs.

Default port is absent and screen states it is not connected. No raw-state RPC
or protocol route is guessed. Actual execution integration is explicitly
PENDING ov-927's reviewed Session implementation after ov-925 review. There is no production activation,
installed-app edit, owner-data access or UI/device proof in this seam.

The authenticated adapter must own command identity, lost-reply receipt retry
and session reconnect independently of a view's lifetime. Cancelling a UI await
must not cancel accepted runtime work or reset a command identity. The display
controllers never retry a mutation automatically. Connect/disconnect host hooks
must invalidate readiness (`WorkflowRunsModel.disconnect`/Web sheet.connection)
and refresh after a new authenticated session before allowing clicks.

## Manager ownership clarification at final handoff

Task decisions c424de3e/2f4203a4/504b425b assign the actual Session adapter to
**ov-927**, after corrected ov-925 security review closes. The proposed contract
is `ov-925-workflow-commands/reports/ov-925/session-contract.md` (not implemented):
`workflow.snapshot`, `workflow.prepare_action`, `workflow.submit_action` over
the existing generic Session handle. Native ports may encapsulate those calls:
read returns only snapshot.presentation; dispatch forwards the original whole
Action into prepare ONCE, then uses exact opaque Prepared bytes for submission.
The shared client driver must retain pending Prepared bytes through errors,
refresh, disconnect, view cancellation and restoration, and retry those bytes
before preparing a replacement mutation. No renderer refresh may mint a new
identity for unresolved work. This is a host integration obligation, not code
implemented by the present renderer source. Source comments naming ov-925
refer to the upstream transport dependency; concrete driver ownership is927.
