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
| Hold a ticket | `Hold { ticket, reason }` | Queued: leaves the queue. Working: its effect is stopped (fenced), its place freed. In a lot: as `Pull`. It keeps its tree and candidate | Visit ends `held`, reason |
| Release a ticket | `Release { ticket, to: Option<station> }` | Back into the queue of the station it was held at, or `to` | Visit, reason |
| Pull from a queue or a forming lot | `Pull { ticket, reason }` | Leaves the lot; holds at the landing queue | Ticket visit and lot visit |
| Pull from a running gate | `Pull { ticket, reason }` | The lot's running effect is stopped and fenced; the lot returns to fold without the ticket, keeping its id; the ticket holds | Both visits; the stopped run's receipt |
| Send back, any number of times | `Send { ticket, to, reason }` | Moves it to any station before where it is, after any landing it has made (see 3.3); its brief is `reason`; it queues there by priority | Visit with `by`, reason, and `overrode` when a policy count was past its limit |
| Run the step again | `Send { ticket, to: <its own station> }` | Today's `Retry`, without `MAX_RETRIES` | As above |
| Skip a step | `Skip { ticket, station, reason }` | The ticket leaves the station as `skipped` and goes on along its `passed` route | Visit ends `skipped`, reason; see decision A for the gate in front of a push |
| Stop the running step | `Stop { ticket or lot, reason }` | Emits the stop for the effect, fences it, and holds the token at the station, nothing else lost | Visit ends `stopped` |
| Cancel a ticket | `Cancel { ticket, reason }` | Closes it (stopping its work first). Its key stays taken; `Reopen` brings it back under the same key | Archive entry with reason |
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
| `SendBack` while a step runs → `Phase` | `reducer.rs:403-406` | `Send` stops and fences the running effect first |
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

## 4. What must stay true

| Guarantee | Today | In the pipeline |
|---|---|---|
| **Pure reducer** | `step(&State, &Input) -> Result<Transition, Refusal>` (`reducer.rs:4`); no clocks or I/O (`lib.rs:1-2`) | `step(&Pipeline, &Input) -> Result<Transition, Refusal>`, same rules. Capacity and order decisions move into it, so it decides more, still purely |
| **Event log and replay** | Each accepted input appended to `workflow_events`, immutable by trigger (`store/src/workflow.rs:176-180`, `198-201`); state replays from it | Same table, same immutability. The cutover's first event per workspace is a `Genesis` input carrying the migrated pipeline's digest (section 5), so replay from that point is exact and the old log stays readable as history |
| **Idempotence and order** | `seen`, `source_sequences`, `expected_revision`, `epoch` (`reducer.rs:12-16`) | Unchanged |
| **Fencing of effects** | Item `fence` and `run` in each effect; `Grant::Operation { effect, run, fence }` (`state.rs:175-179`); outbox claim and admit with resource generations (`workflow_runtime.rs:633-680`) | Fence per ticket and per lot. Every move of a token (send, pull, hold, stop, skip, cancel) bumps its fence, so a late answer from the station it left is refused `Stale`. Outbox, claim, admit and resource generations unchanged |
| **Crash recovery** | Admitted or uncertain effects reconciled from receipts and the journal before anything new starts (`workflow_runtime.rs:590-620`, `coding_execute.rs:601`); `Uncertain` waits for a `Resolve` | Unchanged. An uncertain effect holds its token (`HoldKind::Uncertain`), never released by a policy |
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
2. **`migrate(&State, &[Input]) -> Pipeline`, pure, with a refusal list.**
   Uses the old log for what the old state doesn't hold (the revision each
   blocked item became ready, which orders its queue). Tested against
   fixtures for every phase, and against copies of the live store through the
   existing cutover rehearsal (`crates/store/src/engine_cutover/rehearsal.rs`):
   every live item must map, and the projection (where each ticket is, its
   candidate, tree, followers, and every pending effect's id, run and fence)
   must be equal before and after.
3. **The cutover.** In one landing:
   - the runtime calls the pipeline's `step`; the places module goes, as the
     reducer now decides what starts (`places.rs`, `launch_ready`'s pool
     check at `workflow_runtime.rs:625-632`);
   - wire actions: the new commands from 3.1, each naming a ticket and its
     expected station; `run` and `pin` leave the wire;
   - at startup, before any effect is claimed, the store migrates every
     workspace in one immediate transaction under a new cutover incarnation
     (`engine_allowed`, `engine_cutover.rs:119`), writes each `Genesis`
     event, and stores schema 3 state. Anything that can't be mapped is held
     `Quarantined` with the reason, never dropped (the pattern of
     `engine_cutover.rs:248-306`);
   - the old reducer, `Item`, `Phase` and the group machinery are deleted;
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

| Item today | Ticket after |
|---|---|
| Tracker ticket (`Definition::is_ticket`), `WaitingApproval` | `Resting { column }` |
| Follower (`follows`) | `Riding { lead }` |
| `Blocked` | `Queued` at its step's station, ordered by priority (0), then the revision it became blocked, read from the log |
| `WaitingApproval` on a run | `Awaiting { decider }` at that station |
| `WaitingReview` | `Awaiting { decider }` at the review station |
| `Running`, `Cancelling` | `Working` with the same `run`, `pending`, `stopping`, `fence`, so the outbox effect and its receipt still match |
| `Uncertain` | `Working { uncertain }`, held `Uncertain`, waiting for `Resolve` as today |
| `Failed` | `Held { StepFailed }` with `failure` as the reason |
| `WaitingJoin` | `Queued` at the landing queue |
| `Joined` | `InLot` of the lot its group became |
| Group item | `Lot` with the same id, tickets, candidate, `pushed`, at its step's landing station, same fence and pending effect |
| `Done`, `Cancelled` in live state | Archived, as the next step would have done (`reducer.rs:455-465`) |

Counters (`back_edges`, `sent_back`, `rework_history`, `retries`,
`ci_reruns`) become visits, so policy counts continue from where they were.
The pending effect of every token keeps its id, run and fence, so an effect
running across the restart is reconciled as today, against the new state.

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
queue as the station will start it.

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

**Now.** `Pull { ticket }` stops the lot's gate, fences it, and folds the lot
again without the ticket; the lot keeps its id and the rest of its tickets.
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
| `Cancelling` | `Working { stopping }` with the ticket on its way to `Closed` or `Held` | A stop no longer always means cancel |
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
