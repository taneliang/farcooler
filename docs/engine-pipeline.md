# A workflow engine that work flows through

Design spike, second pass (ov-1251; first pass ov-1223). No product code. All
file and line references are to `main` at cbf8a84ca; the cited engine files
are unchanged since the first pass's base (73974f898).

## Summary (one page)

**What the engine is.** A workflow engine any project can run its work
through. Each workspace has one **workflow**: a layout of **stations** that
work moves through as **tokens**. A token is a board task, from intake to
close, under one key for life. Each station has a queue in an explicit order,
a capacity and a policy. The engine's core knows only that: stations, queues,
order, capacity, policy, tokens, holds and batches. Everything about a
particular kind of work (git branches, folds, gates, pushes and CI for code;
mailboxes and sending for an assistant) lives in an **adapter**, a plug-in
with a fixed interface (section 3). Coding is the first adapter, not the
engine's shape.

**Five station kinds** cover every workflow we tried: an **agent** step, a
**command** step, a **decision** by a person or the orchestrator, a **wait**
on something outside, and a **batch** that carries several tokens through a
run of stations together. Far Cooler's build, review, fold, gate, push and CI
are configurations of these (section 4.2). So are a company's human pull
request review, a personal assistant's triage-draft-approve-send, and a
research newsletter (section 8).

**The orchestrator can do everything.** It can reorder, hold, pull, send back
any number of times, stop a step, change capacity, and **skip any station,
including the gate before a push**, each with a reason written into the
token's history. Limits never kill a ticket; at worst a policy holds it and
the orchestrator moves it on. What the engine still refuses is about safety,
not permission: it never undoes something that reached the outside world (a
push, a sent email) and never starts new work on something an old step may
still be touching.

**Process changes apply at once; nothing is pinned.** A published change
takes effect in the event that publishes it. Queued and held tokens follow
the new layout immediately. A step already running finishes, and its result
is routed by the new layout. A removed or renamed station is named in the
change's **moves**; the publish is refused while anything would be stranded.
`workflow preview` runs the change on a copy of the live pipeline and shows
exactly where every token ends up before anything is published (section 7).

**Limits are per workspace.** Each workspace sets its own pools (how many
agents, how many landing runs). Workspaces are independent: with no host
limit, one never waits for another. A user may also set a **host limit**;
then a step starts only if its station, its workspace pool and the host
limit all have room, so the tighter limit always wins (section 6).

**The cutover** keeps every first-pass guarantee: one engine at a time,
in-flight work migrated by its outbox status, an immutable snapshot that
replay starts from, and a rehearsal of the twelve-builds case (section 10).

### Questions that are the owner's

Everything in this design is the orchestrator's to do except the items below.
Each is a question, with a recommendation; strike any you disagree with.

