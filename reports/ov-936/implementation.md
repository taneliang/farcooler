# ov-936 Store candidate — synthetic rehearsal, not integration approval

Built from immutable fae3f30d0c3d1c475f9dc4463243f293fd88b007 against approved
ov-935 bdff40f0b2944173c2be5b5fa906dcd2af88f7bf. Stop at a signed frozen
candidate for SAME independent data-safety/security review. This does not certify
Service bootstrap, native notice admission, a composed release, main CI, owner-copy
rehearsal, actual owner backup or installed cutover.

**Reserved integration remains open.** ov-937 owns Service::open/runtime roots/
earlymain; ov-938 owns notice producer/helper/tests. Required hooks were published
before execution and requested through task notes/messages. Those functions and
trees are not edited here. The seven-category synthetic quiescence evidence is
not an implementation of live process inventory/joining. A DB lock/incarnation
cannot stop an external process which already has writable resources. Composition
must supply actual joined-writer/resource-incarnation proof before using this API.
Native notice final admission also must consult the freeze/incarnation; a pending
or previously prepared notice is not itself authority to spawn during cutover.

## Data and authority

Append migration66 to the actual 65-entry registry; Older::Refused. No tables are
removed or renamed and no current source column is replaced. Every retained
SQLite value has a typed Cell (NULL/INTEGER/REAL bits/TEXT bytes/BLOB bytes), table
and primary-key identity, exact schema SQL and an injective per-field ledger.
Original relationship columns, equal-time IDs/implicit rowids, bytes/NULLs/JSON, UUIDs/aliases and
sqlite_sequence high-water marks remain. Same-count field corruption fails.
The task mapping binds workspace UUID + task UUID to an inert item UUID; it never
calls the reducer to manufacture gates, grants, accepted outcomes or effects.

Unsupported active legacy dependency/line/budget/check-in semantics and ambiguous
aliases are explicit Q rather than silently replaced by default engine policy.
Filesystem pointer strings are retained verbatim, never traversed from the DB;
unverified pointers have explicit dispositions. Unknown schema blocks release.
Unknown/conflicting engine JSON freezes its workspace before typed deserialization
could erase unknown fields. Historical Done/agent text remains provenance.

Global authority phases are legacy, quiesced, committed, released. One Immediate
transaction writes every workspace binding, every field ledger entry, task
mapping/quarantine, attempt/backup proof, new authority incarnation and resource
fences. Immutable existing engine events/receipts/effect payloads, bundles and
running definitions are retained. Resource generations increase by exactly one;
they never reset. Store handles capture their opening incarnation. Apply, wire
write, claim and admission check that incarnation under the same writer boundary.
Old claims and duplicate admissions cannot dispatch twice. Pending/admitted/
uncertain/settled/suppressed distinctions survive.

SQLite guards freeze execution-bearing task status/wait/identity changes,
line/work/landing/CI/wake/compose/check-in paths, task dependencies and task-bound
terminal re-admission/restart/relink/deletion, including direct Store writers and
cascades. Engine SQL writes/claims are also frozen during quiesced/committed
stages. Task content/notes and display/history readers remain. Historical rows
no longer prohibit a fresh validated engine item merely by existing; unresolved
or unsupported legacy constraints still prohibit conflicting admission. Fresh
engine ownership retains the existing UUID/alias and standalone-NULL semantics.

## Enforced private backup/commit/restore seam

preflight reads before Store startup sweep; open_for_cutover upgrades only the
explicit existing file and does not normalize waits. Ordinary frozen/transferred
reopens also do not sweep raw waits. Public engine_commit requires CommitPermit,
which has no wire constructor/deserializer. PrivateRehearsal mints it only after
closed DB/resource capture, checksum comparison, independent restore comparison
and an exclusive fsynced journal record outside the replaceable DB. Unit tests
have a cfg(test)-only permit explicitly labelled UNIT-ONLY-NO-BACKUP-CLAIM. They
are not substituted for the full file/public-permit or CLI evidence.

