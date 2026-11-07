# ov-940 phase 2 status (private implementation; not a release claim)

Built and tested (store 545 passed/0 failed/2 ignored; daemon affected suites green; clippy store+daemon clean):

* `startup::ordinary_admission` read-only gate in `Store::open`, `Service::open_in`, `open_selected` and main after the lock (stream/stdio inherit via Service). `startup::open_recovery` is the non-normalizing admin open.
* `owners::OwnerJoin`/`JoinedOwners` (no Clone/serde; quiesce/commit/release require it, bound to DB inode and attempt). `Service::join_writers` closes `WriterGate`, stops tails and workflows, then issues it. Gate tickets: check-in scheduler, Watcher follow/holds/collect, pumps, landing jobs, hook connections and tails.
* `transfer::begin`: exclusive artifact dir, synced baseline + intent, then one transaction of DDL, version, refusing floor and quiesced authority. Rollback, tamper and foreign-proof cases tested.
* Registry 67: manual delivery binding, fenced old message rows, notice eligibility floor, freeze checks at notice prepare and admission.
* effect_runner owned-group reap-before-signal fix (earlier commit).

Open, not claimed:

* Authenticated recovery RPC (`workflow.cutover` method group) and CLI entry are NOT built. Recovery is reachable only as library calls (`open_recovery`, `validate_artifacts`).
* Main-spawned observers (ci_watch, pr_watch, pr_review, landing_read, paste sweep, config/log watchers) are not ticketed; `transfer::begin` re-hashes the DB before upgrade as a guard only.
* Fresh installs keep the migration `legacy` phase (engine-only fresh authority would abort legacy writes in most daemon tests).
* No real schema60/old-reader Store control; the floor is asserted by value only.
* Delivery typing adapter is the existing pump, now fed only bound rows; no end-to-end manual-delivery daemon test beyond route tests.
* One unrelated flake seen: `fs_watch::settle::tests::a_sentinel_routed_away_fails_instead_of_retrying`.