| # | Still needs a person? | Recommendation |
|---|---|---|
| Q1 | **Publishing a workflow** | No by default: the orchestrator publishes. A workspace setting `publish: person` (set only by a person) reserves it, for teams that want it |
| Q2 | **Writing to the secret store** | Yes. A secret is the one input the orchestrator can't check and can leak; a person types it once |
| Q3 | **Security flags** (granting the orchestrator or an agent a wider permission mode, sandbox exceptions, clearing a security hold) | Yes, for widening; no for narrowing |
| Q4 | **Production deploys** | Not in Far Cooler (it has none). In a template that deploys, the deploy decision is a person's by default; the workspace can hand it to the orchestrator |
| Q5 | **Settling an outward step whose result is unknown** (a push or a send that may or may not have happened) | No: the orchestrator settles, but only with evidence it read from the outside world (the remote's ref, the sent folder), recorded |
| Q6 | **Grants and roles, and the host limit** | Yes. These are where authority and cross-workspace limits come from; one workspace's orchestrator shouldn't widen its own powers or take another workspace's share |
| Q7 | **May a workflow reserve a station to a person** (skip or decide only by a person, for compliance)? | Yes, as opt-in station policy `skip: person`. Off in every Far Cooler template; on for the company template's code review and production deploy |

Sections 5.4 and 9 give each reason in full.

## What changed from the first pass

| Change | Owner's answer behind it (Nov 15) |
|---|---|
| The core no longer knows about code. `Lot`, `RunTree`, `candidate`, `pushed`, `base`, fold, gate, push and CI move into the coding adapter; the core keeps stations, queues, order, capacity, policy, tokens, holds and batches (section 3) | "My intent with the engine was to make it super flexible so that each project can define its own workflows… a flexible workflow engine can also support non engineering work like being a personal assistant." |
| Station kinds are five generic kinds (agent, command, decision, wait, batch), not coding verbs; coding stations are configurations (section 4) | Same |
| Template workflows per kind of project: agentic, company with human review, personal assistant, content (section 8) | "publish/ship some template workflows depending on the project" |
| A workflow may use more than one adapter, and adapters may create tokens (intake), so an assistant can read mail and a calendar | Same: non-engineering work needs it |
| One workflow per workspace, with the door left open for several (section 2.4) | "Having 1 workflow per workspace is probably best for now to limit complexity." |
| The orchestrator may skip any station, including the gate before a push. The first pass's `PersonOnly` rule for that skip is gone (section 5) | "yes, again allow the orchestrator to do everything. There should be almost nothing that needs to be approved by a person." |
| Publishing, settling and the other person-only actions become owner questions, each with its own reason (Q1-Q7, section 9) | Same |
| A published change applies to work in flight at once. The first pass let a running step finish "under the version it started with"; now its result is routed by the new layout, and a removed station's tokens move by the change's `moves`, previewed first (section 7) | "should follow the new process immediately… some way to transition work without interrupting them… Def don't pin tickets to workflow versions" |
| Limits are per workspace; the first pass's host-wide pools become an optional host limit (section 6) | "Agent limits should be set per workspace as well… Workspaces should be independent. If the user needs to set a host level limit they should still be able to do that" |
| The landing rules (never go back across a push; settle before moving) are generalized to any **outward** step, so a sent email gets the same protection as a push (section 5.3) | Follows from the non-engineering intent |
| Kept from the first pass: one identity for life; limits are station policy that holds rather than kills; explicit ordered queues with priority; replay, fencing and crash recovery; safe stopping; bisecting in place; cheap checks and group review before the gate; a cutover with no period of two engines | The ticket's constraints |

## 1. Where the code is today

The engine today is a per-ticket state machine, and its core is written in
coding verbs:

- **Every ticket carries the process.** `State` is a map of `Item`s
  (`crates/workflow/src/state.rs:135`); each item holds its own `Definition`
  and pin (`state.rs:22-23`), position, fence and budgets (`state.rs:27-99`).
  A landing group is one more `Item` (`reducer.rs:302-319`), born again at each
  split (`reducer.rs:435-449`). Nothing represents a station, its queue or its
  capacity; the only shared thing is the runtime's three host-wide pools
  (`crates/daemon/src/workflow_runtime/places.rs:5-12`), filled in outbox
  order (`crates/store/src/workflow.rs:162`).
- **The core's step kinds are coding verbs.** `StageKind`
  (`crates/workflow/src/definition.rs:21-52`) is `Provision`, `Build`,
  `Check`, `Review`, `Land`, `Ci`, `AwaitBase`, `Fold`, `Push`, `AwaitCi`,
  `PushBranch`, `OpenPr`, `AwaitPr` and so on. Only `Run`, `Action(String)`,
  `Evaluate(String)`, `Review(Reviewer)` and `Progression::AwaitApproval`
  (`definition.rs:5-19`) are generic.
- **The core reducer knows git and CI.** `Item` has `tree: RunTree` (branch,
  path, base; `state.rs:110-114`), `candidate`, `pushed`, `ci_retry`; the
  reducer handles `Outcome::CiResult` and `Outcome::FoldConflict`
  (`reducer.rs:163-194`), spends CI reruns (`crates/workflow/src/ci.rs`), and
  refuses by `lands()` (`definition.rs:488`).
- **There is already an adapter slot, with one adapter.**
  `AdapterConfiguration` is a closed enum with one arm, `coding/v1`
  (`crates/protocol/src/workflow_config.rs:94-97`). A published bundle is a
  definition plus an adapter (`crates/daemon/src/landing_templates.rs:43`).
  The coding adapter already validates layouts (the order rules in
  `crates/daemon/src/workflow_runtime/coding.rs:204-280`), executes
  (`coding_execute.rs:446`), reconciles (`coding_execute.rs:601`) and stops
  (`coding_execute.rs:389`). The design below widens this seam until the core
  needs nothing else from coding.

## 2. The model

### 2.1 Terms

- **Workflow**: one per workspace (2.4). A published layout of stations, the
  routes between them, their policies, the workspace's pools, and the
  adapters it uses with their configuration.
- **Pipeline**: the running workflow: the published layout plus the live
  state of every station and token. One per workspace, with one event log.
- **Station**: a place where one kind of work is done. It has a **kind**
  (4.1), a **queue** in order, a **capacity**, and a **policy**.
- **Token**: one board task, from intake (or `Track`) until it closes, and
  again under the same key if reopened.
- **Batch**: tokens that travel a run of stations together (a landing group,
  a newsletter issue, a daily digest). A carrier, not a token: its tokens
  keep their keys, and it can shrink without becoming a new batch.
- **Visit**: one stay of a token or batch at a station: arrival, start, end,
  the label it left with, who moved it and why.
- **Hold**: a token or station stopped with a reason until someone releases
  it. The pipeline's only "needs you". There is no terminal failure.
- **Adapter**: the code for one kind of work (coding, mail, a CMS). It names
  the **operations** stations run (`coding.build`, `mail.send`) and keeps its
  own state on each token and batch, which the core stores but never reads.
- **Outward step**: an operation that changes something outside the engine
  that can't be taken back: a push, a merge, a sent email, a published post,
  a deploy. The adapter declares which of its operations are outward; the
  core enforces the rules for them (5.3).
- **Intake**: an adapter source that creates tokens, such as new mail threads
  or a research topic list.

### 2.2 What the core owns, what the token carries, what the adapter owns

| The pipeline (core) owns | A token carries (core) | The adapter owns, per token or batch |
|---|---|---|
| The published workflow and its digest | Its key, forever | Its state blob: for coding, the tree, branch, base, candidate commit, pushed commit, PR number, CI run and rerun counts |
| Each station's queue, working set, capacity, hold, drain | Where it is (`Location`) | What an operation needs and produces |
| Pools and their capacity | Its priority | How an outcome becomes a label (a fold conflict is label `conflict`) |
| Batches: members, location, crossing record, bisect progress | Its brief and last failure | Which members are suspects when a batch fails |
| The event log, idempotence, epoch | Its visits (all history and overrides) | Board fields to show (branch, PR link, recipient) |
| Fences and stop receipts | Its dependencies, followers, picked models | Execution, reconcile, stop, probe |
| Holds and policy counts | Its crossing record, if an outward step took it out | Validation of a layout's use of its operations |

### 2.3 Core types

Sketch, not final code. No core type names a branch, commit, base, push, pull
request or CI run; the core's word for "reached the outside world" is
**crossing**.

```rust
pub struct Pipeline {
    pub schema: u32,                          // 3
    pub workspace: String,
    pub revision: u64,
    pub epoch: u64,
    pub workflow: Published,                  // the layout in force; changed only by a Publish event
    pub stations: BTreeMap<StationId, Station>,
    pub retired: BTreeMap<StationId, Retired>,// removed by a publish, still finishing work (7.3)
    pub pools: BTreeMap<String, Pool>,
    pub tokens: BTreeMap<String, Token>,
    pub batches: BTreeMap<String, Batch>,
    pub seen: BTreeSet<String>,
    pub source_sequences: BTreeMap<String, u64>,
    pub archive: BTreeMap<String, Archived>,
}
pub struct Published { pub digest: String, pub layout: Layout, pub adapters: BTreeMap<String, AdapterRef> }
pub struct AdapterRef { pub version: String, pub config_digest: String }   // e.g. "coding/v1"
pub struct Station { pub kind: Kind, pub queue: Vec<String>, pub working: BTreeSet<String>,
    pub capacity: u16, pub held: Option<Hold>, pub draining: bool }
pub struct Pool { pub capacity: u16, pub held_by: BTreeSet<String> }
pub struct Token {
    pub at: Location,
    pub priority: i32,
    pub data: BTreeMap<String, Opaque>,       // per adapter; the core stores and passes it, never reads it
    pub evidence: Option<Evidence>,
    pub crossing: Option<Crossing>,
    pub brief: Option<String>,
    pub failure: Option<String>,
    pub models: BTreeMap<StationId, String>,
    pub dependencies: BTreeSet<String>,
    pub followers: Vec<String>,
    pub visits: Vec<Visit>,
    pub fence: u64,
}
pub struct Batch { pub at: Location, pub members: Vec<Member>, pub data: BTreeMap<String, Opaque>,
    pub crossing: Option<Crossing>, pub suspects: Vec<String>, pub bisect: Option<Bisect>,
    pub visits: Vec<Visit>, pub fence: u64 }
pub struct Opaque { pub version: String, pub value: serde_json::Value, pub digest: String }
pub struct Evidence { pub artifact: String, pub manifest: String, pub receipt: String, pub spec: String }
pub struct Crossing { pub station: StationId, pub artifact: String, pub receipt: String, pub revision: u64 }
pub enum Location {
    Resting { column: String },
    Queued { station: StationId },
    Working { station: StationId, work: Work },
    Stopping { station: StationId, work: Work, then: Box<Location> },   // 5.2
    Settling { station: StationId, work: Work, then: Box<Location> },   // 5.3
    Awaiting { station: StationId, decision: Decision },                // a decision station
    InBatch { batch: String },
    Riding { lead: String },
    Held { station: StationId, hold: Hold },
    Closed { crossed: bool },
}
pub struct Work { pub run: String, pub attempt: u8, pub pending: Option<String>, pub op: OpRef,
    pub stopping: Option<String>, pub uncertain: bool, pub wait_since: Option<i64>,
    pub again: Option<Opaque> }               // an adapter's "run me again with these facts" (the CI rerun)
pub struct OpRef { pub adapter: String, pub op: String, pub spec: String }  // spec = digest of what it ran
pub struct Decision { pub run: String, pub deciders: Vec<String>, pub needs: u8, pub given: Vec<Verdict> }
pub struct Hold { pub reason: String, pub by: String, pub revision: u64, pub kind: HoldKind }
pub enum HoldKind { Asked, PolicyLimit { policy: String }, StepFailed, Uncertain, Quarantined }
pub struct Visit { pub station: StationId, pub arrived: u64, pub started: Option<u64>, pub left: Option<u64>,
    pub ended: Option<String>, pub by: String, pub reason: Option<String>, pub overrode: Option<String> }
pub enum Outcome {
    Labeled { label: String, evidence: Evidence, data: Option<Opaque> },
    Failed { reason: Option<String> }, Interrupted, Uncertain,
    Stopped { operation: String, receipt: String },
    Settled { operation: String, original: Box<Outcome>, receipt: String },   // 5.3
}
pub enum EffectKind { Execute(OpRef), Probe(OpRef), Stop { operation: String } }
```

`Effect` keeps today's fields (`state.rs:252-264`) with `item` renamed
`token`, `kind` as above, `resource` replaced by `resources: Vec<String>`
(opaque names the adapter gave, such as `coding:target:main`), and
`definition_pin`/`policy_pin` replaced by `op.spec`, the digest of the exact
operation spec the effect ran. That is a record of what ran, not a pin on the
token.

A CI check in the repository (`scripts/`, run in CI) greps the core module
for `branch`, `commit`, `base`, `push`, `pull_request`, `ci`, `git`, `fold`
and `gate` as identifiers, so the line stays where it is drawn. It is
mutation-tested: adding `branch: String` to `Token` must turn it red.

### 2.4 One workflow per workspace, with the door open

One workflow per workspace, as the owner chose. Five choices keep several
workflows possible later without a rewrite:

1. **Station ids are already namespaced.** Internally a `StationId` is
   `(workflow, station)`, with the one workflow named `main`. Ids written in
   a layout are short; `/` is reserved in them.
2. **Pools belong to the workspace, not the workflow.** They are declared in
   the workspace's limits (section 6) and stations refer to them by name, so
   two workflows would share them without a new rule.
3. **The log is per workspace, not per workflow.** One order of events covers
   every token, so moving a token between workflows would be one event.
4. **A token's location names its station, and nothing else names the
   workflow.** No token field stores "my workflow".
5. **Intake names its destination station.** Choosing a workflow at intake
   is a routing rule on the intake, not a token property.

To allow several workflows, what would change: `Publish` names which
workflow it replaces; `Start`, `Track` and intake rules name a destination
workflow (or a rule picks one by board column or label); `Send` may name a
station in another workflow; `workflow get` and the boards show one lane per
workflow; and the preview (7.5) reports per workflow. The reducer, the log,
fences, pools, holds and the cutover stay as they are.

## 3. The line between core and adapter

### 3.1 The core

The core is the pure reducer and its types: `step(&Pipeline, &Input) ->
Result<Transition, Refusal>`, with no clock and no I/O, as today
(`crates/workflow/src/lib.rs:1-2`). It knows:

- **Stations**: kinds, queues, order, priority, capacity, holds, drains.
- **Pools**: capacity shared by stations, per workspace, plus the host limit
  stamped into each input (section 6).
- **Tokens**: key, location, priority, brief, visits, dependencies,
  followers, models, fence, and an opaque data blob per adapter.
- **Batches**: forming by policy, travelling a run of stations, bisecting on
  failure, releasing members.
- **Routes and policies**: labels to stations; attempts, rework, returns,
  patience, `again`, skip, lot size.
- **Decisions**: deciders, how many verdicts are needed, verdicts given.
- **Outward steps and crossings**: what an adapter declares outward, the
  settle-before-move rule, and "never back across a crossing".
- **Fencing, stopping, the log, replay, crash recovery**: unchanged from the
  first pass.

### 3.2 What an adapter must implement

An adapter is two halves with one version string (`coding/v1`). The rules
half is pure and lives in the workflow crate, so replay runs it; a new
behavior is a new version, and old versions stay linked so old events replay.
The execution half lives in the daemon.

```rust
/// Pure. Linked into the reducer; chosen by the version string in the published workflow.
pub trait AdapterRules {
    fn version(&self) -> &'static str;
    /// Every operation: the station kinds it may run under, the labels it can answer, whether it is outward,
    /// the resources it holds while it runs, and what it needs and makes (for validation).
    fn catalog(&self) -> &[OpSpec];
    /// A layout's use of this adapter's operations, in order: today's coding.rs:204-280 order rules.
    fn validate(&self, layout: &Layout, config: &serde_json::Value) -> Result<(), Vec<Problem>>;
    /// Whether an operation may start for this token or batch now, and with what arguments and resources.
    /// Sees the token's visits, so it can accept a skipped station (5.1) as the orchestrator's choice.
    fn start(&self, op: &str, subject: &Subject, config: &serde_json::Value) -> Result<StartPlan, Refusal>;
    /// An answer becomes a label, new adapter data, and optionally "again": run this station once more with
    /// these facts (the CI rerun rule, today ci.rs). The core spends the `again` count before it emits.
    fn observe(&self, op: &str, subject: &Subject, outcome: &Outcome) -> Result<Applied, Refusal>;
    /// Which members of a batch a failure points at (paths changed versus failing tests, for coding).
    fn suspects(&self, batch: &Subject, outcome: &Outcome) -> Vec<String>;
    /// Fields the boards show, read from this adapter's data.
    fn project(&self, data: &Opaque) -> Vec<(String, String)>;
}
/// Effectful. In the daemon's runtime.
#[async_trait]
pub trait AdapterExec {
    async fn execute(&self, binding: &Binding) -> Result<Outcome>;       // command and agent operations
    async fn probe(&self, binding: &Binding) -> Result<Outcome>;         // waits: look once, answer or "not yet"
    async fn reconcile(&self, binding: &Binding) -> Result<Outcome>;     // after a crash, from receipts and journal
    /// Stop a running operation. For an outward operation it must answer `Settled` with the original's
    /// outcome (5.3); a plain `Stopped` for an outward operation is refused `Evidence`.
    async fn stop(&self, binding: &Binding, operation: &str) -> Result<Outcome>;
    /// Optional. Facts from outside that become inputs: new tokens (intake), verdicts on a decision station
    /// (GitHub reviews), each sent with an operation grant bound to its station.
    async fn sources(&self) -> Vec<SourceInput> { Vec::new() }
}
```

**Agent steps are a core runtime service**, not an adapter's. The agent
harness (Claude, Codex, their launch policy, models) moves out of
`CodingConfig::harness` (`workflow_config.rs:87`, `20-30`) into the
workspace's agent settings, because an assistant and a research workflow run
agents too. For an agent station the adapter only supplies the working folder,
the material the agent gets, and how its receipt becomes a label (`start`,
`observe`). For coding that is today's worktree, brief and verdict receipt.

**An outward operation must be reconcilable.** The adapter declares an
operation outward only if `reconcile` and `stop` can say from the outside
world whether it happened (the remote's ref for a push; the sent folder and a
message id for an email). An adapter that cannot (a plain shell command that
sends something) still runs, but every unknown outcome of it holds for a
`Settle` (Q5).

### 3.3 What the coding adapter owns

Everything below leaves the core and becomes `coding/v1`:

| Today, in the core | In `coding/v1` |
|---|---|
| `RunTree` (`state.rs:110-114`): worktree name, branch, path, base, adopted | Token data |
| `candidate`, `pushed` (`state.rs:34,54`) | Token and batch data. The core keeps only `Evidence::artifact` (an opaque id) and `Crossing` |
| `StageKind` coding verbs (`definition.rs:21-52`) | Operations: `coding.provision`, `.build`, `.check`, `.review`, `.route`, `.base`, `.fold`, `.gate`, `.group_review`, `.push`, `.ci`, `.push_branch`, `.open_pr`, `.pr_ready`, `.pr_reviews`, `.merged`, `.land` |
| `lands()` (`definition.rs:488`) | `outward: true` on `.push`, `.land`, `.merged` |
| `makes_candidate`, `needs_candidate` (`definition.rs:489-502`) and the order rules (`coding.rs:204-280`) | `catalog()` needs/makes and `validate()` |
| `Outcome::FoldConflict`, `Outcome::CiResult` (`state.rs:240-248`) | Adapter answers mapped by `observe` to labels `conflict`, `passed`, `red`, `cancelled`, with the conflict files and CI run in its data |
| CI rerun policy, `CiRun`, `CiRetry` (`ci.rs`) | `observe` returning `again`; its count is the core's `again` policy |
| "A push sends only a candidate gated on the same base" (`definition.rs:341-358`) | `start("coding.push")` refuses unless the batch's data says it was gated on the current base, or the gate visit ended `skipped` |
| The landing target's exclusive resource (`GroupDef::exclusive`, `definition.rs:91`) | `resources` of `.base` through `.ci` in the catalog: `coding:target:<ref>` |
| Path routing rules (`StageKind::Route`) | `coding.route`, labels from changed paths |
| Suspects from failing tests and changed paths | `suspects()` |
| `CodingConfig` (`workflow_config.rs:78-90`) minus the harness | The adapter's config in the workflow |

## 4. Stations

### 4.1 Five kinds

| Kind | Takes a place | Does | Answers | Examples |
|---|---|---|---|---|
| `agent` | Yes, in its pool | Starts an agent (model from the station's list, picked per token) in the folder the op's adapter prepares | A label from the agent's receipt, never its prose | Build, review, group review, triage, draft |
| `command` | Yes, in its pool | Runs one adapter operation | A label | Provision, fold, gate, push, send, publish |
| `decision` | No | Waits for verdicts from its deciders: named people, the orchestrator, or a group with a count (`needs: 2`). Verdicts come from the board, the CLI, or an adapter source | `approved`, `rejected`, or any label the deciders use | Approval, human code review, a release sign-off |
| `wait` | No | Asks an adapter's probe until it answers or `patience` runs out | The probe's label, or `timeout` | Base current, CI result, PR merged, a reply arrived |
| `batch` | Its inner stations do | Forms batches from its queue by policy, then carries each batch through its own inner stations | Each member's label: `passed`, `returned`, `crossed` | Landing group, weekly issue, daily digest |

A station's `op` names `<adapter>.<operation>`; the adapter's catalog says
which kinds each operation may run under. Labels route to stations;
`passed` with no route goes to the next station in the list, and any other
unrouted label holds the token (`HoldKind::StepFailed`), never closes it.
Inside a batch, an inner station may route to two batch outcomes:
`rejected` starts the batch's `on_reject` policy (4.5), and `returned` sends
the members the adapter names (a fold's conflicting tickets) out along the
batch station's `returned` route while the rest carry on.

### 4.2 Coding stations as configurations

| Station today | Kind | `op` | Notes |
|---|---|---|---|
| Provision | command | `coding.provision` | pool `work` |
| Build | agent | `coding.build` | pool `agents`, `models` list |
| Check (ticket) | command | `coding.check` | pool `work` |
| Review: agent / dispatch / person / orchestrator (`Reviewer`, `definition.rs:9-19`) | agent / command / decision / decision | `coding.review` for the first two | A person's or the orchestrator's review is a decision station with that decider |
| Approval (`Progression::AwaitApproval`) | decision | none | deciders from the step's principal |
| Route | command | `coding.route` | labels per path rule |
| Join, landing group (`StageKind::Join`, `GroupDef`) | batch | none | `form: { size, order }`, `on_reject: bisect` |
| AwaitBase | wait | `coding.base` | inside the batch |
| Fold | command | `coding.fold` | label `conflict` returns suspects |
| Cheap checks, gate (`Run`) | command | `coding.check`, `coding.gate` | |
| Group review (new, owner's Nov 15 suggestion) | agent | `coding.group_review` | one reviewer on the folded tree |
| Push, Land | command, outward | `coding.push`, `coding.land` | |
| AwaitCi, Ci | wait | `coding.ci` | `again` rule = today's rerun policy |
| PushBranch, OpenPr, PrReady | command | `coding.push_branch`, `.open_pr`, `.pr_ready` | |
| AwaitPr | wait, outward | `coding.merged` | outward: a person can merge at any time |
| Tracker | none | | a token `Resting` in a board column |

### 4.3 Queues, order, priority, capacity

Unchanged from the first pass:

- Each station's queue is an explicit list in the reducer state, in the log,
  replayed, shown by `workflow get` as it is.
- A token arriving at a station goes after every token of equal or higher
  priority; equal priority is first come, first served.
- `Prioritize` sets a token's priority, which follows it to every station.
  `Reorder` places it at an exact position in one queue until it leaves.
- A station starts from the head of its queue, skipping tokens whose
  dependencies aren't done and tokens still `Stopping` (5.2).
- A start needs room in the station, its pool, and the host limit if one is
  set (section 6). Starting is a reducer decision, so it is logged and
  replayed; the runtime runs what the reducer emitted and decides nothing.
- Decisions and waits take no place.

### 4.4 Policies

Station policy replaces every ticket-level limit. Defaults come from today's
numbers.

| Policy | Default | When reached |
|---|---|---|
| `attempts`: automatic re-runs of a failed effect | 2 (5 for waits) (`templates.rs:92,96`) | Hold `StepFailed` |
| `rework`: automatic send-backs from this station | 2 (`templates.rs:129`) | Hold `PolicyLimit` |
| `returns`: automatic returns from a batch, per token | 1 (`templates.rs:160`) | Hold `PolicyLimit` |
| `again`: adapter-requested re-runs (CI rerun) | today's `RerunFailed::times` (`ci.rs:22-25`) | The label as it came |
| `patience` (waits) | base 3 h, CI 3 h, merge 14 d | label `timeout` |
| `form` (batch): `size`, `window`, `order` | size 8 (`templates.rs:84`) | the next batch forms behind |
| `on_reject` (batch): `bisect`, `return_suspects`, `hold` | `bisect` | 4.5 |
| `skip`: who may skip this station | `orchestrator` | Q7: a workflow may set `person` |
| `capacity`, `pool` | per workspace limits (section 6) | |

A policy never closes a token. The worst it does is hold it with a reason
naming the policy; the orchestrator sends it on and the visit records
`overrode: "review.rework"`. Counts are read from visits: history, not a
budget.

### 4.5 Batches and bisecting

Kept from the first pass, now generic. A batch that comes back rejected stays
one batch: the adapter's `suspects()` marks the likely members (for coding,
changed paths that meet failing tests); under `bisect` the batch re-runs the
rejecting station on halves of its members, on the same inputs, until it has
found the members that fail; nothing outward runs while bisecting. The ones
found leave with the output as their brief (policy `returns`, then a hold);
the rest pass the station together and go on as one batch. For coding: an
8-ticket batch with one bad ticket costs five gates, one push and one CI run,
against seven gates, three pushes and three CI runs today.

## 5. Control: the orchestrator can do everything

### 5.1 Interventions

Every command names the token (or batch or station) and the station the
orchestrator believes it is at. That is the only staleness check: no run,
pin or fence on the wire. If the token has moved, the command is refused
`Stale` and nothing changes.

| Intervention | Command | What happens |
|---|---|---|
| Reorder a queue | `Reorder { station, token, position }` | Moves it within that queue |
| Change priority | `Prioritize { token, priority }` | Re-places it; applies at every later station |
| Hold, release | `Hold { token, reason }`, `Release { token, to? }` | A working token goes through `Stopping` first (5.2) |
| Pull from a batch, even mid-step | `Pull { token, reason }` | Leaves the batch and holds; a working batch stops and re-runs without it (5.2) |
| Send anywhere before where it is, any number of times | `Send { token, to, reason }` | Queues there by priority, brief = reason; through `Stopping` if working; never back across a crossing (5.3) |
| Run a step again | `Send { token, to: <same station> }` | No cap |
| **Skip any station** | `Skip { token or batch, station, reason }` | Leaves as `skipped` along its `passed` route, through `Stopping` if working. **This includes the gate before a push**: the reason is written in the visit, and the adapter's `start` sees the skip (3.3) |
| Decide on a person's behalf | `Skip` at the decision station | Same, unless the station's policy is `skip: person` (Q7) |
| Stop the running step | `Stop { token or batch, reason }` | `Stopping`, then `Held` |
| Cancel, reopen | `Cancel { token, reason }`, `Reopen` | Same key forever |
| Capacity | `SetCapacity { station or pool, n, reason }` | At the next start; nothing running stops. Up to the host limit (section 6) |
| Pause, drain a station | `HoldStation`, `ResumeStation`, `Drain` | Starts nothing new; what runs finishes |
| Publish a workflow | `Publish { layout, moves, reason }` | Section 7; the orchestrator's unless Q1 reserves it |
| Settle an unknown outward step | `Settle { token or batch, crossed, evidence }` | The orchestrator's, with evidence read from outside (Q5) |
| Resolve an uncertain step | `Resolve` | As today |
| Approve, review, decide | `Decide { token, station, verdict }` | By the station's deciders |

### 5.2 Stopping safely before anything replaces it

Kept from the first pass in full, with "lot" read as "batch". Every
intervention that takes a working token or batch off its station (`Hold`,
`Pull` from a working batch, `Send`, `Skip`, `Stop`, `Cancel`) makes one
transition, in the event that records the command:

1. **Into `Stopping { station, work, then }`.** `work.stopping` names the
   original effect; `work.run` becomes a `stop:` run, as `Cancel` does today
   (`reducer.rs:280-286`). `then` is where the command sends it. The fence is
   bumped, so a late answer from the original is refused.
2. **It keeps what it holds.** The original's binding, its place in the
   station's `working` set and pool, and its resources (today `owns_resource`,
   `reducer.rs:558-561`, keeping the `workflow_resources` row,
   `crates/store/src/workflow_effects.rs:19-40`). The original's outbox row
   is unchanged; an admitted original turns `uncertain` when it goes inactive
   (`store/src/workflow.rs:346-351`) and is reconciled as today.
3. **The stop.** It emits `EffectKind::Stop` only. The runtime keeps both
   current protections: a stop isn't claimed while its original still runs in
   this runtime (`workflow_runtime.rs:620-624`), and the adapter's `stop`
   answers `Stopped` only with a quiescence receipt
   (`workflow_runtime/coding_execute.rs:389-425` for coding).
4. **The destination waits.** `then: Queued { station }` puts the token in
   that queue at its place by priority, marked stopping; the station skips it
   when choosing what to start. A `Release`, `Resolve` or `Skip` that would
   start work on its tree, batch or resource is refused `Stopping`.
5. **Out of `Stopping`** only on `Observe` of the stop with
   `Outcome::Stopped` whose operation is `work.stopping` and whose receipt is
   valid (`reducer.rs:184-192`), in the same event that frees its place and
   resources. Outward operations never take this path (5.3).
6. **An ambiguous stop holds** `Held { Uncertain }`, keeping binding, place,
   resources and `then`, until a `Resolve` with a stopped or settled receipt.

**A crash at any point.** The command, the move to `Stopping` and the stop's
outbox row commit in one transaction (`store/src/workflow.rs:296-344`).
Before it, the token is working as it was; after it, recovery reconciles
admitted and uncertain effects first (`workflow_runtime.rs:590-619`), then
claims the stop, whose reconcile is idempotent. Nothing relies on the fence
alone.

### 5.3 Outward steps: settle first, never go back across

The first pass's landing rules, generalized. The core knows only that an
operation is outward (the adapter's catalog says so). For coding these are
`coding.push`, `coding.land` and `coding.merged`; for an assistant,
`mail.send`; for content, `cms.publish`; for a company, `deploy.production`.

**Rule 1: never back across a crossing.** A token or batch whose outward step
crossed (the core's `Crossing` is set) is never sent back before that station
or cancelled as if it hadn't happened; a further change is a `Reopen` under
the same key, which the adapter starts afresh (for coding, on the new base;
today's `Reopen`, `reducer.rs:56-80`). A batch that crossed can't be pulled
from (`reducer.rs:274-279`).

**Rule 2: settle before moving.** While an outward effect is `admitted` or
`uncertain` in the outbox, or the token waits at an outward wait (a merge a
person can make at any time), `Hold`, `Pull`, `Send`, `Skip`, `Stop` and
`Cancel` are accepted but only recorded:

1. The token or batch goes to `Settling { station, work, then }`, a
   `Stopping` whose stop must report the original's outcome. Membership,
   binding, place and resources are untouched; the fence is bumped.
2. The adapter's `stop` answers
   `Outcome::Settled { operation, original, receipt }`, where `original` is
   what `reconcile` found (`coding_execute.rs:416-435` already computes it
   for coding and only digests it into the receipt). A plain `Stopped` for an
   outward effect is refused `Evidence`.
3. **Crossed (known success):** the reducer records the crossing first,
   exactly as the original's own `Observe` would have; members stay; the
   token or batch follows its `passed` route. `then` is dropped and the
   request is recorded as overruled `Crossed`, with the receipt.
4. **Unknown:** `Held { Uncertain }` at the station, everything still owned,
   `then` kept in the hold. Only `Settle` releases it: `crossed: true`
   continues as in 3, `crossed: false` as in 5. Whether `Settle` is the
   orchestrator's (with evidence) or a person's is Q5.
5. **Proved not crossed** (the target refused the push, the mail server
   rejected the message, the pull request closed unmerged), or
   `Settle { crossed: false }`: only now does `then` apply, in the same event
   that frees place and resources.

A pending outward effect (never claimed) cannot have crossed: the command's
transaction suppresses it (`store/src/workflow.rs:346-351`), and 5.2 applies.

**Walk-through: a push that landed, its `Observe` lost.** Kept from the first
pass. Batch B7 (T1-T3) at `land.push`, push effect E admitted. The push lands
(`main` = `c7`); the daemon dies before `Observe(E)`. On restart E is
`uncertain`, no crossing recorded. The orchestrator sends `Pull { T2 }`,
`Cancel { T3 }`, `Send { T1, build }`: B7 goes to `Settling`, all three are
recorded, nothing moves. Another crash; recovery reconciles E first, then the
stop; `stop` answers `Settled { original: Labeled(passed, c7) }`. B7's
crossing is set to `c7`, T1-T3 stay members, B7 goes to `land.ci`, the three
requests are recorded as overruled `Crossed`. Had the remote refused the push,
T2 would leave, B7 refold without it, T3 close, T1 queue at build. Had the
remote been unreadable, B7 would hold for `Settle`.

**The same for an email.** Token M4 at `send`, `mail.send` admitted; the
daemon dies after the server accepted the message. The person, seeing a
wrong recipient in the draft, cancels M4. M4 goes to `Settling`; `mail/v1`'s
`stop` finds the message id in the sent folder; M4's crossing is recorded,
the cancel is overruled `Crossed`, and M4 goes to `await_reply`. The board
says the mail went out, so the person sends a correction instead of
believing it was stopped.

### 5.4 What still refuses, and why none is a count

| Refusal | Why it stays |
|---|---|
| `Stale`, `Duplicate` | The token moved, or the input was seen |
| `Invalid`, `Missing`, `Evidence`, `Definition` | Malformed input; `Evidence` also for a plain `Stopped` on an outward stop |
| `Unauthorized` | The grant doesn't match (`reducer.rs:590-596`) |
| `Crossed` | Rule 1 of 5.3: a send-back or cancel across a crossing |
| `Stopping` | Work on a tree, batch or resource whose last effect has no stop receipt yet (5.2) |
| `PersonOnly` | Only for what the owner keeps on the list in section 9; empty if every question is answered "no" |
| `Stranded` | A publish that would leave a token at no station (7.3) |

Every refusal today that exists because a ticket carries a budget is gone:
`Retry` past `MAX_RETRIES`, `SendBack` past `send_backs` or a back-edge limit,
rework past `max_reworks`, a routed back edge failing the run, `SendBack`
while a step runs, `Leave` from a group that has left, `Cancel` of a
follower, `Resource` while a resource is held (section 11.4 maps each).

## 6. Limits: per workspace, with an optional host limit

**Per workspace.** Each workspace declares its pools in its limits, outside
the workflow layout (so a later second workflow shares them, 2.4):

```yaml
limits:
  pools: { agents: 4, work: 2, landing: 1 }
```

Stations name a pool; each station also has its own capacity (default: its
pool's). Pools and capacities are in the reducer state, set by the publish
and changed by `SetCapacity`, which the orchestrator may send at any time.
With no host limit, a workspace's starts depend only on its own state:
workspaces are independent, and one never waits for another.

**The host limit is optional.** A user may set, per pool name, a host-wide
ceiling across all workspaces on this host (`farcooler host limits set
agents 6`):

```yaml
host_limits: { agents: 6 }        # unset by default
```

**How they compose: a start needs room at every level, so the tighter limit
always wins.**

- A token starts when its station, its workspace pool, and (if set) the host
  pool of that name all have a free place.
- A workspace can set its pool above the host limit; the host limit still
  binds. The host limit never raises a workspace's pool.
- The host's occupancy (what every other workspace holds) is stamped into
  each input by the store, as `busy_resources` is today (`state.rs:192-194`),
  so replay sees the same numbers. Starts are reducer decisions under the
  store's immediate transaction, so two workspaces can't both take the last
  host place.
- **Which workspace goes next under a host limit:** when a host place frees,
  the runtime sends `Advance` to the workspace whose queue head has waited
  longest (by the `now_ms` of its arrival), then the next. Each workspace's
  log records only its own start and the occupancy it saw, so replay is per
  workspace and exact. Starvation can't happen: the longest waiter always
  goes first.
- Lowering either limit stops nothing running; it only delays new starts.

**Who sets which.** A workspace's pools are its orchestrator's (by publish or
`SetCapacity`). The host limit is the user's, because it spans workspaces and
one workspace's orchestrator shouldn't take another's share (Q6).

**At the cutover** today's host-wide caps (4 agent, 2 landing, 2 work,
`places.rs:5-12`) become both each workspace's pools and the host limit, so
the first release behaves exactly as today. The owner can then clear the host
limit with one command.

## 7. Changing the process: no pinning, no interruption

A published change takes effect in the event that publishes it. There is one
workflow in force per pipeline; no token remembers an older one.

### 7.1 What a publish carries

```yaml
publish:
  layout: { ... }                 # the whole new workflow
  moves:                          # only for stations that are removed or renamed
    review: code-review           # renamed: tokens go to the same place in the new layout
    checks: { to: gate }          # removed: where its tokens go
  reason: "add group review before the gate"
  preview: <digest of the preview it was checked against>
```

The `Publish` event stores the whole layout and `moves` in the log (content
addressed, in the immutable `workflow_genesis`-style table of 10.2 if large),
so replay needs nothing outside the log.

### 7.2 Queued, held, resting and awaiting tokens

In the publish event:

- A token queued at a station that still exists stays in its queue at its
  place. If the station's policy changed (say `rework` lowered from 2 to 1),
  the new policy applies the next time it counts; nothing is held
  retroactively.
- A token queued or held at a removed or renamed station moves to the `moves`
  destination: into that queue at its place by priority and original arrival
  revision, or held there with its hold reason kept.
- A token awaiting a decision at a station whose deciders changed keeps its
  verdicts already given; new verdicts come from the new deciders, and `needs`
  is the new count.
- Resting tokens are unaffected.

### 7.3 A token mid-step

The step is not interrupted. The running effect continues under the
operation spec it started with (the effect's `op.spec` records what it ran);
that is a fact about running work, not a pin on the token.

- **Its station still exists:** when it answers, its label is routed by the
  **new** layout's routes. A changed operation (another model, another check
  command) applies at the next start. To use the new operation now, the
  orchestrator sends `Send { token, to: <same station> }`, which stops the old
  step safely (5.2) and queues it.
- **Its station was removed or renamed:** the station becomes **retired**:
  it stays in `Pipeline::retired` with its old routes and policy, takes no new
  arrivals, and starts nothing. The token finishes there. When it answers,
  its label is routed by the retired station's routes; a route to a station
  that no longer exists goes through `moves`. When the retired station is
  empty, it is deleted. The board shows it greyed with its working tokens, so
  nothing is hidden.
- **Its station is a decision or wait:** as a queued token (7.2): it moves to
  the `moves` destination at once, and a wait's probe effect is stopped
  (5.2), unless the wait is outward, in which case it settles first (5.3).
- A step stopping or settling at publish time (5.2, 5.3) keeps doing so; its
  `then` is mapped through `moves` if it names a removed station.

**Stranding is refused.** The publish is refused `Stranded` if any token,
batch, or `then` would end at a station that neither exists nor is in
`moves`. Rename is a `moves` entry; there is no other way to rename.

### 7.4 A batch mid-gate

A batch is a token for these rules, plus:

- **The station it is at still exists:** the gate finishes; its result is
  routed by the new layout. Stations added before it (the owner's group
  review) are not run for this batch; stations added after it are.
- **The batch station's inner stations were reordered or removed:** as 7.3,
  station by station; a retired inner station finishes its batch.
- **Batch formation changed** (size 8 to 4): applies to the next batch
  formed; a formed batch keeps its members.
- **The batch station itself was removed** (the workspace stops batching):
  `moves` must name where its members go, written `land: { unbatch: review }`.
  A forming batch's members go there at once. A working batch finishes its
  current inner station, then its members leave one by one to the
  destination, each with the batch's result in its visit. A batch that has
  crossed finishes its post-crossing stations first (5.3, rule 1).
- **A bisect in progress** finishes under the batch's current membership; a
  changed `on_reject` applies to the next rejection.

### 7.5 Who writes the mapping, and how a change is previewed

**Who writes `moves`:** whoever publishes (the orchestrator, or a person if
Q1 reserves it). `workflow preview` proposes it: a removed station's
default destination is the next surviving station on its `passed` route,
and a station whose id changed but whose kind and op did not is proposed as a
rename. The author accepts or edits it.

**Preview** is the reducer itself: `step` is pure, so
`preview(&Pipeline, &Publish) -> Report` applies the publish to a copy of the
live pipeline and diffs the result. Nothing is written. The report lists:

- every token and batch whose location changes, from and to;
- each queue before and after, in order;
- retired stations and the work finishing in them;
- what the new routes will do with each running step's possible labels;
- policies that will hold something at its next count;
- the adapter's `validate` problems;
- refusals (`Stranded`, adapter problems, capacity above the host limit).

`Publish` carries the preview's digest and the revision it was taken at. If
the pipeline moved since, the publish recomputes the preview inside the same
transaction and is refused only if the new result has a refusal; otherwise it
goes through and the event stores the final report. The orchestrator's
playbook: preview, read, publish.

## 8. Walkthroughs

Each is a complete workflow in the schema-3 definition format, shown as
YAML (the stored form is JSON). Unlisted `passed` routes go to the next
station.

### 8.1 Far Cooler: fully agentic

```yaml
schema: 3
name: agentic-batched-push
adapters:
  coding: { version: coding/v1, config: { repository: ., target: main, ci: ci.yml, checks: [...] } }
limits: { pools: { agents: 4, work: 2, landing: 1 } }
stations:
  - { id: provision, kind: command, op: coding.provision, pool: work }
  - { id: build, kind: agent, op: coding.build, pool: agents,
      models: [claude:opus, codex:gpt-5.5, claude:sonnet], policy: { attempts: 2 } }
  - { id: review, kind: agent, op: coding.review, pool: agents, models: [claude:opus, codex:gpt-5.5],
      routes: { rejected: build }, policy: { rework: 2 } }
  - { id: route, kind: command, op: coding.route, pool: work,
      routes: { passed: land, needs_review: decide } }
  - { id: decide, kind: decision, deciders: [orchestrator], routes: { approved: land, rejected: build } }
  - id: land
    kind: batch
    pool: landing
    form: { size: 8, order: priority }
    on_reject: bisect
    policy: { returns: 1 }
    routes: { returned: build }
    stations:
      - { id: base, kind: wait, op: coding.base, policy: { patience: 3h } }
      - { id: fold, kind: command, op: coding.fold, routes: { conflict: returned } }
      - { id: checks, kind: command, op: coding.check, routes: { red: rejected } }
      - { id: group-review, kind: agent, op: coding.group_review, models: [claude:opus],
          routes: { rejected: rejected } }
      - { id: gate, kind: command, op: coding.gate, routes: { red: rejected } }
      - { id: push, kind: command, op: coding.push }          # outward
      - { id: ci, kind: wait, op: coding.ci, policy: { patience: 3h, again: 1 } }   # red: unrouted, holds
```

| Station | Who decides |
|---|---|
| review | An agent's receipt; two rejections hold for the orchestrator |
| decide | The orchestrator, for changes the route rule flags (security paths, migrations) |
| land.gate | The gate command; the orchestrator may skip it with a reason |
| land.ci red | Holds for the orchestrator after one adapter rerun; the batch has crossed, so a fix is a new ticket or a `Reopen` |

Every skip, send, hold and capacity change is the orchestrator's. No station
needs a person.

### 8.2 A company with human pull request review

Agents build; people review on GitHub; two approvals are required, one from
code owners; a person merges; a release manager signs off production.

```yaml
schema: 3
name: company-human-review
adapters:
  coding: { version: coding/v1, config: { repository: ., pull_request: { repository: acme/app, base: main } } }
  deploy: { version: command/v1, config: { staging: [./deploy, staging], production: [./deploy, production] } }
limits: { pools: { agents: 2, work: 2 } }
stations:
  - { id: provision, kind: command, op: coding.provision, pool: work }
  - { id: build, kind: agent, op: coding.build, pool: agents, models: [claude:opus] }
  - { id: self-review, kind: agent, op: coding.review, pool: agents, routes: { rejected: build },
      policy: { rework: 2 } }
  - { id: open-pr, kind: command, op: coding.open_pr, pool: work }      # push_branch + open + ready
  - { id: pr-ci, kind: wait, op: coding.ci, policy: { patience: 2h }, routes: { red: build } }
  - id: code-review
    kind: decision
    deciders: [group:codeowners, group:engineers]
    needs: { total: 2, from: { group:codeowners: 1 } }
    source: coding.pr_reviews                  # GitHub reviews arrive as verdicts
    routes: { approved: merged, changes_requested: build }
    policy: { skip: person }                   # Q7: only a person may waive review here
  - { id: merged, kind: wait, op: coding.merged, policy: { patience: 14d } }   # outward; a person merges
  - { id: main-ci, kind: wait, op: coding.ci, policy: { patience: 2h } }
  - { id: staging, kind: command, op: deploy.staging, pool: work }
  - id: release
    kind: decision
    deciders: [person:release-manager]
    routes: { approved: production }           # rejected: unrouted, holds
    policy: { skip: person }
  - { id: production, kind: command, op: deploy.production, pool: work }       # outward
```

| Station | Who decides |
|---|---|
| self-review | An agent; the orchestrator overrides past two rejections |
| code-review | People on GitHub: two approvals, one from a code owner. `changes_requested` sends the ticket back to build with the review comments as its brief. Skipping is a person's (`skip: person`) |
| merged | A person merges on GitHub; outward, so interventions settle first (5.3) |
| release | The release manager. Skipping is a person's |

Branch protection on GitHub still enforces the approvals. The engine's
station mirrors them so the board shows where each ticket waits and who it
waits on. `deploy` uses a generic `command/v1` adapter: `production` is
declared outward, so the deploy command must accept an idempotency key and
answer a reconcile query (3.2); otherwise its unknown outcomes hold for
`Settle`.

### 8.3 A personal assistant: inbox triage, draft, approve, send

Tokens are mail threads, created by intake. A daily digest batches what needs
the person's attention but no reply.

```yaml
schema: 3
name: assistant-inbox
adapters:
  mail: { version: mail/v1, config: { account: me@example.com, folder: INBOX } }
  calendar: { version: calendar/v1, config: { calendar: primary } }
limits: { pools: { agents: 2, work: 2 } }
intake:
  - { op: mail.inbox, filter: "is:unread -category:promotions", to: triage }
stations:
  - id: triage
    kind: agent
    op: mail.triage
    pool: agents
    models: [claude:sonnet]
    routes: { reply: draft, schedule: propose-times, fyi: digest, ignore: archive }
  - { id: propose-times, kind: command, op: calendar.free_slots, pool: work }
  - { id: draft, kind: agent, op: mail.draft, pool: agents, models: [claude:opus] }
  - id: approve
    kind: decision
    deciders: [person:me]
    routes: { approved: send, rejected: draft, edited: send }
    policy: { skip: orchestrator }              # the orchestrator may send routine replies for me
  - { id: send, kind: command, op: mail.send, pool: work }          # outward
  - { id: await-reply, kind: wait, op: mail.reply, policy: { patience: 3d },
      routes: { replied: triage, timeout: follow-up } }
  - { id: follow-up, kind: agent, op: mail.draft_follow_up, pool: agents, routes: { passed: approve } }
  - { id: archive, kind: command, op: mail.archive, pool: work }
  - id: digest
    kind: batch
    form: { window: "daily 08:00" }
    stations:
      - { id: summarize, kind: agent, op: mail.summarize, models: [claude:sonnet] }
      - { id: deliver, kind: command, op: mail.send_digest }       # outward
```

| Station | Who decides |
|---|---|
| triage | An agent's label |
| approve | The person, by default. The orchestrator may approve on their behalf with a reason (a reply confirming a meeting time it proposed), because `skip: orchestrator`. A stricter person sets `skip: person` |
| send, deliver | Outward: an unknown result settles from the sent folder (5.3) |

What this asks of the core, and gets: intake (tokens created by an adapter),
two adapters in one workflow, a batch formed by a time window rather than a
size, a wait with patience routing to a follow-up, and an outward step that
isn't code. A window batch forms on the runtime's `Advance` tick, whose
`now_ms` is in the input, so replay forms the same batch.

### 8.4 Research and content: a weekly brief

Topics come from a board column; each becomes an article; articles for the
week publish together as one issue.

```yaml
schema: 3
name: weekly-research-brief
adapters:
  files: { version: files/v1, config: { root: content/ } }
  cms: { version: cms/v1, config: { site: blog, list: weekly } }
limits: { pools: { agents: 3, work: 1 } }
intake:
  - { op: files.board_column, column: "Topics", to: research }
stations:
  - { id: research, kind: agent, op: files.research, pool: agents, models: [claude:opus],
      policy: { attempts: 2 } }
  - { id: outline, kind: agent, op: files.outline, pool: agents }
  - id: pick
    kind: decision
    deciders: [orchestrator]
    routes: { approved: write, rejected: research, drop: archive }
  - { id: write, kind: agent, op: files.write, pool: agents, models: [claude:opus] }
  - { id: fact-check, kind: agent, op: files.fact_check, pool: agents, models: [codex:gpt-5.5],
      routes: { rejected: write }, policy: { rework: 2 } }
  - id: issue
    kind: batch
    form: { window: "weekly fri 09:00", size: 6, order: priority }
    on_reject: return_suspects
    routes: { returned: write }
    stations:
      - { id: assemble, kind: command, op: cms.assemble }
      - { id: edit, kind: agent, op: cms.edit_issue, models: [claude:opus] }
      - { id: editor, kind: decision, deciders: [person:editor, orchestrator], needs: 1 }
      - { id: publish, kind: command, op: cms.publish }      # outward
      - { id: measure, kind: wait, op: cms.stats, policy: { patience: 7d } }
  - { id: archive, kind: command, op: files.archive }
```

| Station | Who decides |
|---|---|
| pick | The orchestrator, on the outline |
| fact-check | An agent of a different model from the writer's; two rejections hold |
| issue.editor | The editor or the orchestrator, whichever answers first (`needs: 1`) |
| issue.publish | Outward: a lost publish settles from the CMS before any pull or cancel |

`return_suspects` uses the adapter's `suspects()` (`cms/v1` names the
articles the editor marked) to send those back to `write` while the rest of
the issue publishes.

### 8.5 What the walkthroughs changed in the model

Writing these forced four changes, each now in sections 2-4: intake (8.3,
8.4); several adapters per workflow (8.3); time-window batch formation (8.3,
8.4); and decisions with counts and groups, fed by an adapter source (8.2).
Nothing in the four layouts needed a core concept named after code.

## 9. Every action still reserved to a person

The engine rule is that the orchestrator may do anything above, including
skipping any station, with a recorded reason. These are the only exceptions
this design keeps. Each is a question for the owner.

**Q1. Publishing a workflow.** *Reason to keep it:* a workflow decides what
every later token goes through, including which stations a person must
decide (Q7); an orchestrator that can publish can remove those. *Reason to
drop it:* the owner wants the orchestrator to run the process, and preview
(7.5) makes a change's effect exact before it applies.
*Recommendation:* drop it by default; offer a workspace setting
`publish: person`, itself set only by a person, so a company can keep it.
Every publish records its reason and preview.

**Q2. Writing to the secret store.** *Reason:* a secret is the one input the
orchestrator can neither verify nor take back once it's been used or shown;
agents' transcripts and logs are where secrets leak. *Recommendation:* keep;
the orchestrator may ask on the board and a person enters it once. Reading a
secret for a step stays the runtime's, as today.

**Q3. Security flags.** This design reads "security flags" as: widening a
permission mode (an agent running with permissions skipped, a sandbox
exception, a network allowance), and clearing a hold that a security review
placed. *Reason:* these are the orchestrator granting power to itself or its
agents. *Recommendation:* keep widening a person's; narrowing is anyone's.
Clearing a security hold: the orchestrator's, if a second review of a
different model agrees; otherwise a person's. The owner may mean something
else by the term; if so, say which.

**Q4. Production deploys.** Far Cooler has none (its releases are the app
update and the relay deploy, which standing rules already let the
orchestrator do). *Reason:* a deploy is outward and reaches customers.
*Recommendation:* in a template that deploys (8.2), the deploy's decision
station has a person as decider and `skip: person` by default; a workspace
that trusts its orchestrator changes it by publishing.

**Q5. Settling an unknown outward step.** Today `Settle` is person-only
(`workflow_runtime/admission.rs:30`). *Reason to keep:* a wrong settle either
lands twice or loses a landing. *Reason to drop:* the orchestrator can read
the same evidence a person would (the remote's ref, the sent folder).
*Recommendation:* the orchestrator settles, but only by attaching evidence it
read outside the engine (the ref value, the message id), which the event
records; a person can always settle too.

**Q6. Grants and roles, and the host limit.** *Reason:* grants are where the
orchestrator's authority comes from, so it can't grant itself more; the host
limit spans workspaces, and workspaces are independent, so one workspace's
orchestrator shouldn't change what the others may use. *Recommendation:*
keep both a person's (the host limit, the user's).

**Q7. Station reservations.** May a workflow mark a station `skip: person`,
so that only a person may skip it or decide it? *Reason:* a company with
compliance rules (two human approvals) can't run on the engine otherwise.
*Recommendation:* yes, as opt-in station policy; never set in the Far Cooler
templates, set on code review and production deploy in the company template.
Under Q1's recommendation the orchestrator could publish the reservation
away, unless the workspace also sets `publish: person`; the company template
sets both.

Not on this list, because they are the orchestrator's: skipping any station
in a workflow without reservations, including a gate before a push or CI;
sending back any number of times; capacity within the host limit; holding,
draining and pausing stations; resolving an uncertain step from a receipt;
cancelling and reopening.

## 10. Cutover

One engine at a time, always. The new reducer lands unwired; one cutover step
switches the runtime, migrates every workspace in one transaction, and
deletes the old reducer. Each step lands on its own and leaves main working.

1. **Core and coding rules, pure and unwired.** `crates/workflow/src/pipeline/`
   with the core types of 2.3, `step`, schema 3 and the `AdapterRules`
   trait; `crates/workflow/src/adapters/coding.rs` implementing it with
   today's rules moved, not rewritten (order rules from `coding.rs:204-280`,
   candidate rules from `definition.rs:489-502`, CI rerun from `ci.rs`). The
   four coding templates (`templates.rs:109-192`) become layouts. Two
   fixture adapters, `test/mail` and `test/cms`, exist only in tests, and
   every walkthrough of section 8 runs as a reducer test, so the core is
   proved neutral before anything is wired. The core-vocabulary check of 2.3
   lands here.
2. **`migrate`, pure, with a refusal list**, as in the first pass:
   `migrate(&State, &[Input], &[Dispatch], &Occupancy) -> Result<Pipeline, Vec<Quarantine>>`,
   reading the old state, the old log (for queue order), each pending
   effect's dispatch status (`workflow_outbox.dispatch_state`,
   `store/src/workflow.rs:187-188`) and other workspaces' occupancy. It moves
   each item's coding fields into the token's `coding` data (11.1). Also adds
   the snapshot table of 10.2, unused. Tested on fixtures for every phase and
   dispatch status, and on copies of the live store through the existing
   rehearsal (`crates/store/src/engine_cutover/rehearsal.rs`) with the cases
   of 10.3.
3. **The cutover**, in one landing:
   - the runtime calls the new `step`; `places.rs` and `launch_ready`'s pool
     check (`workflow_runtime.rs:625-632`) go, as the reducer now decides
     starts;
   - the runtime's coding code (`coding_execute.rs`, `coding_landing.rs`,
     `coding_pr.rs`, `coding_settle.rs`, `coding_verbs.rs`) implements
     `AdapterExec`; `stop` returns `Settled` for outward operations;
   - `AdapterConfiguration` (`workflow_config.rs:97`) becomes a map of named
     adapters; `CodingHarness` moves to workspace agent settings;
   - wire actions: the commands of 5.1, each naming a token and its expected
     station; `run` and `pin` leave the wire;
   - limits: each workspace's pools and the host limit are set from today's
     caps (section 6);
   - at startup, after the old runtime is quiesced (`engine_quiesce`,
     `engine_cutover.rs:403-417`) and before any effect is claimed, the store
     migrates every workspace in one immediate transaction under a new
     cutover incarnation (`engine_allowed`, `engine_cutover.rs:119`): writes
     each snapshot and `Genesis`, suppresses the outbox rows of work that
     never started (10.1), and stores schema 3. Anything unmappable is held
     `Quarantined`, never dropped (`engine_cutover.rs:248-306`). The same
     transaction replays every workspace from its snapshot and commits only
     if each replay equals what it is about to store;
   - the old reducer, `Item`, `Phase`, `StageKind` and the group machinery
     are deleted;
   - `workflow get` keeps the fields today's apps read, projected from the
     pipeline and the coding adapter's `project()`.
4. **Apps and the orchestrator skill.** Boards draw stations, queues, holds,
   retired stations and the preview; the manager skill uses `Reorder`,
   `Hold`, `Pull`, `Send`, `Skip`, `preview` and `Publish`. The legacy
   projection goes.
5. **Join old continuation keys** (ov-1195 → ov-1204 → ov-1208, ov-1170 →
   ov-1214, ov-1143 → ov-1221) with a `continues` link on the board.
6. **After the cutover, not part of it:** a generic `command/v1` adapter and
   the assistant and content templates, as their own tickets.

### 10.1 How work in flight migrates

Kept from the first pass. Station ids are the template step ids (`provision`,
`build`, `gate-N`, `route`, `approve`, `branch`, `pr`, `merged`, `ci`), and a
group's steps become the batch's inner stations (`land.base`, `land.fold`,
`land.gate-0`, `land.push`, `land.ci`). An item whose step id has no station
is held `Quarantined`. Every item lands on the current published layout,
whatever pin it ran under: there is no pinning after the cutover. A running
effect whose operation differs from the current layout's is treated as
mid-step under 7.3 (it finishes; its label routes by the current layout).

`Running` is split by its pending effect's outbox row, never by phase alone:

| `dispatch_state` | Token after | Outbox row |
|---|---|---|
| `admitted` | `Working`, same `run`, `pending`, `fence`; counts in its station and pool | Unchanged; reconciled as today (`workflow_runtime.rs:592-619`) |
| `uncertain` | `Working { uncertain }`, `Held { Uncertain }`; counts in its station and pool | Unchanged; waits for `Resolve` |
| `pending` | `Queued` at its station, no `Work` | `suppressed` in the cutover transaction |
| `claimed` | `Queued`, no `Work`: the runtime admits before any adapter call (`workflow_runtime.rs:652-671`), and the old claim can't be admitted under the new incarnation (`workflow_effects.rs:120`) | `suppressed` |
| `settled`, `suppressed`, or no row | `Held { Quarantined }`, both recorded | Unchanged |

Admitted work keeps its effect id, run, fence and resource generation, so its
`Observe` still binds (`store/src/workflow.rs:271-280`). Work that never
started gets `fence + 1` and a new effect when its station reaches it: one
token, one live effect. Queue order is the outbox order today's runtime would
have launched in (`store/src/workflow.rs:162`), then priority from the first
command after. `Genesis` emits no effects. If admitted work exceeds a pool,
all of it stays `Working`; nothing new starts until it's back under.

Other phases map as in 11.2. `Cancelling` becomes `Stopping { then: Closed }`
(or `Settling` at an outward station) with its stop effect kept whatever its
dispatch status.

### 10.2 Replay across the cutover

Kept from the first pass. The cutover stores the migrated state in a table
nothing can change: `workflow_genesis(workspace PRIMARY KEY, revision, epoch,
schema, snapshot, snapshot_hash, legacy_through, legacy_chain, migrator)`,
with the no-update and no-delete triggers of `workflow_events`
(`store/src/workflow.rs:198-201`). The `Genesis` event at
`legacy_through + 1` carries `snapshot_hash`, `legacy_chain` and the stamped
pools, host limit and occupancy. `Pipeline::replay(&Genesis, &[Input])` loads
the snapshot, checks its hash against the bytes and the event, then folds
every later event through the new `step` and the adapter rules versions
named in the workflow; it reads nothing else. Published layouts after the
cutover are in their `Publish` events (7.1). Old events stay as read-only
history; `legacy_chain` proves the snapshot was cut from exactly them. The
replay-from-empty validation of the first pass (build an old store covering
every phase, a batch mid-gate, a `Cancelling` ticket and the twelve-build
case; cut over; apply new inputs including a `Publish` that retires a
station with a token working in it; delete every `workflow_states` row;
replay from genesis with a test target that doesn't link the old reducer;
compare byte for byte, including emitted effects against outbox rows) runs
in step 2, inside the cutover transaction in step 3, and afterwards as
`farcooler workflow verify`.

### 10.3 Rehearsal: twelve builds, four places

Kept from the first pass, with its limits renamed.

- **Before.** One workspace, build pool 4. Twelve tickets `Running` at
  build: A1-A4 `admitted`, P1-P8 `pending` in that outbox order; P6 and P7
  are speed tickets. A second workspace holds one admitted build. Host limit
  `agents: 5`.
- **Cutover.** Build `working` is exactly A1-A4 with their original effect
  ids, runs, fences and resource generations, outbox rows still `admitted`.
  The build queue is P1-P8 in order; their rows are `suppressed`, a claim
  returns nothing, an `Observe` is refused. `Genesis` emitted nothing; the
  host pool is 5 of 5.
- **The workspace pool binds.** `Prioritize { P6, 10 }`,
  `Prioritize { P7, 10 }`, then `Observe` A2 passed: exactly one new build
  effect, for P6, new id, fence `old + 1`; `working` A1, A3, A4, P6 (4 of 4);
  queue P7, P1-P5, P8; host 5 of 5. The other workspace's build ends (host 4
  of 5): nothing is emitted and P7 is still first, because the workspace pool
  is full and section 6 needs room at every level. `Observe` A1 passed: one
  effect, for P7. A second run replaces that `Observe` with a logged
  `SetCapacity { build, 5 }`: P7 starts on that event.
