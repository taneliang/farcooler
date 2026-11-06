# ov-923 runtime source handoff

Signed source pin: f52442374, stacked on reviewed signed
bd87202f0fa7628c8905e5179e6fd5d8c408002c. Early API pin:33a20d19c.
The isolated runtime now claims/admit-CAS durable effects, executes finite bound
adapters, reconciles accepted receipts without replay, releases blocked work via
Advance and drives routine edges without a controller or informational ACK.
See contract.md for the exact public seam, trusted local capabilities and limits.

## Evidence

- `final-focused.log`:12 passed, no failures/ignored. Covers offline automatic
  rework/Done, no-review/automatic/selected/every-stage approval policies,
  unauthorized/stale decisions, human semantic review, pre-admission/admitted/
  accepted-result-before-commit/cancel recovery, every adapter receipt before
  workflow commit, corrupt/wrong-generation receipt rejection, lifetime writer
  exclusion, process timeout/cancellation cleanup, resource release/recovery,
  stable withheld notice reads and pure replay.
- Generated tracker recovery test:64 workspace traces under two receipt-arrival
  schedules,160 accepted events,32 wrong-generation receipts refused. Deterministic
  transition equality and full persisted-input replay checked. This is bounded
  evidence, not formal verification or exhaustive schedule exploration.
- Three negative controls (`mutation-{binding,advance,decision}.log`) each failed
  at its intended assertion with exit101; the restored versions each pass exit0.
  `checks.py` restores exact bytes and verifies hashes even on ordinary interruption.
- `retained-gate.log`:18 retained landing tests passed. The shared process runner
  also closes a verified gap in the old gate: background children after a successful
  shell exit are killed and checked for quiescence. The gate retains its environment.
- `store.log`:13 workflow tests passed, including temporary SQLite reopen,
  concurrent CAS/resource admission, replay, SQL rollback, and migration from all61
  registered source schema positions. No schema change in this package.
- `clippy.log`:daemon/store all targets with -D warnings, exit0. Source whitespace
  and file-size checks passed. No formatter or full combined release gate ran.
- `trace.log`/`trace/trace.json`:standalone public API smoke (production library
  configuration, no test Git override),10 accepted events,10 deterministic notices,
  nine settled operation journals and25 raw receipt/log files. One reviewer rejection
  causes rework, then exact candidate32b83cb71461de6ddee12f358e86345d12b82a7b reaches
  Done through passing check/review, local landing CAS and exact-SHA CI fixture.
  No controller commands or notice reads occur between enqueue and Done.
  `trace/origin.txt` retains the private /tmp repository/database location; raw
  receipts are copied under trace/receipts. No owner data was read/imported.

All heavy routes used normal machine FIFO, jobs6/load5<20, native /usr/bin/python3,
offline Cargo, debug0/incremental0,1800-second supervisor deadline,10GiB reserve
and owned group cancellation/reap. Validation minimum free space20,981,166,080 bytes;
standalone smoke minimum20,865,556,480. Source hashes and tool/profile details are in
source-sha256.json and validation-provenance.json. Initial compile/setup failures
remain in focused.log; final-focused.log is the restored-source certification.

## Review and release boundary

Independent security/data-safety review requested from the orchestrator at source
pin f52442374; its verdict must be attached before this handoff is complete.

This is trusted deterministic fixture execution, not real provider certification.
The actual guarded Git worktree/revision primitives, retained gate process mechanics
and retained CI parser/summarizer run; the harness/reviewer/CI provider is a local
fixture and landing never pushes. No Service/main runtime caller, installed app,
owner database, external provider, ACP/TUI, SDK, adapter registry or production
selector was activated. The private journal is trusted same-user storage, not an
untrusted-code sandbox. Root construction/local capabilities are the embedding
boundary; future remote callers must use existing authenticated admission.

Missing exact settlement keeps uncertainty and reservations; it does not authorize
retry or arbitrary serialized Resolve. Effects are serial and bounded, with command
admission between effects. Actual concurrent harness cancellation/provider-specific
reconciliation must be integrated before connecting production authority. Shared
clients/CLI transport, subscription reconnect behavior, combined gates/main CI,
real-provider smoke, conservation/removal and installed cutover remain parent release
requirements. No production completeness or owner-data migration is claimed here.