PrivateRehearsal creates a new 0700 root. No installed-path fallback, owner DB
lookup, provider calls or resource-pointer traversal exists. Copy operates only
on the explicitly created fixture tree. Regular files are read nofollow/nonblock
and must be single-link; symlinks retain target identity without following it;
special/multiply-linked files refuse. Directory modes/empty directories, dirty
bytes and attachment/config/ref stand-ins are included. Residual WAL/SHM/journal,
exclusive-lock/integrity/close failures, destination collisions and mismatched
resources stop the backup. This finite helper does not claim a general archive
or hardlink migration capability.

The external create-new journal survives restoration. DB authority is the crash
oracle even when outer stage text is stale. Actual SIGKILL/reap before and after
commit proves frozen precommit vs committed forward recovery. No downgrade into
a migrated DB occurs. Restore verifies the baseline into another new destination,
then freezes it with a new incarnation before any Store opens, imports exact
post-backup facts and quarantines the conflicting resources. Restored pending
state cannot authorize replay. Fake send/push/Enter/write counters count actual
fake adapter invocations outside the replaced DB; there are no real providers.
A lost local push receipt and ambiguous fake write stay fenced, including after
forward release; exact read-back must match operation/request/resource/incarnation.

The feature-gated engine_cutover_rehearsal example accepts only an explicit new
/private/tmp destination. It creates synthetic state, records preflight, freezes,
closes, backs up/restores independently, obtains the permit, commits/verifies,
restores again and prints zero external send/push/Enter counters. It never opens
an existing installation.

## Evidence and known integration failures