- **The host limit binds.** From the same cutover state, a logged
  `SetCapacity { build, 6 }` first: it emits nothing (host 5 of 5) although
  the station has two free places. `Observe` A2 passed: only P6 starts (host
  5 of 5). The other workspace's build ends: P7 starts; P1 still waits with a
  station place free.
- **No host limit.** From the same cutover state with the host limit cleared
  by a person: the other workspace's build no longer counts; `Observe` A2
  passed starts P6 and nothing more (workspace pool 4).
- **Crash** inside the cutover transaction leaves the old store under the old
  incarnation; after commit, before any claim, restart reconciles A1-A4 first
  and no P effect exists.
- **A push that landed, its `Observe` lost**: the walk-through of 5.3, run as
  written, plus the refused-push and unreadable-remote variants.
- **A publish mid-flight:** after the cutover, publish a layout that adds
  `group-review` before `land.gate` and removes `land.checks` with
  `moves: { land.checks: land.group-review }`, while one batch is working at
  `land.checks` and another is queued for it. Assert: the working batch
  finishes at the retired `land.checks` and its `passed` goes to
  `group-review`; the queued batch moves to `group-review`'s queue at once;
  the retired station is deleted when empty; the preview taken before
  publishing equals the state after.
- **Replay** from empty holds at each point.

