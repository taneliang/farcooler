# ov-927 Session routes (implementation under validation)

C: existing farcooler_client_call(handle, method, argsJSON) -> u64 ticket; farcooler_client_poll(handle) returns result JSON.
JNI: existing NativeClient.nativeCall(handle, method, argsJSON) -> long ticket, existing poll route.
WASM: WebRunner.workflow_call(method: string, argsJSON: string): Promise<string>.
All delegate to Session::workflow_call; no platform Intent/wire mapping.

- workflow.snapshot args {workspace: UUID}. Returns {schema:2,state,configuration,notices,context:{capabilities,binding},presentation:{schema:2,workspace,epoch,revision,runs:[schema2 cards]}}. Use this presentation directly: Action.guard includes authenticated admission binding. Pure workflowPresentation exports remain unchanged and do not imply authenticated connectivity.
- workflow.prepare_action args {workspace,item,action: ENTIRE selected Action,identity:{id,sequence,revision,epoch}}. Identity revision/epoch come from the displayed presentation. Returns {kind:"Mutation",prepared:{schema:1,workspace,json}} OR {kind:"Inspection",intent:{InspectLatestEvidence:{evidence}}|{InspectUncertainty:{effect,run,fence}}}. Keep prepared opaque and persist before submit. Stale selection errors use ResourceConflict/what=workflow_stale/retryable=false and require fresh inspection and a new intent, never auto-reprepare.
- workflow.submit_action args {prepared: ENTIRE prepared}. Returns daemon {schema:1,receipt:{...}}. Retain exact prepared for bounded retry on replacement Session. Transport correlation IDs can change; durable mutation identity and canonical payload bytes cannot. Never retry by calling prepare again.

Missing server context/unsupported capability fails closed. Caller capabilities, Grant, principal, snapshots and unknown top-level argument fields are rejected by route argument decoders. Server recomputes mutation authority on every submission, including receipt lookup.

Native/DOM implementation belongs to ov-926. No native connectivity or release proof is claimed by this Rust source.