schema65.json comes from the real registry: 87 tables, 201 objects, 540 columns.
The all-column SQL fixture seeds every table/column with real FK/CHECK/trigger
validation; source engine JSON deliberately unsupported by the typed reader is
retained/Q, while F09 separately uses real typed publication/state/events/effects.
F01–F12 names and assertions are in tests.rs and tests/cases.rs. Full typed
manifests/ledger files are under fixtures/<run>/; counts alone cannot pass them.
The final file fixture also retains closed baseline/migrated/restored SQLite
artifacts, full filesystem manifests and F11 original external facts. Signed
*.json.gz/*.sqlite.gz copies decompress to the exact original bytes; fixture-
index.json binds raw and compressed SHA-256/length. Original private raw files
are retained locally and excluded from Git only by fixtures/.gitignore; nothing
is deleted or reformatted to reduce evidence size.
The all-field closed-file test separately exercises the public backup permit,
full resource manifest, independent restore and stable reopen.

manager-source-reconciliation.json verifies all88 approved product-pin source
hashes, resolves91 original file:line anchors, and identifies12 source files
changed in the supplied post933 base. The approved map's schema64 is historical;
actual entry65 is workflow_notices and this candidate appends66. Original
reports/receipts and obsolete readers are retained, not rewritten as new proof.

Real immutable schema65 (fae3f30d0) and schema64 (77250986d) Store::open helpers
both refuse the private migrated file with DomainError::NewerData. SHA-256 before
and after is identical; helper source/registry input hashes, actual commands and
private archive roots are in old-reader-receipts.json. The first archive omitted
proto build inputs and failed preparation; that receipt remains and is not a red
control. A subsequent cross-root cache reuse failed compilation when returning
from the schema64 build. Rebinding local dependency root mtimes under the same
FIFO fixes that actual dep-info hazard; no cleanup or cold target is used.

Six deliberate removed guards cover exact schema namespace identity, Store incarnation, pinned retired-writer schema,
raw-source conservation, restore freeze and post-backup quarantine. Each must
reach its intended Rust test failure (semantic assertion or explicit pinned-schema refusal); compile failure cannot count. Exact
original/mutant bytes and SHA-256, failure logs and restored flags are retained.
The evidence-label underscore failure and initial fixture/compile/lockfile/lint
failures are also retained as setup failures, not meaningful red controls.

Full Store final-first ran517 passing tests and found two specific failures:
(1) plan::no_existing_table_gained_a_column rejects the pre-existing frozen-base
workflow_excludes_ticket_lines_insert trigger; (2) the reserved notice migration
test assumes CURRENT_SCHEMA_VERSION-1 is pre-notice, now false at append66.
The latter needs a narrow pin to schema64 under an explicit owner-region handoff.
Neither test was edited or hidden. Later scoped Store checks name both exclusions;
no full-suite green is claimed. The full independent composition must resolve
these together with reviewed937/938 deltas and required runtime hook integration.

All heavy receipts use /usr/bin/python3, normal exclusive machine FIFO, original
ov-923 warm target, actual empty-holder/dep-info/source/profile provenance,
profile000/jobs6/offline, load5<20, minimum10GiB reserve, and owned1800s process-
group cancellation/reaping supervision. Final receipt names/counts and frozen
source/evidence hashes are recorded in freeze.json at handoff.

## Additional verified ordering counterexample

The approved map's stable (at,id) timeline description is insufficient for the
actual notes_for reader: tasks.rs:776 orders by at,rowid. The mapper now retains
implicit rowid identity, with explicit @rowid ledger entries in addition to all
540 declared columns. Two SQLite tables with identical typed fields/counts but
reversed equal-time insertion order fail conservation. Source task_notes are not
dropped/recreated and the existing reader is unchanged. This is conservation of
actual historical ordering, not a new UI order or fabricated note identity.

## Concrete reserved-region proposal (not applied)

reserved-notice-proposal.patch shows the exact two Store transaction checks
needed before notice preparation and final native spawn admission, plus the
pre-notice test's schema64 pin. It is a review artifact only: no reserved notice
function/test is edited in this candidate. It must be reconciled with reviewed
ov-938 source under an explicit region handoff, then verified with native queue
and cutover race controls. The bootstrap hook remains a separate ov-937-owned
integration: before starting ANY old/new watcher/scheduler/job/hook/notice owner,
read engine_authority; frozen or committed-unreleased state requires admin
recovery, not ordinary startup. Source/resource joining must be real runtime-owned
proof, never invented by a caller on the wire. Parent must review that combined
caller closure rather than treat this proposal or synthetic proof strings as an
installed-writer shutdown implementation.

## Final sealed checks

finish-root-fences.log/json binds the final product and test source:526 Store tests
pass,2 tests are explicitly excluded for the recorded integration failures, and2
are ignored (one pre-existing helper and the owned crash helper run by its parent).
All six deliberate guard removals fail meaningfully and restore exact source.
Store clippy (all targets, testing feature), migration lint and file-size budget
pass. Actual old-reader/CLI evidence is in seal-canonical-temp.log/json; no schema
or compatibility entrypoint changed afterward. Those tests exercise both pinned
old Store::open implementations and unchanged fixture bytes. restored-final.log/
json passes CLI workflow2, daemon workflow69, legacy workflow socket2 and ordinary
Service workflow socket10, with nested child summaries kept separate. Later
changes affect only cutover preflight/manifest/fixtures, not the exercised ordinary
legacy-phase runtime guard paths; the final Store suite covers those changes.

The final source namespace is pinned to exactly103 migration objects (hash
wf1:7871168a56707659c4aec1ed4e548970a69f4193f5f92726d37b7286a52a5e2d).
Unknown same-prefix schema cannot be excluded as migration metadata. Invalid,
exhausted or inconsistent resource-generation high-water marks refuse preflight
without writes; no SQLite numeric overflow or silent resource repair is accepted.
The all-field resource fixture now contains a real private Git repo with fixed
synthetic author/time, refs/objects/index and dirty/untracked binary files. It
restores Git HEAD, a historical blob and exact dirty status, plus config/rollout
files, symlink identity and empty directories. All Git commands clear inherited
configuration, disable system/global config/optional writes/signing, use private
empty templates/hooks, and make no network call. Actual closed DB and resource/
external-fact manifests are included in the signed compressed evidence index.

Full-task acceptance/composition remains subject to SAME independent safety
review and the explicit reserved bootstrap/notice integration handoffs described
above. The task is submitted for that review, not certified as an installed or
whole-release cutover.

The final resource boundary also validates live/journal/backup directories before
traversal; a replaced symlink root refuses before database or journal access.
Snapshot root mode is explicitly preserved. Unmapped special permission bits
refuse instead of silently stripping security-relevant mode information. The
private stand-in root/special-mode controls retain original bytes and prove no
write reaches the other stand-in. This is a finite closed-writer fixture seam,
not a general concurrent filesystem archive or installed-writer shutdown proof.