## 11. Mapping: every Item field, Phase, Command and Refusal

### 11.1 `Item` fields (`state.rs:21-103`)

| Field | In the new model | Why |
|---|---|---|
| `definition` | Removed from the token; `Pipeline::workflow` | The process belongs to the pipeline; no pinning |
| `pin` | `Published::digest` | One workflow in force |
| `policy_pin` | Removed; each effect's `op.spec` records what it ran | A record of running work, not a pin |
| `dependencies` | `Token::dependencies` | |
| `resource` | Removed; an operation's `resources` in the adapter catalog | Resources are the adapter's names, held by the core |
| `stage` | `Location`'s station | |
| `iteration` | Derived from `visits` | History, not a budget |
| `attempt` | `Work::attempt` | |
| `fence` | `Token::fence`, `Batch::fence` | Fencing |
| `phase` | `Location` (11.2) | |
| `run` | `Work::run`, `Decision::run` | Binds answers |
| `pending` | `Work::pending` | |
| `candidate` | Coding data `candidate`; the core keeps `Evidence::artifact` | A commit is coding's |
| `last_evidence` | `Token::evidence` | |
| `stopping` | `Work::stopping` | |
| `rework_history` | `visits` ending `rejected` | One history |
| `back_edges` | Derived from `visits`, counted by policy | Limits become policy |
| `retries` | Derived from `visits`; no cap | |
| `members` | `Batch::members` | |
| `group_of` | Removed | A batch is not an item with a definition |
| `pushed` | `Batch::crossing` (core) plus coding data `pushed` (the commit) | Crossing is core; what crossed is the adapter's |
| `handed` | Archived batch's members | |
| `since_ms` | Derived from the last visit and the log's `now_ms` | |
| `grouped_in` | `Location::InBatch` | |
| `wait_since` | `Work::wait_since` | Patience survives restarts |
| `models` | `Token::models` | Agent steps are core |
| `tree` | Coding data `tree` (`RunTree`) | A worktree and branch are coding's |
| `sent_back` | `visits` ending `sent_back` | |
| `brief` | `Token::brief` | |
| `follows` | `Location::Riding` | |
| `followers` | `Token::followers` | |
| `cancel_reason` | Archive entry's reason | |
| `failure` | `Token::failure`, and the hold's reason | |
| `ci_reruns` | Derived from `visits` ending `again` at the CI station | `again` policy |
| `ci_retry` | `Work::again` holding coding's `CiRetry` | Binds the rerun to its run |

