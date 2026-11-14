# One pipeline that tickets flow through

Design spike for ov-1223. No product code. The owner, Nov 15:

> The workflow engine should be a pipeline that manages work. Tickets should
> flow through it. It seems like what you've implemented is a per-ticket
> workflow instead? That seems inverted.

And: the engine exists to make the orchestrator more deterministic, but the
orchestrator keeps full control and can step in anywhere.

Short answer: turn the engine inside out. Today each ticket carries its own
copy of the process and its own counters, and the only shared things are the
runtime's places. In the new model the **pipeline** owns the process: a fixed
set of **stations**, each with a queue, an order, a capacity and a policy.
A **ticket** is a token that moves between stations. It carries only what is
its own (its name, branch, candidate and history) and keeps one identity for
life. Limits stop being reasons a ticket dies. They become station policy that
holds a ticket for the orchestrator, who can override any of them, and every
override is written in the ticket's history.

What does not change: the reducer stays pure, every input is logged and
replayable, every effect is fenced, crash recovery reads receipts, and only a
person publishes a process.

All file and line references are to `main` at 73974f898.

## 1. What is inverted today

`State` (`crates/workflow/src/state.rs:135`) is a map of `Item`s. Every item
holds:

- its own copy of a `Definition` and its pin (`state.rs:22-23`), checked on
  every step (`reducer.rs:796`) and against its bundle before every launch
  (`service_owner.rs:124-139`);
- its own position: `stage`, `phase`, `iteration`, `attempt`, `fence`
  (`state.rs:27-31`);
- its own budgets: `back_edges`, `retries`, `sent_back`, `ci_reruns`
  (`state.rs:41,44,78,99`), each spent against limits in its pinned copy.

A landing group is not a place tickets go. It is one more `Item`, born from a
group definition when the first ticket is queued (`reducer.rs:302-319`) and
born again for each half of a split (`reducer.rs:435-449`).

Nothing in `State` represents a station, its queue, its order or its capacity.
The only shared thing is outside the reducer: the runtime's places
(`crates/daemon/src/workflow_runtime/places.rs:5-12`), three pools with fixed
caps (4 agent, 2 landing, 2 work) that the runtime fills in outbox order,
`ORDER BY revision, id` (`crates/store/src/workflow.rs:162`), as it walks
workspaces in name order (`workflow_runtime.rs:580-632`). What waits for a
place is a list the runtime rebuilds on every scan (`places.rs:46`). It is not
in the log, can't be reordered, and the reducer never sees it.

Control follows the same shape. Every wire action names `item`, `run` and
`pin` (`crates/protocol/src/workflow_commands.rs:19-59`). To act, the
orchestrator must know which run and pin the ticket is on, and which phases the
reducer accepts each command in.

## 2. The pipeline model

### 2.1 Terms

- **Pipeline**: one per workspace. Its process (the stations and the routes
  between them) is one published definition, and it holds every ticket on the
  board.
- **Station**: a place where one kind of work is done: provision, build,
  review, route, approve, fold, check, group review, gate, push, CI, and the
  pull request steps. A station has a **queue**, an **order**, a **capacity**
  and a **policy**.
- **Ticket**: a token. One per board task, from the moment it is tracked until
  it is closed, and again if it is reopened. It never changes its key.
- **Lot**: tickets that travel the landing stations together (today's landing
  group). A lot is a carrier, not a ticket: its tickets keep their identity,
  their place in the lot is recorded on each of them, and a lot can shrink
  without becoming a new lot.
- **Visit**: one stay of a ticket (or lot) at a station: when it arrived, when
  work started, how it ended, who moved it on.
- **Hold**: a ticket (or a whole station) stopped where it is, with a reason,
  until the orchestrator or a person releases it. A hold is the pipeline's
  only way of saying "needs you". There is no terminal `Failed`.

### 2.2 What the pipeline owns and what a ticket carries

