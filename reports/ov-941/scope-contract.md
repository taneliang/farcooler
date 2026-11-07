# ov-941 scope correction and finite checks

Base: exact 54ad7d530cfb6a9f2abfcc2212bca481a61fec3d, branch ov-941-cli-rework-scope.
Controlling sources:2768c71b3 architecture9/P6/P8 and absolute scope-disposition
SHA256 43f2bdbd409de361988b3bf0a70b5c3f039dc62cce3bffd9fed81501f586d23d.

Change only crates/cli/src/workflow.rs, existing private daemon test module in
crates/daemon/src/workflow_runtime/wire_commands.rs, and reports/ov-941.
No registration/main/Store overlap with940. Shared validators/Session/native/Web
and journals remain byte-identical. Remove unshipped saved-file Retry and mandatory
prepared-file, including genuinely CLI-orphan helpers. Keep bounded evidence
reader, common reason validator and existing action/request/connect_to/send path.
Explicit same-destination retransmission requires unchanged reason/evidence contents,
run/pin/observation/mutation identity and selected authenticated runner/principal.
No snapshot refresh, new ID, auto retry or original-recipient artifact recovery.
Actual engine run retry/resolve semantics remain outside this pruning.

Private fixture: one temp root under /private/tmp with Runtime::create/open and
principal owner, existing coding/person-review fixture, no socket/server/provider.
Existing wire_commands.rs ignored test workflow_cli_payload_receipt_boundary has
seed and consume modes. Seed exports exact observed state/evidence and run/pin;
CLI ignored test workflow_cli_emit_receipt_fixture uses actual Cli parser ->
WorkflowCmd::action -> request to emit schema1 bytes twice; consume parses those
bytes and invokes private real Runtime::wire_command, real Store receipt boundary.
This proves same principal/runner receipt semantics, not transport authentication
or a real socket reconnect. Reopen/discarded answer and a fresh Runtime handle
represent lost-answer/reconnect at this private boundary. No mocked receipt logic.
Assert typed receipt success, exactly one accepted event/history/rework increment,
same receipt after immediate duplicate and reopen, no state/event change. Change
each reason/evidence/run/pin/revision/epoch/sequence under same ID: conflict.

Checks: focused CLI workflow tests; seed/production emit/consume sequence;
old-source CLI parser control must fail without prepared-file; fresh identity derived from current revision
control must fail receipt-equality assertion, restore exact original bytes
and rerun affected proof. Preserve all logs/reds separately. CLI/daemon only lint.
Use native Python PATH, canonical FIFO scripts/heavy load5<20, original923 target
only with actual post-lock holder/source/profile proof, jobs6/dev-test-debug0/
incremental0/offline/locked, >=10GiB reserve, owned foreground1800s cancel/reap.
No cleanup/fmt/stash/reset/amend/add-A, cold target, automatic retry, accepted46
or platform suites. Normal signed forward commits/hooks. Freeze exact signed pin
for SAME scope correction review; parent932 retains real app/platform/release.