`State`: `schema` 3; `workspace`, `revision`, `epoch`, `seen`,
`source_sequences`, `archive` unchanged; `items` splits into `tokens` and
`batches`; `workflow`, `stations`, `retired` and `pools` are new.
`MAX_RETRIES`, `DEFAULT_SEND_BACKS`, `MAX_SEND_BACKS`, `MAX_BACK_EDGE_LIMIT`
and `max_reworks` become policy defaults with no cap behind them; `MAX_ITEMS`
bounds tokens plus batches.

### 11.2 `Phase` (`state.rs:15-19`)

| Phase | In the new model | Why |
|---|---|---|
| `Blocked` | `Queued` | A queue with an order |
| `WaitingApproval` | `Awaiting` at a decision station; `Resting` for a tracked ticket | |
| `WaitingReview` | `Awaiting` at a decision station | Reviews by people or the orchestrator are decisions |
| `Running` | `Working`, or `Queued` by dispatch status (10.1) | |
| `Uncertain` | `Working { uncertain }` plus `Held { Uncertain }` | |
| `Cancelling` | `Stopping { then: Closed }`, or `Settling` at an outward station | 5.2, 5.3 |
| `Done` | `Closed { crossed }`, archived | |
| `Failed` | Removed: `Held { StepFailed }` or `Held { PolicyLimit }` | Nothing dies of a limit |
| `Cancelled` | `Closed { crossed: false }`, archived | |
| `WaitingJoin` | `Queued` at the batch station | |
| `Joined` | `InBatch` | |