| The pipeline owns | A ticket carries |
|---|---|
| The published process: stations, routes, policies, model lists, landing target (one definition pin) | Its key (the board task's), forever |
| Each station's queue, in order | Its branch and worktree (`RunTree`), and followers riding with it |
| Each station's capacity and what holds it | Its candidate commit and last evidence |
| Each station's held/draining flag | Where it is (one `Location`, below) |
| Lots: their tickets, base, candidate, push | Its priority |
| Counters of the host-wide budget it was stamped | Its brief (why it was last sent back) and last failure |
| The log of every input | Its visits: the per-station history with every override and reason |
| | Its picked models, its dependencies |

A ticket carries no definition, no stage index, no limits and no budgets. Its
counters ("review sent this back twice") are derived from its visits, so they
can never disagree with its history.

### 2.3 Shapes

Sketch, not final code. Names follow the reducer's existing style.

```rust
pub struct Pipeline {
    pub schema: u32,                     // 3
    pub workspace: String,
    pub revision: u64,
    pub epoch: u64,
    pub pin: String,                     // the published process this pipeline runs now
    pub stations: BTreeMap<String, Station>,
    pub tickets: BTreeMap<String, Ticket>,
    pub lots: BTreeMap<String, Lot>,
    pub seen: BTreeSet<String>,
    pub source_sequences: BTreeMap<String, u64>,
    pub archive: BTreeMap<String, Archived>,   // closed tickets and landed lots, as today
}
pub struct Station {
    pub queue: Vec<String>,              // ticket or lot ids, in the order they will start
    pub working: BTreeSet<String>,       // what holds its capacity now
    pub capacity: u16,                   // the published default until the orchestrator sets it
    pub held: Option<Hold>,              // a held station starts nothing new
    pub draining: bool,                  // finishes what it holds, takes nothing new in
}
pub struct Ticket {
    pub at: Location,
    pub priority: i32,
    pub tree: Option<RunTree>,
    pub candidate: Option<String>,
    pub last_evidence: Option<Evidence>,
    pub brief: Option<String>,
    pub failure: Option<String>,
    pub models: BTreeMap<String, String>,
    pub dependencies: BTreeSet<String>,
    pub followers: Vec<String>,
    pub visits: Vec<Visit>,              // bounded; the log has all of it
    pub fence: u64,
}
pub enum Location {
    Resting { column: String },                         // tracked, no run (today's ticket template)
    Queued { station: String },                         // in that station's queue
    Working { station: String, work: Work },            // holds a place; an effect may be out
    Stopping { station: String, work: Work, then: Box<Location> }, // still holds the place and the tree (3.4)
    Awaiting { station: String, decider: String, run: String }, // an approval or a person's/orchestrator's review
    InLot { lot: String },                              // the lot's location is where it is
    Riding { lead: String },                            // a follower (ov-970)
    Held { station: String, hold: Hold },               // stopped, needs someone
    Closed { landed: bool },                            // done or cancelled; moves to the archive
}
pub struct Work { pub run: String, pub attempt: u8, pub pending: Option<String>,
    pub stopping: Option<String>, pub uncertain: bool, pub wait_since: Option<i64>,
    pub ci_retry: Option<CiRetry> }
pub struct Hold { pub reason: String, pub by: String, pub revision: u64, pub kind: HoldKind }
pub enum HoldKind { Asked, PolicyLimit { policy: String }, StepFailed, Uncertain, Quarantined }
pub struct Visit { pub station: String, pub arrived: u64, pub started: Option<u64>, pub left: Option<u64>,
    pub ended: Option<String>,           // the label it left with: passed, rejected, conflict, skipped, pulled…
    pub by: String,                      // policy, or the principal that moved it
    pub reason: Option<String>, pub overrode: Option<String> }
pub struct Lot { pub at: Location, pub tickets: Vec<Member>, pub base: Option<String>,
    pub candidate: Option<String>, pub pushed: Option<String>, pub suspects: Vec<String>,
    pub visits: Vec<Visit>, pub fence: u64 }
```

`Visit` gives every ticket and lot one timeline under one key. Revisions map
to times through the log (`Input::now_ms`), so per-station timing is a read of
`visits`, not a reconstruction across items.

### 2.4 Stations

The published definition lists stations in order, with routes, like today's
schema-2 steps (`definition.rs:60-73`), but the list belongs to the pipeline,
not to each ticket. The four landing templates (`templates.rs:109-192`) become
four station layouts. The batched-push layout, with the group review the owner
asked for on Nov 15 (board note on this ticket):

```
backlog ─▶ provision ─▶ build ─▶ review ─▶ [route ─▶ approve] ─▶ landing queue
                          ▲          │ rejected (policy: back to build)
                          └──────────┘
landing queue ─▶ base ─▶ fold ─▶ cheap checks ─▶ group review ─▶ gate ─▶ push ─▶ CI ─▶ done
                          │ conflict     │ red          │ findings       │ red
                          └──────────────┴──────────────┴────────────────┴─▶ bisect or return to build (lot policy)
```

- Ticket stations (provision, build, review, route, approve, the pull request
  steps) take one ticket per place.
- Landing stations (base, fold, cheap checks, group review, gate, push, CI)
  take one lot per place. The landing queue is where tickets wait to be formed
  into a lot; it replaces `WaitingJoin` and the `Queue` command.
- **Cheap checks** and **group review** are new. Cheap checks run the fast
  commands on the folded tree (format, lint, the scripts' self-tests). Group
  review is one reviewer agent on the merged tree, hunting conflicts git merges
  cleanly: ratchets, `ci.yml` against `lane-gate.py`, fixtures, shared
  registries, API drift. Both run before the heavy gate, so a lot that would
  fail for those reasons fails in minutes, not after a full gate.

### 2.5 Queues, order and priority

Each station's queue is an explicit, ordered list in the reducer's state, so
it is in the log, replays, and is shown by `workflow get` as it is.

- A ticket arriving at a station is placed after every ticket of equal or
  higher priority already there. Equal priority is first come, first served,
  by the revision it arrived at.
- `priority` is the ticket's own. The orchestrator sets it (`Prioritize`), and
  it follows the ticket to every station. "Speed work first" is one command
  per ticket, not one per queue.
- `Reorder` puts a ticket at an exact position in one station's queue. The
  position holds until the ticket leaves that station; priority applies again
  at the next.
- A station starts work strictly from the head of its queue, skipping only
  tickets whose dependencies are not done. A held ticket is not in any queue.

### 2.6 Capacity

Capacity is a station's number of places, kept in the reducer state, so
starting work is a reducer decision, logged and replayable. The runtime no
longer decides what starts; it runs what the reducer emitted.

- Stations may share a pool. Build and review share the agent pool, as
  agent sessions do today (`places.rs:6`); the landing stations share the
  landing pool, which also takes the landing target's exclusive resource
  (`GroupDef::exclusive`, `definition.rs:91`). Each pool and each station has
  a capacity; a ticket starts when both have room.
- What other workspaces hold of a host-wide pool is stamped into each input
  by the store, as `busy_resources` is today (`state.rs:192-194`), so replay
  sees the same occupancy.
- The orchestrator changes a station's or pool's capacity with
  `SetCapacity`, up to the host's ceiling, which the runtime's configuration
  sets (today `set_effect_cap`, `workflow_runtime.rs:334`).
- Waits (base, CI, a pull request's merge) take no place, as today
  (`places.rs:25`).

### 2.7 Policies

A station's policy is everything that today is a ticket-level limit or a
hard-coded rule, written once per station in the published definition:

| Policy | Default (from today's numbers) | When it is reached |
|---|---|---|
| `attempts`: automatic re-runs of a failed effect | 2 (5 for waits), `templates.rs:92,96` | Hold `StepFailed` with the failure |
| `rework`: automatic send-backs from this station | review 2 (`templates.rs:129`), merge changes 2 | Hold `PolicyLimit` |
| `returns`: automatic returns to build from the landing stations, per ticket | 1 (`templates.rs:160`) | Hold `PolicyLimit` |
| `on_reject` (landing): `bisect`, `return_suspects`, or `hold` | `bisect` | see 2.8 |
| `lot_size` | 8 (`templates.rs:84`), none for "no cap" | the next lot forms behind |
| `patience` (waits) | base 3 h, CI 3 h, merge 14 d | label `timeout`, routed |
| `ci_rerun` | the ov-1113 rule (`ci.rs:22-25`) | as today |
| `order` | priority, then arrival | |
| `capacity` | per pool, see 2.6 | |

A policy limit never closes a ticket. The worst it does is hold the ticket at
the station with a reason that names the policy. The orchestrator then sends
it wherever it should go, with an override recorded in the visit
(`overrode: "review.rework"`), and the counter keeps counting. The count is
history, not a budget.

### 2.8 Lots, rejection and bisecting

Today a red gate splits the group: the first half goes back to its base wait
and the second half becomes a new group item from step 0 (`reducer.rs:731-741`,
`435-449`). Each half then waits for the base, folds, gates, pushes and waits
for CI on its own.

In the pipeline, a lot that comes back red stays one lot:

1. **Suspects**. The gate's receipt lists failing tests and files. Tickets
   whose changed paths meet them are marked as suspects on the lot. The
   orchestrator can pull a suspect at once (`Pull`, 3.1).
2. **Bisect** (the default policy). The lot keeps its id, and runs the gate
   on halves of its tickets, folded onto the same base, until it has found the
   tickets that fail. Halves only gate; nothing pushes while bisecting.
3. The tickets found failing leave the lot and go back to build with the
   gate's output as their brief (policy `returns`, then a hold).
4. The rest gate once more together, then push once and wait for CI once.

Take an 8-ticket lot with one bad ticket. Today it costs seven gates (8; the
two halves of 4; the two 2s under the bad half; the two 1s under the bad 2),
and the three good halves (4, 2 and 1) each wait for the base, push, and wait
for CI on their own: three pushes and three CI runs. Bisecting in place costs
five gates (8, 4, 2, 1, then the 7 that are left), one push and one CI run.
Cheap checks and group review before the gate catch many of these failures
in minutes, before any gate runs.

## 3. Control: the orchestrator can do anything at any station

Every intervention is one command that names the ticket (or station or lot)
and the station the orchestrator believes it is at. That is the only
staleness check: no run, pin or fence on the wire. If the ticket has moved,
the command is refused `Stale` and nothing changes; the orchestrator reads
again and decides again.

### 3.1 Interventions

| Intervention | Command | What happens | Recorded as |
|---|---|---|---|
| Put a ticket first, or anywhere | `Reorder { station, ticket, position }` | Moves it within that station's queue | The station's queue change in the log; the ticket's visit notes it |
| Change what goes first everywhere | `Prioritize { ticket, priority }` | Re-places it in its current queue; applies at every later station | Visit note |
| Hold a ticket | `Hold { ticket, reason }` | Queued: leaves the queue. Working: it goes to `Stopping` with `then: Held` (3.4); its place and tree are freed only by the stop's receipt. In a lot: as `Pull`. It keeps its tree and candidate | Visit ends `held`, reason |
| Release a ticket | `Release { ticket, to: Option<station> }` | Back into the queue of the station it was held at, or `to` | Visit, reason |
| Pull from a queue or a forming lot | `Pull { ticket, reason }` | Leaves the lot; holds at the landing queue | Ticket visit and lot visit |
| Pull from a running gate | `Pull { ticket, reason }` | The ticket leaves the lot and holds at once (the gate runs on the lot's tree, not the ticket's). The lot goes to `Stopping` with `then: Queued { land.fold }`; it keeps its id, its place, its tree and the landing target until the gate's stop receipt, and only then folds again without the ticket (3.4) | Both visits; the stop's receipt |
| Send back, any number of times | `Send { ticket, to, reason }` | Moves it to any station before where it is, after any landing it has made (see 3.3); its brief is `reason`; it queues there by priority. If it is working, it goes through `Stopping` first (3.4) | Visit with `by`, reason, and `overrode` when a policy count was past its limit |
| Run the step again | `Send { ticket, to: <its own station> }` | Today's `Retry`, without `MAX_RETRIES` | As above |
| Skip a step | `Skip { ticket, station, reason }` | The ticket leaves the station as `skipped` and goes on along its `passed` route, through `Stopping` if it is working (3.4) | Visit ends `skipped`, reason; see decision A for the gate in front of a push |
| Stop the running step | `Stop { ticket or lot, reason }` | `Stopping` with `then: Held` at the same station (3.4), nothing else lost | Visit ends `stopped` |
| Cancel a ticket | `Cancel { ticket, reason }` | `Stopping` with `then: Closed` if it is working, so it closes only on the stop's receipt, as today. Its key stays taken; `Reopen` brings it back under the same key | Archive entry with reason |
| Change capacity | `SetCapacity { station or pool, capacity, reason }` | Takes effect at the next start; nothing running is stopped | Station change in the log |
| Pause a station | `HoldStation { station, reason }` / `ResumeStation` | Starts nothing new; what runs finishes; its queue keeps its order | Station change |
| Drain a station | `Drain { station, reason }` | Finishes what it holds; arrivals queue in front of it but are not started; ends when empty | Station change |
| Start, track, move, reopen | `Start`, `Track`, `Move`, `Reopen` | As today, on the same ticket key | Visit |
| Approve, review, resolve, settle | `Approve`, `Review`, `Resolve`, `Settle` | As today, by their deciders | Visit |

### 3.2 No ticket-level limit refuses any of them

Each refusal below exists today only because a ticket carries a budget. All
of them go:

| Today's refusal | Where | In the pipeline |
|---|---|---|
| `Retry` past `MAX_RETRIES` (3) → `Limit` | `reducer.rs:333`, `state.rs:131` | No cap. `Send` to the same station |
| `SendBack` past `send_backs` (default 3, max 5) → `SendBackRefused` | `reducer.rs:412-415`, `definition.rs:506-507` | Policy `rework` counts and holds; `Send` overrides |
| `SendBack` past a step's back-edge limit → `SendBackRefused` | `reducer.rs:416-419` | Same |
| A review's rework past `max_reworks` or `limit` → `Limit` | `reducer.rs:114-115`, `700-705` | Same |
| A routed back edge past its limit → the run fails | `reducer.rs:757-758` | Hold `PolicyLimit`, not `Failed` |
| `SendBack` while a step runs → `Phase` | `reducer.rs:403-406` | Accepted: `Send` puts the ticket in `Stopping` and its new station waits for the stop's receipt (3.4) |
| `SendBack` that can't start the build at once → `SendBackRefused` | `reducer.rs:528-539` | It queues at build; the queue says why it waits |
| `Leave` from a group that has left → `Phase` | `reducer.rs:573` | `Pull` from any lot that has not pushed |
| `Cancel` of a follower → `Phase` | `reducer.rs:266` | `Cancel` on a follower ends its ride and closes it alone |
| `Start`/`Enqueue` while the resource is held → `Resource` | `reducer.rs:39-43`, `379-383` | It queues for the place |
| A fence at `u64::MAX` → `SendBackRefused` | `reducer.rs:420-422` | Stays an `Invalid` refusal; it is not a policy |

What can still refuse, and why each is not a ticket limit:

- `Unauthorized`: the grant does not match (`reducer.rs:590-596`).
- `Stale`, `Duplicate`: the ticket moved, or the input was seen.
- `Invalid`, `Missing`, `Evidence`, `Definition`: malformed input.
- `Landed`: the hard landing rules in 3.3.
- `Stopping`: a `Release`, `Resolve` or `Skip` that would start work on a
  tree or lot whose last effect has not proved it stopped (3.3 rule 4, 3.4).
  This is a safety rule about the tree, not a count; the command is accepted
  as soon as the stop's receipt is in.
- `PersonOnly`: publishing, settling a lost push, and (decision A) skipping
  the gate in front of a push.

### 3.3 Hard rules no override crosses

These are what keeps the engine safe, and stay rules, not policy:

1. Nothing goes back across a landing. A ticket whose candidate was pushed (or
   whose pull request merged) is not sent back to build; it is reopened under
   the same key, which builds on the new base (today's `Reopen`,
   `reducer.rs:56-80`). A lot that pushed can't be cancelled or pulled from
   (`reducer.rs:274-279`).
2. A push only sends a candidate a gate passed on the same base (today's
   route check, `definition.rs:341-358`), unless decision A says otherwise.
3. Every effect's answer is bound to its run and fence (`reducer.rs:126-142`).
4. No new work starts on a tree, a lot or the landing target until the last
   effect that used it has a stopped or settled receipt. A fence only refuses
   a late answer; it does not prove the process stopped touching the tree.
   Section 3.4 is how this rule is kept.

### 3.4 Stopping safely before anything replaces it

Every intervention that takes a working ticket or lot off its station (`Hold`,
`Pull` from a running lot, `Send`, `Skip`, `Stop`, `Cancel`) makes the same
transition, in the one event that records the command:

1. **Into `Stopping`.** The token's location becomes
   `Stopping { station, work, then }`. `work` is the running `Work` unchanged
   except that `work.stopping` names the original effect and `work.run`
   becomes a `stop:` run, as `Cancel` does today (`reducer.rs:280-286`).
   `then` is where the command sends it: `Held`, `Queued { station }` for a
   `Send` or a `Pull`'s refold, the `passed` route for a `Skip`, or `Closed`.
   The fence is bumped, so a late answer from the original is refused.
2. **What it keeps.** The original effect's binding (its id, run, resource
   generation and base) and everything it owns stay held:
   - the token still counts in its station's `working` set and in its pool,
     so no replacement starts in its place over capacity;
   - the tree or landing resource stays owned. Today that is
     `owns_resource` (`reducer.rs:558-561`), true for `Cancelling`, which
     keeps the `workflow_resources` row (`workflow_effects.rs:19-40`); the
     pipeline's `owns_resource` is true for `Stopping` for the same reason;
   - the original outbox row stays as it is: an admitted original turns
     `uncertain` when it goes inactive (`store/src/workflow.rs:346-351`), and
     recovery reconciles it as today.
3. **The stop.** It emits `EffectKind::Stop { operation }` and nothing else.
   The runtime keeps both of today's protections unchanged: a stop is not
   claimed while the effect it stops is still running in this runtime
   (`workflow_runtime.rs:620-624`), and `stop_coding`
   (`coding_execute.rs:389-425`) only answers `Stopped` with a quiescence
   receipt: a settled provision, a reconciled original, or a settled process
   for a process stage. An unknown push or landing acceptance is refused, and
   the stop answers `Uncertain`.
4. **The destination waits.** A `then: Queued { station }` puts the token in
   that station's queue at its place by priority, marked `stopping`, so the
   board shows where it will go and in what order. The station skips it when
   choosing what to start. Nothing launches on its tree, its lot or the
   landing target, and a `Release`, `Resolve` or `Skip` that would is refused
   `Stopping`.
5. **Out of `Stopping`.** Only `Observe` of the stop effect with
   `Outcome::Stopped { operation, receipt }` whose operation equals
   `work.stopping` and whose receipt is valid (the check of
   `reducer.rs:184-192`) moves the token to `then`, in the same event that
   frees its place and its resources. The station may then start the next
   ticket, which may be this one.
6. **An ambiguous stop holds.** If the stop answers `Uncertain` (or its own
   admission is lost), the token goes to `Held { Uncertain }` with the
   binding, the place and the resources still held, and `then` kept in the
   hold. Only `Resolve` with a stopped or settled receipt for the original
   (and `Settle` by a person for a lost push, as today) releases it; then
   `then` applies. No policy and no `Release` can skip this.

**A crash at any point.** The command, the move to `Stopping` and the stop
effect commit in one transaction, with the outbox rows
(`store/src/workflow.rs:296-344`). A crash before it leaves the token working
as it was. A crash after it leaves `Stopping` in the state and the stop in the
outbox; on restart, recovery reconciles admitted and uncertain effects first
(`workflow_runtime.rs:590-619`), then claims the stop, and `stop_coding`
reconciles the original from its receipts and journal, which is idempotent.
A stop that was admitted before the crash is reconciled the same way. Either
way the destination is reached only by the stop's receipt, or held. Nothing
in this path relies on the fence alone.

## 4. What must stay true

| Guarantee | Today | In the pipeline |
|---|---|---|
| **Pure reducer** | `step(&State, &Input) -> Result<Transition, Refusal>` (`reducer.rs:4`); no clocks or I/O (`lib.rs:1-2`) | `step(&Pipeline, &Input) -> Result<Transition, Refusal>`, same rules. Capacity and order decisions move into it, so it decides more, still purely |
| **Event log and replay** | Each accepted input appended to `workflow_events`, immutable by trigger (`store/src/workflow.rs:176-180`, `198-201`); state replays from it | Same table, same immutability. The cutover writes one immutable schema-3 snapshot per workspace and a `Genesis` event bound to it (5.2). Replay starts from that snapshot and folds every later event through the new `step`; it needs neither the old reducer nor `workflow_states`. The old log stays as read-only history |
| **Idempotence and order** | `seen`, `source_sequences`, `expected_revision`, `epoch` (`reducer.rs:12-16`) | Unchanged |
| **Fencing of effects** | Item `fence` and `run` in each effect; `Grant::Operation { effect, run, fence }` (`state.rs:175-179`); outbox claim and admit with resource generations (`workflow_runtime.rs:633-680`) | Fence per ticket and per lot. Every move of a token (send, pull, hold, stop, skip, cancel) bumps its fence, so a late answer from the station it left is refused `Stale`. The fence is not the proof of stopping: a working token moves through `Stopping`, which keeps its binding, place and resources until a stop receipt (3.4). Outbox, claim, admit and resource generations unchanged |
| **Crash recovery** | Admitted or uncertain effects reconciled from receipts and the journal before anything new starts (`workflow_runtime.rs:590-620`, `coding_execute.rs:601`); `Uncertain` waits for a `Resolve` | Unchanged, including across a crash between an intervention and its stop (3.4). An uncertain effect holds its token (`HoldKind::Uncertain`), never released by a policy |
| **Publishing needs a person** | `person_only` (`admission.rs:30`); the orchestrator only proposes (`workflow_commands.rs:51`) | Unchanged. Changing a station's policy or layout is a publish. `SetCapacity`, `Reorder`, holds and drains are operations on the running pipeline, not its process, and stay the orchestrator's |
| **Who decides a gate** | Approval and review deciders (`definition.rs:134-147`) | Unchanged, per station |

### 4.1 Changing the process for tickets in flight

There is one published process per pipeline. When a person publishes a new
one:

- Work already running finishes under the station version it started with;
  its effect still names the bundle it ran under (`Effect::policy_pin`).
- Every later arrival at a station follows the new version.
- A station that is removed must be named in the publish's mapping
  (`moves: { old_station: new_station }`). The publish is refused while a
  token sits at a removed station without a mapping, so nothing is ever
  stranded.

## 5. Cutover

One engine at a time, always. The new reducer lands unwired; one cutover step
switches the runtime, migrates every workspace in one transaction, and deletes
the old reducer. Each step below lands on its own and leaves main working.

1. **The pure pipeline crate module, unwired.** `crates/workflow/src/pipeline/`
   with the types in 2.3, `step`, `Definition` schema 3 (stations, pools,
   policies, moves) and the four template layouts. Tested alone, including
   every walkthrough in section 6 as a reducer test. Nothing in the daemon
   uses it.
2. **`migrate`, pure, with a refusal list.**
   `migrate(&State, &[Input], &[Dispatch], &Occupancy) -> Result<Pipeline, Vec<Quarantine>>`.
   It reads the old state, the old log for what the state doesn't hold (the
   revision each blocked item became ready, which orders its queue), each
   pending effect's dispatch status from the outbox, and what every other
   workspace holds of each host-wide pool (5.1). `Dispatch` is a plain
   value in the workflow crate, filled by the store from
   `workflow_outbox.dispatch_state` (`store/src/workflow.rs:187-188`), so
   `migrate` stays pure. This step also adds the snapshot table of 5.2,
   unused. Tested against fixtures for every phase and every dispatch
   status, and against copies of the live store through the existing
   cutover rehearsal (`crates/store/src/engine_cutover/rehearsal.rs`) with
   the cases of 5.3: every live item must map, and the projection (where
   each ticket is, its candidate, tree, followers, and each admitted
   effect's id, run and fence) must be equal before and after.
3. **The cutover.** In one landing:
   - the runtime calls the pipeline's `step`; the places module goes, as the
     reducer now decides what starts (`places.rs`, `launch_ready`'s pool
     check at `workflow_runtime.rs:625-632`). Section 5.1 is why that is safe
     for work already in flight;
   - wire actions: the new commands from 3.1, each naming a ticket and its
     expected station; `run` and `pin` leave the wire;
   - at startup, after the old runtime is quiesced (`engine_quiesce`,
     `engine_cutover.rs:403-417`) and before any effect is claimed, the store
     migrates every workspace in one immediate transaction under a new
     cutover incarnation (`engine_allowed`, `engine_cutover.rs:119`): it
     writes each workspace's snapshot and `Genesis` event (5.2), updates the
     outbox rows of work that never started (5.1), and stores schema 3 state.
     Anything that can't be mapped is held `Quarantined` with the reason,
     never dropped (the pattern of `engine_cutover.rs:248-306`). The same
     transaction then replays every workspace from its snapshot (5.2) and
     commits only if each replay equals the state it is about to store;
   - the old reducer, `Item`, `Phase` and the group machinery are deleted.
     Nothing after the cutover needs them: replay starts from the snapshot;
   - `workflow get` keeps the fields today's apps read, projected from the
     pipeline, so an app a release behind still draws the board.
4. **Apps and the orchestrator skill.** The Mac, iOS and web boards draw
   stations, queues and holds; the manager skill's playbook uses `Reorder`,
   `Hold`, `Pull`, `Send` and `Skip`. The legacy projection in `workflow get`
   goes.
5. **Join old continuation keys.** Tickets that were continued under new keys
   before the cutover (ov-1195 → ov-1204 → ov-1208, ov-1170 → ov-1214,
   ov-1143 → ov-1221) get a `continues` link on the board so their history
   reads as one. New ones never need it.

### 5.1 How tickets in flight migrate

Mapping is by station id: the template step ids (`provision`, `build`,
`gate-N`, `route`, `approve`, `branch`, `pr`, `merged`, `ci`) become the
station ids, and a group's steps become the landing stations, prefixed
`land.` (`land.base`, `land.fold`, `land.gate-0`, `land.push`, `land.ci`). An
item on an older pin whose step id has no station is held `Quarantined`.

**`Running` is not one thing.** The old reducer puts an item in `Running`
with a pending effect whether or not the runtime ever started it: on Nov 14
twelve builds were `Running` while four places existed, and the runtime only
skipped the other eight on each scan (`workflow_runtime.rs:625-632`). So
`migrate` splits `Running` (and a group item's running step) by the outbox
row of its pending effect, never by phase alone:

| Pending effect's `dispatch_state` | What it means | Token after | Its outbox row |
|---|---|---|---|
| `admitted` | It may be running or have run outside: admission is the boundary after which nothing re-executes it (`workflow_effects.rs:115-117`) | `Working` with the same `run`, `pending`, `fence`; counts in its station's `working` and its pool | Unchanged; reconciled by recovery as today (`workflow_runtime.rs:592-619`) |
| `uncertain` | Same, with its answer lost | `Working { uncertain }`, `Held { Uncertain }`; counts in `working` and its pool | Unchanged; waits for `Resolve` |
| `pending` | Emitted, never claimed. No external operation | `Queued` at its station, without `Work` | `suppressed`, in the cutover transaction |
| `claimed` | A runtime held a lease but did not admit it. No external operation: the runtime admits before any adapter call (`workflow_runtime.rs:652-671`), and the old runtime's claim can't be admitted under the new incarnation (`workflow_effects.rs:120`) | `Queued` at its station, without `Work` | `suppressed` |
| `settled`, `suppressed`, or no row | The state and the outbox disagree | `Held { Quarantined }`, with both recorded | Unchanged |

How the preserved and the queued effects meet the queue:

- **Admitted work keeps its identity and its place.** Its effect keeps its
  id, run, fence and resource generation, so its receipt, journal and
  `Observe` still bind (`store/src/workflow.rs:271-280` requires `admitted` for an
  `Observe`). It is in `working`, never in a queue. It frees its place only
  when its answer is observed, exactly as today.
- **Work that never started loses its effect id, not its turn.** Its token
  gets `fence + 1`, and the old row is `suppressed`, so no runtime can claim
  it (`workflow_effects.rs:98-103` claims only `pending` or an expired
  `claimed`) and no `Observe` of it is accepted. When the station reaches it,
  the reducer emits a new effect with a new id under the new fence. One
  ticket, one live effect: no duplicate can run.
- **Queue order is the order today's runtime would have started them.**
  Queued tickets are placed by priority (all 0 at cutover), then by the
  `(revision, id)` of the suppressed effect, which is the outbox order the
  old runtime launched in (`store/src/workflow.rs:162`). Tickets that were
  `Blocked` take the revision they became blocked. The orchestrator can
  reorder from the first command after the cutover.
- **Nothing starts at `Genesis`.** The `Genesis` event emits no effects, so
  the cutover itself never launches work. The first start happens on the
  next input, under the capacity rules below.

**Capacity across the cutover.** Each station's and pool's capacity is
set at `Genesis` from the runtime's configured caps at that moment
(`places.rs`, `set_effect_cap` at `workflow_runtime.rs:334`), recorded in the
snapshot. Occupancy is computed in the same transaction for every workspace
at once: each workspace's admitted and uncertain effects are its own
`working`, and what the others hold of a host-wide pool is stamped into its
`Genesis` and every later input as `busy_resources` is today
(`state.rs:192-194`). Starts are reducer decisions under the store's
immediate transaction, so two workspaces can't both take the last place. If
admitted work already exceeds a cap (a cap lowered just before the cutover),
all of it stays `Working`: the cutover never stops admitted work, and the
station starts nothing until it is back under the cap.

Other phases:

| Item today | Ticket after |
|---|---|
| Tracker ticket (`Definition::is_ticket`), `WaitingApproval` | `Resting { column }` |
| Follower (`follows`) | `Riding { lead }` |
| `Blocked` | `Queued` at its step's station, ordered by priority (0), then the revision it became blocked, read from the log |
| `WaitingApproval` on a run | `Awaiting { decider }` at that station |
| `WaitingReview` | `Awaiting { decider }` at the review station |
| `Running` | By dispatch status, above |
| `Cancelling` | `Stopping` (3.4) with `then: Closed`, the same `run`, `stopping` and `fence`; the original keeps its place and resources. Its stop effect keeps its id whatever its dispatch status: a stop takes no place and must still go out |
| `Uncertain` | `Working { uncertain }`, held `Uncertain`, waiting for `Resolve` as today |
| `Failed` | `Held { StepFailed }` with `failure` as the reason |
| `WaitingJoin` | `Queued` at the landing queue |
| `Joined` | `InLot` of the lot its group became |
| Group item | `Lot` with the same id, tickets, candidate, `pushed`, at its step's landing station, same fence; its pending effect split by dispatch status as above. A lot queued for the landing pool is queued in that order |
| `Done`, `Cancelled` in live state | Archived, as the next step would have done (`reducer.rs:455-465`) |

Counters (`back_edges`, `sent_back`, `rework_history`, `retries`,
`ci_reruns`) become visits, so policy counts continue from where they were.

### 5.2 Replay across the cutover

A digest can check a state but can't rebuild one, and the old reducer is
deleted at the cutover. So the cutover stores the migrated state itself, in
a place nothing can change:

- **The snapshot.** A new table,
  `workflow_genesis(workspace PRIMARY KEY, revision, epoch, schema, snapshot, snapshot_hash, legacy_through, legacy_chain, migrator)`,
  with the no-update and no-delete triggers `workflow_events` has
  (`store/src/workflow.rs:198-201`). `snapshot` is the whole schema-3
  `Pipeline` JSON inline (a few hundred KB at most under `MAX_ITEMS`), so
  its retention is the event log's: kept for as long as the store is.
  `legacy_through` is the last old revision; `legacy_chain` is a running
  digest of every old event row `(revision, id, payload_hash)` up to it;
  `migrator` names the code version that wrote it.
- **The `Genesis` event.** The first new event in `workflow_events`, at
  `legacy_through + 1`. Its payload carries `snapshot_hash`,
  `legacy_through`, `legacy_chain` and the stamped occupancy and caps, so the
  event and the snapshot each vouch for the other.
- **Replay.** `Pipeline::replay(&Genesis, &[Input])` is the only replay
  there is. It loads the `workflow_genesis` row, checks its `snapshot_hash`
  against the bytes and against the `Genesis` event, parses and validates the
  snapshot as schema 3, then folds every event after `Genesis` through the
  new `step`. It reads nothing else: not `workflow_states`, not the old
  reducer, not the outbox.
- **The old segment.** Events at or below `legacy_through` are never
  replayed again. They stay in the same immutable table as history, and
  `legacy_chain` proves the snapshot was cut from exactly that log. While
  the old reducer still exists (step 2), the rehearsal proves
  `migrate(replay_old(old_log)) == snapshot`; after step 3 that equation
  has been checked once, at the cutover, and the snapshot is the base.
- **A store from before the cutover** still holds only old events and no
  genesis row. The new daemon refuses to run on it without the cutover; it
  never tries to read old events as new ones.

**Replay-from-empty validation**, a test in step 2 and a check in step 3:

1. Build a store with an old log covering every phase, a lot mid-gate, a
   `Cancelling` ticket and the twelve-build case of 5.3.
2. Run the cutover.
3. Apply new inputs after it: an `Observe` of an admitted effect, a
   `Prioritize`, a `Pull` from the running lot and its stop receipt.
4. Open a fresh store connection, delete every `workflow_states` row, and
   build with the `pipeline` module only (the test target does not link the
   old reducer).
5. `Pipeline::replay` from each genesis row plus the events after it must
   equal, byte for byte, the state the live run stored, and its emitted
   effects must equal the outbox rows written after `Genesis`.

The same comparison runs inside the cutover transaction (step 3) and is
available afterwards as a new `farcooler workflow verify`, which replays every
workspace and reports any difference without changing anything.

### 5.3 Cutover rehearsal: twelve builds, four places

The case from 6.1, run in the rehearsal on a fixture and on a copy of the
live store:

- **Before.** One workspace, build cap 4. Twelve tickets `Running` at
  build. Four effects `admitted` (A1-A4), eight `pending` (P1-P8) at outbox
  revisions in that order. P6 and P7 are speed tickets. A second workspace
  holds one admitted build on the same host pool, host ceiling 5.
- **Cutover.** Then assert:
  1. Build `working` is exactly A1-A4, each with its original effect id,
     run, fence and resource generation; the outbox rows of A1-A4 are still
     `admitted`.
  2. The build queue is P1-P8 in outbox order. Their eight old outbox rows
     are `suppressed`; a claim on any of them returns nothing, and an
     `Observe` of any of them is refused.
  3. `Genesis` emitted no effect. The host pool counts 5 of 5 busy (A1-A4
     plus the other workspace's one), so nothing can start.
- **After.** `Prioritize { P6, 10 }`, `Prioritize { P7, 10 }`, then
  `Observe` A2 passed. Assert exactly one new build effect, for P6, with a
  new id and fence `old + 1`, and that the queue is P7, P1-P5, P8. Then the
  other workspace's build ends: P7 starts, and nothing else.
- **Crash.** Kill the process inside the cutover transaction: the store is
  exactly the old store, still under the old incarnation. Kill it after
  commit, before any claim: on restart, A1-A4 are reconciled first and no
  P effect exists to run.
- **Replay.** The replay-from-empty check of 5.2 holds at each point above.

## 6. The Nov 14-15 failures, walked through

### 6.1 12 builds for 4 slots, in arbitrary order

**Then.** The reducer put twelve tickets into `Running` with a build effect
each. The runtime started four, in outbox order `revision, id`
(`store/src/workflow.rs:162`), and noted the rest as waiting, a list it
rebuilt on each scan (`workflow_runtime.rs:625-632`). Speed tickets waited
behind paused feature work, and nothing could change the order: the list was
not in the reducer, and features could only be cancelled, losing their run.

**Now.** The build station's queue holds twelve tickets in order. The
orchestrator sends `Prioritize` for each speed ticket, or `Reorder` to put one
first, and `Hold` on the paused features: they leave the queue, keep their
worktrees and briefs, and `Release` puts them back. `workflow get` shows the
queue as the station will start it. A paused feature that is already
building goes through `Stopping` (3.4): its place frees, and the next speed
ticket starts, only when its agent has proved it stopped.

If this state is live at the cutover, the four admitted builds stay working
with their effect ids and the eight that never started become the build
queue in the order the old runtime would have started them (5.1, rehearsed
in 5.3).

### 6.2 A ticket that uses up its limit dies and restarts under a new key

**ov-1195 → ov-1204 → ov-1208 (stage timing).** **Then.** The review step's
route back to build has a limit of 2 (`templates.rs:129`). After two rejections
the route failed the run (`reducer.rs:757-758`); `Retry` only re-runs the
failed review step (`reducer.rs:329-339`), and `SendBack` was refused at the
same limit (`reducer.rs:416-419`). The only way on was a new ticket, on a new
branch started from the old one. ov-1208's own intent lists "continued under
a new key" as a lifecycle state its timing had to handle.

**Now.** The second rejection holds the ticket at review, `PolicyLimit
review.rework`, with the finding as the brief. The orchestrator reads it and
sends `Send { to: build, reason }`. The visit records the override. Same key,
same branch, and timing is a read of one ticket's visits; the "new key" state
no longer exists.

**ov-1170 → ov-1214 (builders yield).** **Then.** Three failed reviews; the
ticket was re-keyed with a narrowed scope. **Now.** Held after the second
rejection; the orchestrator edits the board acceptance and sends it back with
the narrowed scope as the brief. One key.

**ov-1143 → ov-1221 (no orphan tests).** **Then.** It passed review, then
broke the landing gate three times; its join's rejected route has limit 1
(`templates.rs:160`), and its send-backs were spent, so it failed for good.
**Now.** The lot's gate comes back red; bisecting finds ov-1143 (the failing
fixture tests name files it changed, so it is a suspect from the first
result). It returns to build with the gate's output as its brief, and the
other tickets gate and push without it. A third return holds it for the
orchestrator, who can send it again, with the override recorded.

### 6.3 The process can't change for tickets in flight

**Then.** Each item carries its definition and pin (`state.rs:22-23`); the
state is invalid if they differ (`reducer.rs:796`), and the runtime refuses to
launch if the item's bundle no longer matches (`service_owner.rs:135-139`).
Adding the owner's group review to the landing steps would have applied only
to tickets started after the publish.

**Now.** A person publishes the layout with group review between cheap checks
and the gate. The next lot to arrive at that point goes through it, whatever
its tickets started under. A lot gating at that moment finishes under the
version it started.

### 6.4 The orchestrator can't pause, drain, reorder, or pull from a gating group

**Then.** No station exists to pause or drain. Pulling one ticket from a group
works only while the group is still `Blocked` (`detach`, `reducer.rs:569-581`);
once it is gating, the only move is to cancel the whole group by its group
id, and every command needs the item's current `run` and `pin`
(`workflow_commands.rs:19-59`).

**Now.** `Pull { ticket }` takes the ticket out and holds it at once. The lot
keeps its id and the rest of its tickets and goes to `Stopping`: it keeps its
place, its tree and the landing target until the gate's stop receipt proves
the gate process ended, then folds again without the ticket (3.4). If the
stop can't prove that, the lot holds `Uncertain` for reconciliation instead
of refolding over a tree a gate may still be using.
`HoldStation { land.gate }` pauses gating (say, while main is red);
`Drain { build }` lets builds finish before a cutover. Every command names the
ticket and the station the orchestrator believes it is at, nothing more.

### 6.5 A split spawns many small groups, each costing a full gate and CI

**Then.** A red gate splits the group: one half goes back to its base wait,
the other is born as a new group item at step 0 (`reducer.rs:731-741`,
`435-449`; templates' `split("base")`, `templates.rs:150-152`). Each half
waits for the base, folds, gates, pushes and waits for CI on its own.

**Now.** Cheap checks and group review catch many implicit conflicts in
minutes before the gate. A red gate marks suspects and bisects inside the one
lot; only the failing tickets leave; the rest push once and wait for CI once
(section 2.8).

## 7. Mapping: every Item field, Phase, Command and Refusal

### 7.1 `Item` fields (`state.rs:21-103`)

| Field | In the pipeline | Why |
|---|---|---|
| `definition` | Removed from the ticket; the pipeline's published definition | The process belongs to the pipeline (6.3) |
| `pin` | `Pipeline::pin`; each effect keeps the pin it ran under | Same |
| `policy_pin` | `Pipeline::pin` (bundle), on each effect as today | Same |
| `dependencies` | `Ticket::dependencies` | The ticket's own |
| `resource` | Removed for tickets (the tree lock stays the runtime's, by ticket); a lot's exclusive resource is the landing pool's | Capacity is the station's |
| `stage` | `Location`'s `station` | Position is a station, not an index into a private copy |
| `iteration` | Derived from `visits` | History, not a budget |
| `attempt` | `Work::attempt` | Per visit |
| `fence` | `Ticket::fence`, `Lot::fence` | Kept: fencing |
| `phase` | `Location` (see 7.2) | |
| `run` | `Work::run`, `Awaiting::run` | Kept: binds answers |
| `pending` | `Work::pending` | Kept |
| `candidate` | `Ticket::candidate`, `Lot::candidate` | The ticket's own |
| `last_evidence` | `Ticket::last_evidence` | The ticket's own |
| `stopping` | `Work::stopping` | Kept |
| `rework_history` | `visits` entries ending `rejected`, with principal, reason, evidence | One history |
| `back_edges` | Derived from `visits`; counted by station policy | Limits become policy |
| `retries` | Derived from `visits`; no cap | Limits become policy |
| `members` | `Lot::tickets` | Groups become lots |
| `group_of` | Removed | A lot is not an item with a definition |
| `pushed` | `Lot::pushed` | Kept: the landing rule (3.3) |
| `handed` | `Lot` archive entry's tickets | Kept for the board |
| `since_ms` | Derived from the last visit's revision and the log's `now_ms` | History |
| `grouped_in` | `Location::InLot { lot }` | |
| `wait_since` | `Work::wait_since` | Kept: patience survives restarts |
| `models` | `Ticket::models` | The ticket's own picks |
| `tree` | `Ticket::tree` | The ticket's own |
| `sent_back` | `visits` entries ending `sent_back`, with id, revision, from, to, reason | One history |
| `brief` | `Ticket::brief` | The ticket's own |
| `follows` | `Location::Riding { lead }` | |
| `followers` | `Ticket::followers` | |
| `cancel_reason` | Archive entry's reason | |
| `failure` | `Ticket::failure`, and the hold's reason | |
| `ci_reruns` | Derived from `visits`; counted by the `ci_rerun` policy | |
| `ci_retry` | `Work::ci_retry` | Kept: binds the rerun to its run |

`State` itself: `schema` (3), `workspace`, `revision`, `epoch`, `seen`,
`source_sequences` and `archive` stay as they are; `items` splits into
`tickets` and `lots`; `stations` and `pin` are new. Constants:
`MAX_RETRIES`, `DEFAULT_SEND_BACKS`, `MAX_SEND_BACKS`, `MAX_BACK_EDGE_LIMIT`
and `max_reworks` become policy defaults with no hard cap behind them;
`MAX_ITEMS` stays as a bound on tickets plus lots.

### 7.2 `Phase` (`state.rs:15-19`)

| Phase | In the pipeline | Why |
|---|---|---|
| `Blocked` | `Queued { station }` | A queue with an order, not a flag |
| `WaitingApproval` | `Awaiting { decider }`, or `Resting` for a tracked ticket | The tracker template is a resting place, not a run |
| `WaitingReview` | `Awaiting { decider }` | |
| `Running` | `Working` | |
| `Uncertain` | `Working { uncertain }` plus `Held { Uncertain }` | Visible as needs-you, still bound to its effect |
| `Cancelling` | `Stopping { then }`, where `then` is `Closed`, `Held`, a queue, or a `Skip`'s next station | A stop no longer always means cancel; the place and tree are kept until its receipt (3.4) |
| `Done` | `Closed { landed: true }`, then archived | |
| `Failed` | Removed. `Held { StepFailed }` or `Held { PolicyLimit }` | Nothing dies of a limit |
| `Cancelled` | `Closed { landed: false }`, then archived | |
| `WaitingJoin` | `Queued` at the landing queue | |
| `Joined` | `InLot` | |

### 7.3 `Command` (`state.rs:197-230`) and wire `Action` (`workflow_commands.rs:16-60`)

| Today | In the pipeline | Why |
|---|---|---|
| `Advance` | `Advance` | The runtime's tick when cross-workspace occupancy changes |
| `Enqueue` | `Admit { ticket, station }` | Used by fixtures; same as `Start` without a resting ticket |
| `Approve` | `Approve` | Decider unchanged |
| `Review` | `Review` | Decider unchanged; a rejection follows the station's policy |
| `Observe` | `Observe` | Unchanged: effect, run, fence |
| `Cancel` | `Cancel { ticket, reason }`; `Stop` for "stop the step, keep the ticket" | Separates the two |
| `Retry` | `Send { to: same station }` | No cap |
| `Queue` | Removed. The landing queue forms lots by its policy; `Reorder` and `Hold` steer it | Joining is arriving at a station |
| `Leave` | `Pull` | Works on a gating lot too |
| `Move` | `Move` | Resting columns, as today |
| `Start` | `Start` | Same key, now into the pipeline's process |
| `Resolve` | `Resolve` | Unchanged |
| `SendBack` | `Send { to, reason }` | Any station, any number of times |
| `Reopen` | `Reopen` | Same key |
| (wire) `Get` | `Get`, showing stations and queues | |
| (wire) `Publish` | `Publish`, person-only, with `moves` | 4.1 |
| (wire) `Settle` | `Settle`, person-only | Unchanged |
| (wire) `Track` | `Track` | |
| (wire) `Propose`, `Drafts` | Unchanged | |
| New | `Reorder`, `Prioritize`, `Hold`, `Release`, `Pull`, `Skip`, `Stop`, `SetCapacity`, `HoldStation`, `ResumeStation`, `Drain` | Section 3 |

### 7.4 `Refusal` (`state.rs:7-12`)

| Refusal | In the pipeline | Why |
|---|---|---|
| `Definition` | Kept | Malformed process |
| `Invalid` | Kept | Malformed input, including a fence at its end |
| `Duplicate` | Kept | Idempotence |
| `Stale` | Kept, and now also "the ticket isn't at the station you named" | The one staleness check on the wire |
| `Unauthorized` | Kept | Grants |
| `Missing` | Kept | |
| `Blocked` | Removed | Never produced today |
| `Resource` | Removed | Work waits in a queue instead (3.2) |
| `Phase` | Narrowed to `Landed` | Only the landing rules refuse by position (3.3) |
| `Evidence` | Kept | Binds answers to candidates |
| `Limit` | Removed as a ticket refusal; kept only for `MAX_ITEMS` | Limits are policy |
| `SendBackRefused(String)` | Removed | `Send` is never refused by a count |
| New: `PersonOnly` | Publish, settle, and per decision A | |
| New: `Stopping` | A command that would start work on a tree or lot whose last effect has no stop receipt yet | Safety, not a count (3.3 rule 4, 3.4) |

## 8. Decisions for the owner

These need the owner's word before any build starts.

**A. May the orchestrator skip the gate in front of a push?** Skipping any
other station is the orchestrator's, with a reason. Skipping the gate (or the
CI wait) on a lot that will push puts untested code on main. *Recommendation:*
a person's command only (`PersonOnly`); the orchestrator can ask for it on
the board, as with `Settle`.

**B. Do tickets in flight follow a new process at their next station?**
*Recommendation:* yes, as in 4.1. Running work finishes under the version it
started with; every later arrival follows the new one; a publish that removes
an occupied station must say where its tickets go. The alternative, pinning
each ticket's process until it lands, is today's behavior and the cause of
6.3.

**C. Is the pipeline per workspace, or one per host?** *Recommendation:* one
per workspace (a board's tickets, policies and queues), with the agent pool's
ceiling shared across the host and stamped into each input as
`busy_resources` is today. One per host would let one order cover every
board, but would put every workspace's tickets in one log and make a team's
boards share one process.
