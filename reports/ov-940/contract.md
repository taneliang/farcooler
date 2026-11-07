# ov-940 — source-preparation handoff

Phase 1 only. No product implementation or execution approval. The signed Git
commit containing this package is the handoff pin; `source-inventory.json` binds
immutable inputs and `seal.json` binds report bytes. No candidate code, test,
build, SQL, DB, target/cache, app, provider or owner-data action was run.

## C1 correction and exact Git-only composition

This correction supersedes only the stale prep bindings/order below the original
9fb handoff; the original signed objects and evidence remain immutable.
`composition.json` verifies the literal replay on isolated
`ov-940-cutover-barrier-contract`: approved base
`d43e8f99cc00d81ec5f2ed4e606483e9352fe3a3` -> e853 replay
`51fd5f82fe2233da43bab7da897b21e5604383a3` -> approved93eb replay
`9410dfaae6615bbd3cb1a56d7e21167d4a385fa8` -> reports9fb replay
`896fe58359191314192f9e68646d1e92dc1c0ed8`. All stable patch IDs and
all tracked path/mode/blob entries equal the prescribed base-plus-deltas union.

| Input | Exact pin / accepted disposition |
| --- | --- |
| Store baseline936 | `e853d9c4ee07fbc07ab8e6d8bc0ce7d1658c57ba`; original failures preserved |
| Corrected936 | `93eb86c4c25f2f569ddb7d45a7e9026be9a6a67e`; SAME S1/S2 CLOSED, scoped storage only |
| Approved935 map | `bdff40f0b2944173c2be5b5fa906dcd2af88f7bf`; historical schema64 map |
| Approved937 source/evidence | `e1203d8c6bbeb4bf7eaecb51a08fd0ab2c6884e2` / `f4020a91daeda03219dbcfeda2ff112e62a20764` |
| Approved938 queue | `4c645bb788ec320fb19704ed042507adeccfd911`; queue-only owned-group semantics |
| Required939 reason/history | `821194faedd23fec2b639667034b6a8ba979f827`; included in approved d43 ancestry |
| R-94 CLI disposition | `d43e8f99cc00d81ec5f2ed4e606483e9352fe3a3`; SAME unsafe Retry finding CLOSED by refusal before connect; durable CLI retry remains OPEN under941 |

Current external SAME936/939/C1 review bytes and immutable source bindings are
in source-inventory.json. Earlier936 review prefix remains historical evidence,
not a pending S1/S2 finding. This allocation permits Git replay and C1 reports
only. SAME C1-only source review must close before literal product-region and
resource allocation; no product work or execution approval follows from union.

## Existing APIs, not an implemented lifecycle

At e853 `engine_cutover.rs:61-83`, public serializable `StoppedWriter` and
`Quiescence` validate seven names/nonempty strings/resource-set equality. They
are synthetic data, not capabilities. `engine_quiesce(&Quiescence)` (:314),
`engine_commit(&Preflight,&Quiescence,&CommitPermit)` (:326), `engine_verify`
and `engine_release` (`verify.rs:58,61`) do not join actual processes.
`CommitPermit` has private fields and public minting only through
`PrivateRehearsal::commit_permit`; keep that backup/restore boundary intact.

Future exact implementation boundary: only the retained in-process Service
owner may assemble proof from its actual admission gate, retained task/child
handles, resource identities and opening incarnation. The production Store
freeze/commit/release boundary must require an opaque, non-deserializable,
non-cloneable, one-attempt proof issued after those joins; wire callers cannot
construct it. The existing Quiescence may remain evidence serialization, but
must not by itself authorize runtime transfer. This is a required change to
the named Store entrypoints and Service ownership fields, not an assertion that
an API such as `freeze_and_join` already exists. No new host process framework.

Issuance must bind selected root/DB identity, attempt, old incarnation, exact
writer inventory, protected resource set/generations and joined receipts. Stop
new admission under the owner gate before draining; each admitted operation
stays registered through its final external effect and durable disposition.
Join failure, dropped future, panic, absent handle or unresolved resource owner
prevents issuance. Unknown effects stay quarantined. A consumed/stale proof
cannot authorize a second attempt, reopen, new group or changed inventory.
Store checks phase/incarnation/resource identity inside the same Immediate
transaction as apply/claim/admit/freeze, including direct Store callers.

The old process may already hold writable filesystem descriptors. SQLite
fences and process exit text cannot revoke them. Signal only a directly owned
spawned group while its unreaped leader pins identity; then reap and require
ESRCH for group disappearance, following938's queue ordering. EPERM, any other
probe error, leader exit alone, timeout and generic Unknown never prove clean.
Never signal stored DB PID/PGID values or infer authority from stale inventory.
External/escaped/unowned writers block/quarantine; no invented zero-effects
receipt. After a crash, DB phase/incarnation and the nonreplaceable journal are
the recovery oracle, not a reusable runtime shutdown certificate.

