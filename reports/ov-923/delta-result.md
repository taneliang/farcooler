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
