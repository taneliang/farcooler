# ov-933 implementation and evidence

Base: `77250986dcb4b1dbb6b63e06d0b11dc1b92d88a3`, on the dedicated
`ov-933-workflow-notice-implementation` branch. This implements the ordinary
Service notice producer and bounded native delivery path. Original contract-only
security PASS is historical; the frozen implementation requires the SAME
independent security reviewer before acceptance/composition is closed.

## Delivered behavior

Accepted workflow transitions select finite structured key points inside
`Store::workflow_apply`/`apply_in`, in the same transaction as event, state,
effects and command receipt. `workflow_notices::select` uses old/new typed state,
never reducer prose. Immutable selected records retain event/workspace/epoch,
item/run/revision/stage/iteration/attempt/candidate, definition, execution bundle,
notification policy and point identity. Equal rendered item:phase events have
different deterministic IDs. Command replay returns its existing receipt and
cannot insert another selected notice. A later transaction failure rolls all
selected rows back. The appended registry entry creates no historical sends.

`notifications` is optional on the immutable publication bundle and omitted for
None. Omission preserves old command and receipt bytes and old bundle pins.
An explicit policy uses the new `workflow-bundle/v2` pin domain; old immutable
bundle rows are untouched and optional policy bytes live in an immutable
append-only table. Existing items select their own admitted bundle policy, never
the workspace's latest configuration. The permanent v1 omission default selects
WaitingApproval, WaitingReview, Uncertain, Done, Failed and Cancelled. Blocked and
StageChanged are optional finite points. No prose filters, scripts, adapter grant
or new engine approval is introduced. The real CLI file producer accepts this
bundle shape, including a tracker policy without a coding adapter, and rejects
unknown policy points/authority fields.

Ordinary `Service::start_workflows` retains one independent delivery task beside
the existing sole engine task. Their common shutdown signal is sent before both
are joined, including when the engine failed. Committed engine inputs/wire
mutations and actual target/session/role mutations signal a dedicated wake that
carries no payload or authority and never reenters Store. The sender subscribes
to native inventory health/identity changes before initial recovery/draining.
Its subscription drops with that owned task. Inventory publication releases all
locks before waking; comparison explicitly includes pane PID/TTY/tag/life and
foreground-command hint, excludes output/title/geometry and normalizes ordering.
The sender never refreshes inventory and adds no idle timer or history polling.

The indexed pending queue is read in32-row pages with cooperative yielding.
Unavailable/unsupported records do not prevent later independent rows being
considered. One prepared native attempt per notice is durably recorded before
any child exists. Prepared means possibly dispatched, not provider acceptance.
Recovery changes any uncompleted prepared attempt to visible Unknown and never
relaunches it. Exact lookup by notice ID and bounded recent delivery inspection
are Store APIs. Get preserves id/revision/body presentation and adds bounded
structured notice_deliveries instead of a revision-zero history scan.

## Native admission and receipt limits

The first concrete endpoint is the existing owner's Codex queue route. Member
logins/leads, unknown sessions, competing targets and unsupported endpoints stay
pending with an explicit unsupported/unproven-target reason. No native task UUID
is fabricated and no legacy message coalescing or wake fallback is called.

Discovery proves a unique current private-server pane, controlling TTY, actual
foreground Codex process/incarnation, unique foreground-held main rollout and
thread/home. Existing ps/lsof processes are scoped to a retained discovery owner,
with bounded output/time and cancellation/reaping. The reused metadata head read
now opens nonblocking/nofollow and validates the opened regular/single-link file;
existing64-line/1MiB bounds remain. FIFO, device and final-symlink substitutions
cannot turn that read into a blocking receipt poll.

The server-created NativeTarget has no wire constructor/deserializer. A final
Immediate transaction rechecks current workspace/role, terminal resource version,
epoch/lease/session, owner lead/login absence, competing targets and existing
stop/remove reservations. Monotonic target generations additionally reject ABA
across all INSERT/UPDATE/DELETE writers of terminals, workspace_leads,
check_in_target_reservations and terminal_logins. The transaction serializes
other Store connections too. All discovery awaits precede the final synchronous
kernel/TTY/inventory recheck and spawn. Exact terminal/row, pane/TTY/device,
PID/incarnation, thread/home/program facts are retained with the attempt, not
merely a digest. No caller-supplied terminal ID or policy grants native authority.

Admission linearizes at native spawn; later replacement cannot recall already
admitted bytes. A proven no-child refusal may remain pending for a later real
wake. After spawn, nonzero exit, cancellation, timeout or lost settlement reply
is Unknown with the original attempt/target. There is no blind retry, rebinding,
typing fallback, text matching or transcript watcher. Exit0 records **local
handoff only**. No provider-consumption, model-response or exactly-once guarantee
is claimed. This endpoint exposes no concrete receiver dedup/reconciliation
receipt, so unreconciled Unknown remains Unknown; a resend override is outside
this slice.

Bounds:32 records/page,16KiB message,8KiB target descriptor,100-byte attempt ID,
1KiB outcome evidence,15s discovery and5s queue execution. The native child owns
its process group; timeout/shutdown kills and reaps it before outcome settlement.
Discovery reaping failure is reported instead of claimed as successful cleanup.
Informational state is never effect eligibility, lease, source sequence or a
Decision grant. Intentional approval still requires the existing authenticated
exact observed revision/run/pin/candidate path.

## Private evidence

