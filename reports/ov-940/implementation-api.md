# ov-940 phase2 source and API log

Base: `7f6edddd2ab9541d8cb0fc8a899cfe1a4ac486a1`, dedicated
`ov-940-engine-cutover-implementation`. Corrected936 `93eb86c4`, approved937
`e1203d8c` / `f4020a91`, approved938 `4c645bb7`, and939 `821194fa` / `d43e8f99`
are bound by the unchanged composition.json. CLI workflow.rs is941-owned.

## First changed boundaries

* `effect_runner::exited(pid)` peeks WNOWAIT through synchronous `has_exited`.
  `supervise`: owned spawn -> retained Group/Child -> peek/deadline/cancel ->
  one disarmed Group::kill -> bounded Child::wait -> ESRCH-only Group::gone.
  Former code awaited Child::wait before numeric group signal and treated any
  failed signal0 probe as disappearance. `workflow_notices::queue_exited`
  delegates to the shared observation helper;938 cleanup ordering is retained.
* Notice migration fixture starts at registry64; the notice migration is65.
  `CURRENT_SCHEMA_VERSION-1` had started after that migration once66 existed.
* Plan-layer no-column/no-trigger invariant checks its actual historical entry58
  catalog. Later ownership/cutover triggers remain production requirements.

## Next implementation boundaries, not yet completed

`engine_cutover::startup::ordinary_admission(path)` will inspect transfer
artifacts, schema/floor and authority read-only before any normalization. Normal
main calls it before kept_log and again after daemon lock; selected Service
calls before ensure_dir; open_in before install ID; Store::open before init.
Stream and standalone stdio inherit the shared Service gate. Recovery needs a
separate authenticated non-normalizing seam; ordinary refusal must not imply
recovery completion. Fresh installs need atomic engine-only authority. Existing
synthetic legacy fixture entrypoints must be explicit and test-only.

Production join proof must be opaque, noncloneable and issued after retained
handles/resources are joined. Existing serializable Quiescence is evidence only.
Raw60 publication must precede intent and first upgrade write, followed by one
transaction containing registry DDL/version/refusing floor/frozen attempt. No
current code or test receipt yet establishes these required boundaries.

## Checks so far

`store-invariants-first`:535 passed,0 failed,2 ignored; exit0, source unchanged.
`groups-first`:compile101, siginfo_t was live across await and not Send; no tests
selected/executed claim. Corrected by synchronous peek. Original log retained.

`groups-send-fix`:18 passed,0 failed,0 ignored. `first-controls` restored both
original Store assertion files and the actual reap-before-signal await at the
configured runner callsite: each selected exactly one behavioral failure
(0 passed/1 failed/0 ignored), exit101, all green hashes restored. This does
not count the earlier compiler failure as a behavioral red.

`notices-after-helper`:8 passed,0 failed,0 ignored. The initial copied wrapper
did not bind all four task-required fixture environment paths; receipts are
retained with that limitation. Existing tests used explicit owned roots/native
programs and core config has test-binary isolation. The corrected wrapper binds
literal private FARCOOLER_HOME/FARCOOLER_CONFIG/CLAUDE_CONFIG_DIR/TMUX_TMPDIR,
without changing TMPDIR. `groups-notices-private-envelope` then passed18 runtime
and8 native-notice tests, each0 failed/0 ignored. Source remained unchanged.
`first-lint`:Store/daemon all-target clippy exit0. Migration and file-size scripts
also exit0. No ceilings changed. The535 Store suite still includes2 ignored
tests; no whole-suite zero-ignore claim is made.

First synthetic fixture bytes are archived in first-fixtures.tar.gz with the
per-file first-fixtures-index.json; original generated fixture files remain
local. These contain synthetic data only. All historical936 receipts and sealed
phase1 reports remain unchanged.

This is an incremental source log, not a frozen acceptance or release handoff.
Actual owned-writer proof, raw60, early production gates, recovery, retained
manual messaging/native eligibility and their review remain required.
