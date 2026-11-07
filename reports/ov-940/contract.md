# ov-940 — source-preparation handoff

Phase 1 only. No product implementation or execution approval. The signed Git
commit containing this package is the handoff pin; `source-inventory.json` binds
immutable inputs and `seal.json` binds report bytes. No candidate code, test,
build, SQL, DB, target/cache, app, provider or owner-data action was run.

## Composition inputs and hard hold

| Input | Exact pin / disposition |
| --- | --- |
| This isolated branch/base | `ov-940-cutover-integration-prep`, `e853d9c4ee07fbc07ab8e6d8bc0ce7d1658c57ba` |
| Store candidate936 | e853 above; SAME review REQUEST CHANGES: S1 unsafe JSON/journal reads, S2 permissive executable outbox JSON. Preserve its source and failures. Fresh936 fixer owns correction. |
| Approved935 map | `bdff40f0b2944173c2be5b5fa906dcd2af88f7bf`; report-only PASS, schema64 historical map, not current schema evidence |
| Approved937 source | `e1203d8c6bbeb4bf7eaecb51a08fd0ab2c6884e2`; focused execution closure at reports-only child `f4020a91daeda03219dbcfeda2ff112e62a20764` |
| Approved938 queue ownership | `4c645bb788ec320fb19704ed042507adeccfd911`; SAME933 P2 closed, bounded queue ownership only |
| Required939 reason state | `821194faedd23fec2b639667034b6a8ba979f827`; required shared reason/history source, CLI destination fix pending; no adoption as fully approved composition |

Read all four exact SAME936/935/937/938 reports at
`/Users/e-liang/Dev/overnight/.claude/agent/reports/ov-NNN/independent-review.md`.
Their byte hashes are in the inventory. They are external source-only inputs,
not files assumed present in this checkout. No reviewer verdict is widened.

**No final corrected936 pin or union exists in this allocation.** Before product
work, the orchestrator must supply the SAME-approved corrected936 immutable pin,
corrected939 destination disposition/pin, explicit region ownership and an exact
composed base. Record parent graph, registry/catalog hashes, resolved overlap
blobs and review hashes in a new signed binding before execution. Never silently
replace this e853 base, dirty-cherry-pick siblings or count union-by-prose as
composition. Source preparation can finish while those gates remain open.

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
6. For explicit transfer: close new RPC/job/hook/delivery admission; drain/join
   the inventory in ownership.md; retain active-effect truth; capture raw closed
   source; perform finite upgrade mapping; enter quiesced freeze with bound
   proof; close Store/resources; closed backup + independent restore + external
   journal; reacquire/revalidate exact DB and resources; mint CommitPermit;
   atomic commit; verify; release only resolved resources. Required source60
   migration protocol must define durable interruption between raw capture and
   Store upgrade; no ordinary-open sweep in that interval.
7. Reopen frozen/quiesced/committed state through admin recovery without ordinary
   watcher/scheduler dispatch. Precommit remains frozen; committed uses forward
   recovery. Restored DB gets a fresh incarnation and post-backup external facts
   before any admission; pending restoration never authorizes replay. Keep
   authenticated inspect/recovery reachable when ordinary Service refuses;
   implement the minimal existing admin cutover entry seam, not a new transport.

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
reason history might be quarantined as unknown or erased. CLI retry destination
binding remains939's fresh fixer, not a bootstrap or mapper workaround.