### 11.3 `Command` (`state.rs:197-230`) and wire `Action` (`crates/protocol/src/workflow_commands.rs:16-60`)

| Today | In the new model | Why |
|---|---|---|
| `Advance` | `Advance` | The tick for occupancy and time windows |
| `Enqueue` | `Admit { token, station }` | Fixtures |
| `Approve`, `Review` | `Decide { token, station, verdict }` | One decision kind |
| `Observe` | `Observe` | Effect, run, fence; outcomes generic (2.3) |
| `Cancel` | `Cancel`; `Stop` keeps the token | |
| `Retry` | `Send { to: same station }` | No cap |
| `Queue` | Removed: arriving at a batch station | |
| `Leave` | `Pull` | Works mid-step too |
| `Move`, `Start`, `Resolve`, `Reopen` | Unchanged names | |
| `SendBack` | `Send { to, reason }` | Any station, any number of times |
| (wire) `Get` | `Get`, with stations, queues, retired stations | |
| (wire) `Publish` | `Publish { layout, moves, reason, preview }`; orchestrator's unless Q1 | Section 7 |
| (wire) `Propose`, `Drafts` | `Preview`, plus drafts for people who want to review a change | 7.5 |
| (wire) `Settle` | `Settle { crossed, evidence }`; orchestrator's with evidence unless Q5 | |
| (wire) `Track` | `Track`, and adapter intake | |
| New | `Reorder`, `Prioritize`, `Hold`, `Release`, `Pull`, `Skip`, `Stop`, `SetCapacity`, `HoldStation`, `ResumeStation`, `Drain`, `Preview` | Section 5 |

### 11.4 `Refusal` (`state.rs:7-12`)

| Refusal | In the new model | Why |
|---|---|---|
| `Definition`, `Invalid`, `Duplicate`, `Unauthorized`, `Missing`, `Evidence` | Kept | Malformed or unauthorized input |
| `Stale` | Kept; also "the token isn't at the station you named" | The one staleness check |
| `Blocked` | Removed | Never produced today |
| `Resource` | Removed | Work queues instead |
| `Phase` | Narrowed to `Crossed` | Only crossings refuse by position (5.3) |
| `Limit` | Removed as a token refusal; kept for `MAX_ITEMS` | Limits are policy |
| `SendBackRefused` | Removed | `Send` is never refused by a count |
| New: `Stopping` | Work on something whose last effect hasn't proved it stopped | Safety (5.2) |
| New: `Stranded` | A publish that leaves a token nowhere | 7.3 |
| New: `PersonOnly` | Only what the owner keeps of Q1-Q7 | Section 9 |