`affected-sealed.log/json` and `lint-sealed.log/json` are the restored final gates.
They retain actual commands, all tracked crate/fixture/Cargo input hashes,
unchanged-source checks, warm-target provenance/empty-holder probes, minimum free
space and exit/time evidence. All checks use the normal exclusive FIFO, original
ov-923 target, native Python, jobs6/profile000/offline, load5<20,10GiB reserve and
owned1800s supervisors. The final ledger includes tmux root/dep-info binding.

The final affected log saves actual native queue argv for the same item/phase
rendered twice across different committed events, stale epoch and lead ABA
refusal/restoration, duplicate identity, nonzero/lost reply, owned cancellation
and timeout, offline backlog and reconnect delivery. Recovery-only evidence uses
a fresh unavailable inventory against the SAME private native server: a real
refresh wakes delivery with the entire terminal row unchanged. Repeated unchanged
refreshes preserve generation and cause neither another sender pass nor another
queue invocation. These are local stand-ins, not real-provider consumption proof.

Store regressions cover rollback after notice insertion, receipt replay,
35-record pagination, exact attempt/target outcome fencing, restart Unknown,
immutable policy retention across reconfiguration and zero historical backfill.
Normal Service/socket tests exercise dispatch/check/reviewer/rework/landing/CI
with the orchestrator offline and all informational rows pending/no attempt;
existing authenticated exact-decision refusal tests remain in the affected suite.

Nine deliberately removed guards fail at their intended assertions and restore
exact bytes: final-admission, final-aba, final-atomic, final-target-outcome,
final-metadata, final-ack, freeze-inventory, freeze-inventory-silence and freeze-owner-closed.
`mutation-*.json`, exact original/mutant text snapshots in negative-sources/ and
the corresponding negative logs bind each case. Every selected guard file's
original hash matches its final frozen product bytes. The first six graph
snapshots precede the separately allocated inventory extension; their complete
input maps record that delta honestly. Final restored suites exercise all paths
with that extension present. No compile failure is counted as a guard failure.

Early compile/fixture errors and the first size/lint failures remain preserved
rather than counted as red guards: missing test dependency declaration, duplicate
publication field/private test module placement, incorrect tracker test phase,
invalid task-bound shell fixture, tuple complexity and a test registration that
crossed an existing1500-line file budget. The corrected tracker test actually
claims/admits/observes the effect while informational state is Unknown. The test
registration lives under the existing delivery test module; no fmt or budget
manifest increase was used. Final producer tests also cover optional policy file
roundtrip and unknown field/point rejection.

## Scope and handoff

Task decisions0047e363,47bbe6b3/9392b59f,940228bf and039254bf/c2486954 record
API alignment and the narrow Store, shared metadata-read and native inventory
allocations. The final full file list, source/evidence hashes, suite counts and
mutation facts are in freeze.json. No app/build-script paths, security/owner
configuration, live app/database/backlog, owner terminal, real provider or
external send was touched. Git transport in tests uses private local bare repos.
Pre-existing untracked .codex/.cursor metadata is excluded from the commit.

Freeze for SAME independent security/data-safety review; do not infer that the
old contract-only PASS approves this implementation, or that builder evidence
alone closes acceptance/composition or authorizes activation/landing.


## Closed-owner admission correction

The final ownership audit additionally found that a watch channel can close while
its last bool remains false. The old producer could prepare/spawn at that boundary
and only cancel afterward. `owner-closed-red.log/json` demonstrates this actual
old-source failure with a private proven endpoint. Its exact old producer bytes
are retained in negative-sources/owner-closed.old.txt. The corrected sender treats
closed OR true as shutdown before preparation, again at synchronous native
admission, during draining and in owned child/discovery cancellation. The final
closed-owner test requires pending/no attempt and zero queue bytes. This is an
owned-lifetime fence, not an informational ACK or a new engine gate. Final restored
suites and lint include this correction; earlier affected-final gates are retained
as preceding evidence rather than mislabeled as the corrected source.

## Actual target-writer ordering

This fence does not rely on the seat lock alone. Restart's
Service::relaunch_held calls Store::reseat_orchestrator before external respawn;
that transaction sets runtime_confirmed=0 and increments resource_version before
the pane changes. Native notice admission requires confirmed=1 and the captured
version/generation, so restart either follows an already admitted spawn or makes
its old proof refuse before bytes. Existing stop/remove paths persist their
check_in_target_reservations before external operations; final admission checks
those reservations. Role/workspace/session/lead/login and deletion/recreation
writers are covered by the same SQLite writer boundary and generation triggers.
Later mutations cannot recall an invocation already admitted at spawn.

Final main-suite passes: inventory3, Store25, protocol1, CLI2, daemon workflow69,
shared log_join25, owned discovery3, ordinary Service socket10 and legacy socket2:
140 total. The two pre-existing ignored log-join smoke tests remain ignored; the
ordinary process helper is ignored as a direct test but is explicitly invoked by
its parent tests. Nested child summaries are retained separately and are not
inflated into the main-suite count. Final lint and size budget pass. The last
mechanical lint correction removes a needless reference in the owned cancellation
check; affected-sealed/lint-sealed bind the exact corrected bytes.

Raw logs retain Rust test-progress trailing spaces exactly as emitted. The source
whitespace check is scoped to the explicitly listed product files; generated
evidence is not reformatted to satisfy a source whitespace convention.
