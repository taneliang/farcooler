# ov-923 review corrections

Signed correction source: `74e8ccc8380da15c55531124a46b7094b205f779`.

This package is stacked on signed f98cdcd85 (source f52442374, reviewed core/store
bd87202f0). It addresses only the three P2 findings in the independent source review.
The original raw logs and receipts remain unchanged. The inherited result.md still
records the original source handoff; this report supersedes its pending-review status
only after the same reviewer audits this delta.

## Root causes and corrections

1. `run` held exclusive Runtime access without a concurrent command path. The new
   `command_handle(&Controller)` returns a cloneable bounded local inbox (32 slots).
   Its submit/decide methods forward private capability identities to the owning
   runtime's existing authentication and expected-revision admission. No serialized
   Grant, result, transport or provider was added. The loop services inputs between
   bounded effect sweeps. Dropped replies never undo accepted durable commands;
   retries preserve Request and now_ms, as before. Queue submission waits for capacity.
2. The unconditional 25ms loop scanned all history even after Done or uncertainty.
   The loop now recovers at startup, drains work, then waits for commands, explicit
   trusted-runner reconciliation wake, shutdown or the nearest actual unadmitted
   claim expiry. Read-only state requests do not wake the effect pump. The store's
   existing lease_until column is exposed additively on StoredEffect; no migration.
   Reconcile supplies no result: only the bound private journal can resolve work.
3. The fixture creation SHA was incorrectly used for every landing CAS. After
   claiming resource ownership and before admission, Provision durably pins the
   current landed SHA in a private per-workspace/item file. Every later effect,
   receipt, recovery, provisioning and landing CAS uses that immutable pin. A crash
   before admission retains/reuses it. Legacy admitted fixtures without a pin keep
   their original creation-base semantics. Missing/mismatched journals never replay.
   The exact update-ref compare-and-swap remains mandatory.

## Evidence and boundary

New tests cover owned-loop every-stage approval, unauthorized/stale/foreign
capability refusal, cancellation without stop/restart, idle full-sweep counts,
actual claim-expiry wake, explicit delayed receipt wake, and two coding items in
one root with a crash after the second Build receipt before workflow commit.
`delta-focused.log` initially passed 15 runtime tests; an initial visibility-only
compile failure is retained. Final restored evidence and negative controls are
recorded in `delta-validation.log` and its child logs.

`run-delta.py` preserves the normal heavy FIFO, jobs6, load5 below20, native
/usr/bin/python3, offline Cargo, debug0/incremental0, 1800-second supervised deadline
and 10GiB reserve. It reuses only the authorized original runtime target via
CARGO_TARGET_DIR. Tool-call preflight found no cargo/rustc workers or cargo-lock
holders. No cold duplicate target, deletion/copy, owner DB, installed-app change,
external push, provider, SDK, transport or production selector was used.

The inbox is an in-process authenticated embedding seam, not a durable transport:
command acceptance and receipts remain durable in SQLite, while queued requests
not yet accepted must be retried after restart. Cancellation remains between
bounded effects, not concurrent provider interruption. The sidecar pins and
journals retain the existing trusted same-user private fixture boundary.
Full combined gates/main CI, real provider/harness smoke and release integration
remain parent requirements. Stop at reviewed source handoff.

Final results: all three negative controls exit101 at intended assertions and
restored cases exit0; final current-source runtime suite 15 passed
(`delta-focused.log`, last run), affected store suite 13 passed (`delta-store.log`),
daemon/store all-target clippy -D warnings exit0 (`delta-clippy.log`, last run).
The first validation bundle stopped on clippy's inbox-size lint; it remains in
`delta-validation.log`. The only subsequent source change boxes the request;
final runtime and lint were repeated, without repeating unchanged store/mutations.
Whitespace/file-size checks pass. Exact hashes/toolchain/minimum disk reserve and
signature verification are retained in delta-source-sha256.json,
delta-provenance.json and delta-signature.log. Independent delta verdict pending.

## Same-reviewer publication correction

The first delta audit closed the original three findings and identified one P2
ordinary-crash boundary: the old durable writer could leave an empty/truncated
final base file before admission. Existence alone then admitted that effect before
JSON validation. That audit is authoritative; 74e8ccc is superseded for closure.

The narrow follow-up writes/fsyncs a unique private temporary file, atomically
renames it to the selected base, validates the selected bytes, and fsyncs the file
and containing directory before admission. Runtime uses that returned validated
base to construct the admitted binding. Existing valid pins are reused, not
resampled. Old incomplete final pins are repaired only if the sole item outbox
entry is this exact original Provision in claimed state; Store cannot transition
admitted back to claimed and never erases outbox history. Any other corrupt
historical/admitted pin is refused without repair or execution. Abandoned
publication temporaries are ignored and preserved.

New regressions create empty/truncated pre-admission final files and abandoned
temporaries, recover/reopen the second coding item, and prove it lands using the
first item's candidate as base. Another regression deliberately corrupts an
admitted legacy pin and verifies refusal without resampling or external execution.
These are deterministic crash-boundary fixtures, not a hardware power-loss proof.
The initial follow-up runtime suite passes 17 tests; publication-* logs record the
new negative control, restored/final runtime checks and lint. Unchanged store and
previous three negative-control evidence remain credited, not repeated.

Publication correction source: `096931d48b739004884702390445cbeb4d5d7db9`. Final publication-validation wrapper
exit0/cancelledFalse; publication-mutation exit101, restored exit0, final runtime
17 passed and daemon/store all-target clippy -D warnings exit0. Minimum free bytes
20,447,875,072. Exact final hashes/provenance and Good signature (unknown trust)
are saved under publication-source-sha256.json, publication-provenance.json and
publication-signature.log. Same-reviewer publication-only closure pending.
