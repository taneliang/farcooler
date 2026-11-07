# ov-940 phase 2 status (private implementation; not a release claim)

Built and tested (store 545 passed/0 failed/2 ignored; daemon affected suites green; clippy store+daemon clean):

* `startup::ordinary_admission` read-only gate in `Store::open`, `Service::open_in`, `open_selected` and main after the lock (stream/stdio inherit via Service). `startup::open_recovery` is the non-normalizing admin open.
* `owners::OwnerJoin`/`JoinedOwners` (no Clone/serde; quiesce/commit/release require it, bound to DB inode and attempt). `Service::join_writers` closes `WriterGate`, stops tails and workflows, then issues it. Gate tickets: check-in scheduler, Watcher follow/holds/collect, pumps, landing jobs, hook connections and tails.
* `transfer::begin`: exclusive artifact dir, synced baseline + intent, then one transaction of DDL, version, refusing floor and quiesced authority. Rollback, tamper and foreign-proof cases tested.
* Registry 67: manual delivery binding, fenced old message rows, notice eligibility floor, freeze checks at notice prepare and admission.
* effect_runner owned-group reap-before-signal fix (earlier commit).

Added after the orchestrator rulings (R-97 deferred: fresh installs stay legacy, no fence ported or narrowed):

* `workflow.cutover` closed method group (protocol `workflow_cutover.rs`, HostAdmin scope, owner-only): live daemon serves Inspect and Prepare (join + atomic transfer); a recovery daemon (`rpc::recovery`, served by main when the gate refuses) serves Inspect, Verify, Release, Finish from baseline + DB truth for the local owner only. Commit is deliberately absent (needs the closed-backup permit). CLI: `farcooler workflow-cutover` in `workflow_cutover.rs`; 941 `workflow.rs` untouched.
* Tickets on CI, PR, review and landing-read loops, paste sweep and workspace-home preparation.
* Real pre-cutover reader control: the d43e8f99 Store opens a registry-65 file and is refused (`NewerData`) on the file `transfer::begin` leaves (`old-reader-result.json`).
* End-to-end manual delivery across a cutover (`messages_tests`): pre-cutover queued row inert and retained, fresh manual message told once. Released phase is set by SQL there; commit/release are covered by the store tests.
* `transfer::finish` records the terminal disposition only for a released attempt matching its baseline.

Open, not claimed:

* Commit/backup/permit over RPC (parent 932). Recovery Release proof covers this process only.
* Fresh installs stay legacy (R-97); fences untouched pending the spike.
* Startup one-shots (tunnel, derp) are not ticketed: they write no cutover-protected state.
* Unrelated flake seen: `fs_watch::settle::tests::a_sentinel_routed_away_fails_instead_of_retrying`.
