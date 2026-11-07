# S1/S2 storage repair contract

Base: signed e853d9c4ee07fbc07ab8e6d8bc0ce7d1658c57ba, branch ov-936-store-safety-fixes. Read SAME independent-review.md S1/S2 before edits.

Product regions: engine_cutover/rehearsal.rs metadata reader and restore journal validation; engine_cutover.rs executable outbox preflight; workflow_effects.rs shared lossless Effect/row binding check and actual claim/admit refusal. Focused tests in engine_cutover/tests/storage_safety.rs (new module). No public API/schema/core change. Existing raw evidence stays unchanged.

S1: finite 16 MiB metadata limit, descriptor O_NOFOLLOW|O_NONBLOCK, regular/single-link check before bounded read; use for SNAPSHOT.json and effect JSON in restore/reconcile/commit_permit. Validate root/journal before enumeration. Closed-writer assumption remains; no concurrent filesystem isolation claim. Private controls cover leaf symlink, FIFO, hardlink, oversized metadata and replaced journal; no external import, no fake adapter calls, stand-in bytes unchanged.

S2: lossless supported Effect JSON plus payload hash/id and current state item/run/resource/fence/epoch/pins binding. Preflight quarantines exact workspace and resource for pending/claimed/admitted invalid payloads. Actual claim/admit independently refuse invalid payloads in their existing Immediate transactions, including legacy phase and after a previous claim. Raw outbox bytes and settled history remain untouched. Unknown Effect/OperationSpec fields, row mismatch and valid positive controls exercise this boundary.

Checks: only focused storage tests plus affected workflow tests and Store clippy under canonical heavy FIFO, original ov-923 warm target, jobs6/debug0/test0/incremental0/offline/load5<20, 10GiB reserve, owned 1800s cancellation/reap. Save source hashes, post-lock holder/target/profile proof, raw red failures and exact restored source. No full Store rerun or exclusion edits. Signed frozen checkpoint followed by SAME S1/S2 delta review; raw60, actual writer proof, 937/938/manual messaging and full-suite exclusions remain parent blockers.