## Bootstrap call order and recovery reachability

Use937's held `paths::selection() -> &'static RuntimeSelection` and `root()`.
Do not re-read environment or call ensure/socket/Store merely to learn a path.
Normal default startup behavior/authentication remains preserved; cutover is
an explicit admin path, never automatic conversion on ordinary startup.

1. Select/validate the root. Shared Store/Service admission must read existing
   schema/authority without migration, compatibility stamps or wait sweeping.
   Missing authority table at a trusted older schema is not automatically
   released/legacy authority; unknown schema/errors fail closed to recovery.
   New empty installs need an explicit fresh-install branch, not error fallback.
2. Normal main:937 `main.rs:37,47,169,206,220,230,249` selects, logs, resolves
   socket, acquires lock, probes sshd, opens Service, starts workflows. An early
   read/refusal before kept_log is the conservative cutover insertion; recheck
   under the exclusive lock before offer_passkey_keys/Service. Pure inspection
   before lock is not the transfer linearization point. Path/log creation is
   not quiescence. No workflow/check-in/hook/watch owners start while frozen.
3. All `Service::open`, `open_selected`, and production `open_in` entrypoints
   must share the pre-normalization gate. `open_selected` currently ensures root
   then `open_in`; `open_in` creates install ID, calls Store::open, refreshes
   inventory and loads registry/Sessions. Checking only after it is too late.
4. Stream937 `main.rs:483/488` calls Service directly. Standalone stdio
   `:612/662/680` opens Service and starts Watcher without the normal lock.
   Apply the shared gate there; relay-only stdio forwards to the already gated
   daemon and creates no independent Store owner. Preserve refusal over stdio
   without stdout logging. Stream must not bootstrap legacy progression.
5. Direct `Store::open/init` (`store.rs:57-123`) currently migrates, stamps and
   sweeps. All production opens must refuse/route raw60 and frozen/committed
   recovery before those operations. `open_for_cutover` must be the explicit
   authorized existing-file path, never a fallback to ordinary open.
6. Explicit transfer follows the C1 durable protocol below: close admission,
   actually join old owners, hold resource exclusion, capture and durably bind
   the original raw source before any upgrade write, then atomically commit the
   upgrade/version/refusing floor/frozen attempt. The existing later closed
   quiesced backup + independent restore + external journal + CommitPermit,
   revalidation, atomic mapping commit, verify and bounded release still apply.
   The early baseline cannot substitute for that later backup/restore proof.
7. Reopen frozen/quiesced/committed state through admin recovery without ordinary
   watcher/scheduler dispatch. Precommit remains frozen; committed uses forward
   recovery. Restored DB gets a fresh incarnation and post-backup external facts
   before any admission; pending restoration never authorizes replay. Keep
   authenticated inspect/recovery reachable when ordinary Service refuses;
   implement the minimal existing admin cutover entry seam, not a new transport.

## C1 durable pre-upgrade barrier and crash protocol

This is a required future ordering change, not an existing API or reserved
schema/tag. Source at corrected93eb engine_cutover.rs:412-419 still preflights,
then calls migrate, then stamps the floor separately. migrate.rs:129-146 commits
all remaining DDL and schema_version together; engine_cutover::migration seeds
legacy/incarnation0. Thus a real observable gap exposes upgraded66/oldfloor60
and legacy authority. A pinned old60 Store reader can accept it through
compat::open_newer without knowing any new marker. There is no externally
committed per-migration partial catalog to recover in the current transaction.

The following names denote contract states/artifacts, not invented APIs:

1. **Admission closed / actual owners joined.** Candidate normal main (before
   logging/helpers), shared Service/open_in, stream, standalone stdio and direct
   Store check for an existing intent before any migrate/stamp/sweep or helper.
   Recheck under held root/DB/resource exclusion. Authenticated admin inspection
   and recovery use a non-normalizing seam. Close RPC/job/hook/manual admission,
   drain every actual owner in ownership.md without holding its needed Store
   mutex, retain owned group identities through final effects, and join. No
   missing handle, generic Unknown, leader exit or stored PID proves this.
   Unowned/open writable descriptors and uncertain owners block/quarantine;
   no marker, floor or SQLite fence revokes them. Hold directly owned resource
   exclusion through capture, upgrade and later revalidation. After a crash,
   reacquire exclusion and freshly prove owners; serialized proof never revives.
2. **Original raw baseline durable.** With writers closed/joined and resources
   stable, capture the trusted registry60 typed catalog/ledger before ordinary
   Store normalization. Produce a recoverable closed original DB/resource backup
   plus typed raw manifest, not just hashes of data that cannot be restored.
   Validate finite artifact kinds/identities under the selected private journal
   discipline; reject residual WAL/SHM/journal rather than silently copying an
   incomplete DB. Bind selected root, actual DB/resource identity/generations,
   attempt, exact source60 schema/floor/catalog/typed bytes/rowids/allocator and
   actual joined-owner identity receipts. Sync complete backup/manifest files,
   their publication and containing directories before publishing transfer
   intent. Publication is exclusive/nonreplaceable, never overwrite or recapture
   the source. No registry, stamp, mapping or normalization write is allowed yet.
3. **Intent durable before first upgrade write.** Publish an exclusive attempt
   intent referencing that verified baseline and binding the same DB/resources,
   source hash/catalog, owner receipts and root. Sync file and parent boundary
   before proceeding. All candidate gates give it precedence even with absent
   authority table or legacy phase. Missing/truncated/changed referenced artifact,
   invalid intent or ambiguous partial publication refuses without mutation.
   Reserved incomplete artifacts also block automatic ordinary startup; absence
   of a complete marker is never sufficient to classify an existing DB as fresh.
   No automatic cleanup/replacement or ordinary-Store fallback. Admin re-entry
   validates the original baseline; it never recaptures upgraded rows as raw60.
4. **Atomic upgrade and freeze.** Under revalidated held ownership/exclusion,
   the explicit cutover entry performs all remaining registry DDL, schema_version,
   the registry-derived refusing compatible_down_to (66 in this pinned union,
   greater than60), and bound frozen authority/attempt/source truth in ONE
   transaction. Initial legacy/incarnation0 from migration must not be externally
   committed for this transfer entry. No post-migrate stamp or later quiesce is
   allowed to supply the missing barrier. Keep the external intent unresolved.
   Crash inside this transaction rolls back catalog/version/floor/frozen truth
   together. An old60 open immediately after commit must refuse from the floor
   independently of the candidate intent guard. Before commit source60 is still
   unchanged, protected by the actual old-owner admission/resource barrier.
5. **Quiesced mapping and later transfer.** Retain the same attempt and original
   raw ledger through quiesced freeze, close, closed quiesced backup, independent
   restore validation, external CommitPermit journal and exact revalidation.
   Atomic engine commit retains required939 state/events/receipts and resource
   fences; verify before explicit resource release. Append/sync terminal journal
   disposition only after DB truth confirms it, never clear the intent merely
   because registry upgrade succeeded. Recovery always reconciles DB phase,
   incarnation, attempt/source binding and journal; mismatches refuse/quarantine.
   Precommit recovery remains frozen. Committed state uses forward recovery;
   restoring the original source is an explicit authenticated closed-resource
   operation with new incarnation and external-effect reconciliation, never an
   automatic rollback that authorizes replay. Completion cannot silently reopen
   legacy progression or bypass retained capability release blockers.

| Durable boundary / interruption | Required reopen oracle |
| --- | --- |
| Before baseline publication; no DB mutation | Source60 unchanged. Incomplete reserved artifacts refuse; admin resolves with fresh join proof. No automatic transfer or fresh-install inference. |
| Backup/manifest durable, intent not yet durable | Original source remains unchanged; orphan/partial attempt blocks ordinary startup. Admin validates existing baseline and exclusive attempt, never replaces it. |
| Intent durable, before upgrade transaction | Every candidate seam refuses ordinary admission; admin validates original raw60 baseline and reacquires owner/resource proof. Old readers require actual held exclusion. |
| During upgrade transaction | Rollback leaves complete source60 catalog/version/floor; durable intent still refuses candidate startup. No invented partially committed DDL state. |
| Immediately after upgrade commit, before former separate stamp/quiesce | Upgraded catalog, refusing floor and frozen attempt are atomic. Old60 reader refuses; candidate intent/freeze gate refuses. Admin still uses the ORIGINAL raw60 ledger. |
| After quiesce; before/after later backup/restore/permit publication | Frozen DB plus exact journals; missing permit/backup never authorizes commit. No proof reuse after reopen. |
| During mapping commit / after commit before journal terminal record | DB transaction rolls back wholly or commits wholly. DB attempt/incarnation and external journal distinguish frozen from forward recovery; stale outer text cannot reverse committed truth. |
| After verify; before/after release or final intent disposition | Only verified resolved resources may admit new engine/manual work. Lost/truncated marker, permissive/missing floor, absent/mismatched frozen attempt or baseline refuses/quarantines, even if phase text says legacy/released. |

A missing/low floor on an upgraded DB is inconsistent state, not permission to
stamp it during ordinary open. A missing intent with frozen/committed attempt
likewise routes to admin. A surviving intent with absent/legacy authority takes
precedence. Admin may repair only from validated baseline+journal+DB truth under
fresh ownership proof; unrecoverable missing artifacts remain blocked. This
covers partial marker/floor combinations without relying on a new old-binary
protocol. The pinned old reader test must run the actual immutable schema60
Store entry, not a simulated opens_here comparison.

**Fresh install is explicit.** Only proven absent DB and no transfer artifacts,
under the selected root's exclusive admission boundary, may enter a new-install
branch. An empty/unknown existing file, schema0, missing authority/floor, errors
or orphan artifacts are not that proof. New-install initialization commits its
current catalog/version/refusing floor and the existing migration-defined
legacy/incarnation0 together, explicitly identifying a new empty installation.
That branch supplies no transferred/released engine authority or raw60 history;
engine release still needs its explicit verified admission path. It cannot be
the upgrade fallback. Unknown/conflicting state stays admin-reachable.

Private future fixtures additionally require literal absolute nonempty private
CLAUDE_CONFIG_DIR, FARCOOLER_CONFIG and TMUX_TMPDIR as well as FARCOOLER_HOME.
937 does not make FARCOOLER_HOME a sandbox. No identities/members/terminals or
provider/main launches are authorized by this preparation.

## Retained communication and cross-slice conservation

Smallest retained-path bridge: preserve the existing authenticated manual
message RPC -> note + explicit fresh delivery -> existing targeted delivery
adapter. Service owns authorization/current target/incarnation and the retained
delivery task; Store messages/wakes owns atomic note/delivery and final CAS;
Watcher answer_wake owns only that bounded manual delivery, without starting
the answer/hold/worker/collecting/check-in scheduler. Do not blanket un-fence
message_wakes, re-enable pump_wakes, or infer manual authority from actor text,
extra JSON, enqueue time or a user-supplied flag.

e853 messages.rs:54-90 writes a note and message_wakes in one transaction;
engine_cutover.rs:112-119 aborts its INSERT after transfer. send_runner_notice
uses the same queue. A future Store transaction therefore needs a narrowly
issued server-only manual admission and an exact fresh-delivery binding that
the retained consumer can prove. It must retain the original guard for every
unadmitted legacy write. Whether this uses an existing receipt identity or a
minimal new durable binding must be settled from the corrected composed source
before code; no schema/tag reservation is made here. If no such durable binding
can be implemented in the allocated regions, communication remains an explicit
release blocker; a direct paste-only shortcut is not equivalent durability.

Retain every old pending/claimed/pasted/done queue row and all notes/history
verbatim; none becomes fresh manual authority on release/restart. Claimed or
pasted uncertainty must not retype or press Enter on recovery. Fresh manual
delivery is deduplicated by its exact identity, current target and incarnation;
loss of a receipt is Unknown, not a retry permission. Explicit manual send is
not approval or reducer progress. Existing compose text/image/draft/native
conversation routes require the same bounded retained delivery bridge; any
unsupported route is declared blocked, never advertised as preserved.

Disable/join automatic producers `watch/agent_start.rs:208-219` (ended/not-started
runner notice), `watch/collecting_scan.rs:57-77` (departure notices), worker
follow/status/hold/answer progression and legacy scheduler. Do not scan retained
history into new workflow notices. Native informational notices keep their own
existing Store outbox/attempt/target proof and approved938 cleanup, with new
freeze/incarnation checks at both preparation and final synchronous spawn.
Prepared/unknown/handed_off history stays exact; no startup backlog replay.
Concrete counterexample:938 `workflow_notices::run` starts every drain with
`after=0` and queries pending notices. Reusing it unchanged after release would
rediscover retained pending history. Its production consumer needs an exact
durable cutover eligibility binding/floor tied to retained notice identities
and the transferred incarnation, checked again at preparation/admission.
Do not use wall-clock timestamps, an in-memory cursor or changing old payloads
as that proof. Preserve old pending rows as inert history; newly admitted
informational notices remain eligible once, with their original state machine.

Keep terminal screen/history/stream and activity/conversation observation routes
(`Service::screen/history/stream`, Watcher activity/feed/said/log observers).
Watcher::run is currently mixed observation and mutation; retaining activity
requires disabling/joining progression/pumps while preserving those observations,
not restarting the entire legacy loop. Native/Web control linkage and real
conversation usability remain932/parent gates, with gaps reported honestly.

939's compatible Review optional reason and Item.rework_history are required
data: exact reason bytes, principal/revision/run/evidence/policy/iteration/ID,
events and receipts survive commit/restore/reopen. Its default-empty history
preserves old encoding; never synthesize historical reasons. Reconcile936's
lossless state/outbox checks against the reviewed939 type graph; otherwise valid
reason history might be quarantined as unknown or erased. R-94's d43 refusal
is accepted; durable authenticated CLI retry stays OPEN under941 and is not a
bootstrap or mapper workaround.
