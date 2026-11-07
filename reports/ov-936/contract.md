# ov-936 finite implementation contract

Base fae3f30d0c3d1c475f9dc4463243f293fd88b007, approved map
bdff40f0b2944173c2be5b5fa906dcd2af88f7bf. Registry at this base is 65,
including workflow_notices. Append entry 66 (Older::Refused); do not reuse
historic suffix/schema numbers. Old-reader identities to exercise: the actual
Store::open implementation at fae3f30d0 (schema65) and approved-map parent
77250986d (schema64), private fixtures only. Neither is release approval.

## Files and API

Own crates/store/src/engine_cutover{,/manifest,/rehearsal,/tests}.rs,
append-only migrate.rs entry and lib.rs registration; narrow guards in
store.rs, workflow.rs, workflow_effects.rs, workflow_publication.rs and workflow_ownership.rs. No
Service::open, paths/log roots, earlymain, notice queue/helper/test changes.
No generic archive: preserve existing tables at original keys. A per-field typed
manifest and injective retained-record ledger compare every value, schema,
relationship and allocator; task UUID/workspace binds the engine item. Imported
legacy status/text remains inert provenance and never calls the reducer.
Unknown/ambiguous facts are explicit quarantine, not success. Retained history
is allowed beside a fresh validated engine admission only after transfer.

Typed entrypoints: preflight (read without Store startup sweep), quiesce
(external joined-writer/resource-incarnation evidence), commit (one immediate
transaction covers every workspace, ledger, global authority, compatibility and
resource fences), recover (DB authority wins over stale outer journal; after
commit forward only). Keep an exclusive create-new durable journal outside the
replaceable DB; retain original attempts/errors. Freeze both engine and legacy
admissions before backup/map. A Store opened before a new authority incarnation
cannot apply/admit afterward. No interactive running-definition migration.

## Required bootstrap hook — reserved region handoff requested

Before any scheduler/jobs/hooks/effects start, bootstrap must read cutover status.
Prepared/quiesced/backed_up/mapping: refuse dispatch and require admin recovery.
Committed/verified: attach only a fresh Store incarnation; old services must be
joined and their process groups/resources proved stopped before quiesce can pass.
A durable DB fence cannot kill an external process. Runtime must supply exhaustive
writer inventory, stopped incarnation/resource evidence and no-writable-resource
proof; missing proof blocks transfer. New writer starts frozen until conservation
and external effect reconciliation pass. No edits to reserved bootstrap regions
without explicit handoff. Native notice producer also needs a freeze/admission
check before any prepared attempt/spawn; ov-938 owns that region.

## Private finite fixture/effect boundary

Only explicitly supplied private temporary fixture roots. No installed path
fallback, owner discovery, provider/config/secret/backlog reads or writes. Resource
copy never follows a symlink or a DB path into another root. Capture regular files,
mode/link identity, missing-pointer quarantine, dirty bytes and explicit pointers.
Closed DB and resources only, no live/WAL copy shortcut; create-new backup names,
checksum validation and independent restore before trusting backup. Reconciliation
ledger survives restoration and fences post-backup operations. Fake adapters
increment explicit send/push/Enter counters; no network/subprocess provider sends.

F01 identities/history/dependencies; F02 line/stack/equivalence/NULL;
F03 agents/work/panes; F04 landing/CI/history; F05 budgets/usage;
F06 wakes/compose/images; F07 all finite check-in states/fences;
F08 all current retained families/columns/security/content;
F09 existing engine/pins/immutable effects and generation>1;
F10 ambiguity/unknown/invalid waits/standalone;
F11 fake accepted push/lost response and fake write/stale claims;
F12 pre58 actual registry upgrade plus independent closed restore.
Each compares typed fields and relationship sets, never counts alone. Red controls:
field corruption, missing resource, nonclosed backup/collision, mid-transaction
failure, stale incarnation/claim, admitted-effect replay, actual old-reader open.
Reopen twice measures normalization, preserving invalid raw waits. Interrupt before
and after commit, reconcile post-backup effects without duplicate counters.

## Verification and remaining gates

This contract is published before fixture execution. Private reversible tests are
authorized by ov-936. Heavy checks only through the normal exclusive original923
warm target/FIFO with actual provenance, native /usr/bin/python3, profile000,
jobs6/offline, load5<20, 10GiB reserve and owned1800s cancel/reap supervisor.
Keep all failed/red/restored receipts and source bytes. Normal signed commits;
freeze final source for independent data-safety/security review. Composition
must reconcile reviewed ov-937/938 deltas. Main CI, owner-copy rehearsal, real
backup, installed writer inventory/cutover and whole-release gates remain parent
work. This file is an early contract, not evidence that those checks passed.

## Final contract refinements recorded during implementation

The source reader's equal-time task-note order uses implicit rowid; preserve it
as @rowid ledger evidence rather than substituting UUID order. Public commit
requires the opaque backup/journal-bound CommitPermit; the cfg(test)-only unit
permit is explicitly labelled as no backup proof. Completed line/landing history
is inert and permits fresh validated admission; active dependencies, task/theme
budgets, landing/person policy and unknown state remain scoped Q. Frozen engine
SQL writes are refused as well as current Store API calls. Explicit admin opens
stamp the actual refused compatibility floor without normalizing raw waits.

Bootstrap ordering is part of the reserved handoff: use read_schema/read-only raw
preflight before any ordinary Store::open on legacy data. Ordinary legacy open
can migrate/sweep waits; it is not the conservation preflight. Route the frozen
admin transition through open_for_cutover before starting Service writers. After
commit, only the new writer may start, and dispatch remains held until verification
and post-backup reconciliation. New native admission checks must use the same
Store incarnation/freeze transaction, including attempts prepared before freeze.
