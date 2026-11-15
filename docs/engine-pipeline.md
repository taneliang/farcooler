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
is routed by the new layout. A renamed station keeps its work under the new
name; a removed one is named in the change's **moves** with the station that
replaces it, and a step that finishes there then enters that replacement, so
the new process runs in full. Work that has already crossed a once-only
boundary (a push) never re-enters it: it resumes at the station after that
boundary. If the new layout ends at the boundary, crossed work closes as
done, as fresh work would; the preview lists any that would close without
running a station it would have run before (say CI), and the author may hold
it instead. Only when no single continuation can be named does crossed work
hold for the orchestrator. Cases the design leaves open hold the same way. The publish is refused while anything
would be stranded.
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
| Q1 | **Publishing a workflow** | **Answered (owner, Nov 15): no.** The workspace's orchestrator publishes, with a reason. The owner can reserve it to a person per workspace |
| Q2 | **Writing to the secret store** | Yes. A secret is the one input the orchestrator can't check and can leak; a person types it once |
| Q3 | **Security flags** (granting the orchestrator or an agent a wider permission mode, sandbox exceptions, clearing a security hold) | Yes, for widening; no for narrowing |
| Q4 | **Production deploys** | Not in Far Cooler (it has none). In a template that deploys, the deploy decision is a person's by default; the workspace can hand it to the orchestrator |
| Q5 | **Settling an outward step whose result is unknown** (a push or a send that may or may not have happened) | No: the orchestrator settles, but only with evidence it read from the outside world, recorded. "Crossed" needs positive evidence of that round; "not crossed" needs an authoritative rejection and proof nothing is still in flight. Not finding it is never enough |
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
| The landing rules (never go back across a push; settle before moving) are generalized to any **outward** step. A push stays `repeat: once`; a sent email is `repeat: fresh`, so a second reply or a follow-up is a new round with fresh evidence and a fresh idempotency key, and no crossing is ever undone. Crossings and rounds belong to the operation's boundary in the world, not the station, so renaming or replacing a station can't reset them; an unknown outcome is settled only by positive evidence or a proved rejection, never by absence (sections 3.2, 5.3, 7.3) | Follows from the non-engineering intent |
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
  a deploy. The adapter declares which of its operations are outward, and
  whether each may cross once (a push) or again in fresh rounds (an email);
  the core enforces the rules for them (5.3).
- **Intake**: an adapter source that creates tokens, such as new mail threads
  or a research topic list.

### 2.2 What the core owns, what the token carries, what the adapter owns

| The pipeline (core) owns | A token carries (core) | The adapter owns, per token or batch |
|---|---|---|
| The published workflow and its digest | Its key, forever | Its state blob: for coding, the tree, branch, base, candidate commit, pushed commit, PR number, CI run and rerun counts |
| Each station's queue, working set, capacity, hold, drain | Where it is (`Location`) | What an operation needs and produces |
| Pools and their capacity | Its priority | How an outcome becomes a label (a fold conflict is label `conflict`) |
| Batches: member keys, location, crossing record, bisect progress | Its brief and last failure | Which members are suspects when a batch fails; what each member joined with (coding: its candidate) |
| The event log, idempotence, epoch | Its visits (all history and overrides) | Board fields to show (branch, PR link, recipient) |
| Fences and stop receipts | Its dependencies, followers, picked models | Execution, reconcile, stop, probe |
| Holds and policy counts; the archive of closed tokens and batches | Its crossings, one per outward operation that took something out | Validation of a layout's use of its operations; what an effect needs to run (coding: tree, candidates, CI run) |

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
/// A station a publish removed while work was still running in it: only its translation, never its old routes.
pub struct Retired { pub kind: Kind, pub working: BTreeSet<String>, pub to: StationId,
    pub labels: BTreeMap<String, Target> }
pub enum Target { Station(StationId), Done, Finish, Rejected, Returned }   // 4.1
pub struct Pool { pub capacity: u16, pub held_by: BTreeSet<String> }
pub struct Token {
    pub at: Location,
    pub priority: i32,
    pub data: BTreeMap<String, Opaque>,       // per adapter; the core stores and passes it, never reads it
    pub evidence: Option<Evidence>,
    pub crossings: Vec<Crossing>,             // every outward operation it crossed, in order (5.3)
    pub brief: Option<String>,
    pub failure: Option<String>,
    pub models: BTreeMap<StationId, String>,
    pub dependencies: BTreeSet<String>,
    pub followers: Vec<String>,
    pub visits: Vec<Visit>,
    pub fence: u64,
}
pub struct Batch { pub at: Location, pub members: Vec<Membership>, pub data: BTreeMap<String, Opaque>,
    pub crossings: Vec<Crossing>, pub suspects: Vec<String>, pub bisect: Option<Bisect>,
    pub visits: Vec<Visit>, pub fence: u64 }
/// One token in a batch. What the token joined with (for coding, its candidate commit) is the adapter's.
pub struct Membership { pub token: String, pub joined: u64, pub data: BTreeMap<String, Opaque> }
pub struct Opaque { pub version: String, pub value: serde_json::Value, pub digest: String }
pub struct Evidence { pub artifact: String, pub manifest: String, pub receipt: String, pub spec: String }
/// One outward act that happened. Identity is `boundary` + `round`, never the station's id: `boundary` is the
/// adapter's name for what in the world the operation changes (`coding:target:main`, `mail:send`; 5.3), so a
/// rename or a replacement of the station can't reset rounds or lose a repeat-once boundary. `round` counts
/// this token's crossings of that boundary (1 for the first send, 2 for the second) and is copied from the
/// effect, never recomputed. `station` is history for the board, re-keyed on rename like a visit's.
pub struct Crossing { pub boundary: String, pub round: u32, pub effect: String, pub station: StationId,
    pub artifact: String, pub receipt: String, pub revision: u64 }
/// What a later step needs to know about a token or batch that has left live state (today's
/// `Archived`/`ArchivedGroup`, state.rs:150-166, without `candidate` and `pushed`).
pub struct Archived {
    pub closed: Closure,                      // Done, Cancelled; and its reason
    pub reason: Option<String>,
    pub crossings: Vec<Crossing>,             // what it took out into the world, as opaque artifacts
    pub batch: Option<ArchivedBatch>,         // for a batch: what the board needs to keep drawing it
    pub data: BTreeMap<String, Opaque>,       // the adapter's last data (coding: candidate, pushed commit)
}
pub enum Closure { Done, Cancelled }
pub struct ArchivedBatch { pub name: String, pub members: Vec<String>, pub since_ms: i64 }
/// One unit of work for the runtime. Every field is core; whatever an adapter needs to run it (coding's tree,
/// its members' candidates, a CI rerun's run) is in `data`, copied from the subject when the effect is made.
pub struct Effect {
    pub id: String,
    pub subject: Subject,                     // Token(key) or Batch(id); today `item`
    pub run: String,
    pub epoch: u64,
    pub fence: u64,
    pub resources: Vec<String>,               // opaque names from the catalog, e.g. "coding:target:main"
    pub kind: EffectKind,
    pub op: OpRef,                            // op.spec: digest of the exact operation spec it runs
    pub created_ms: i64,
    pub model: Option<String>,                // agent steps
    pub brief: Option<String>,                // agent steps: why it was sent back
    pub followers: Vec<String>,               // tokens riding on this one; today `with`
    pub members: Vec<String>,                 // a batch effect's member keys, in order
    pub idempotency: String,                  // outward ops: (subject, boundary, round), fixed when the effect
                                              // is made; no rename, replacement or publish ever changes it
    pub data: BTreeMap<String, Opaque>,       // per adapter: coding puts tree, candidates, ci_retry here
}
pub enum Subject { Token(String), Batch(String) }
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
    Closed { crossed: bool },                 // crossed = !crossings.is_empty()
}
pub struct Work { pub run: String, pub attempt: u8, pub pending: Option<String>, pub op: OpRef,
    pub stopping: Option<String>, pub uncertain: bool, pub wait_since: Option<i64>,
    pub again: Option<Opaque> }               // an adapter's "run me again with these facts" (the CI rerun)
pub struct OpRef { pub adapter: String, pub op: String, pub spec: String }  // spec = digest of what it ran
pub struct Decision { pub run: String, pub deciders: Vec<String>, pub needs: u8, pub given: Vec<Verdict> }
pub struct Hold { pub reason: String, pub by: String, pub revision: u64, pub kind: HoldKind }
pub enum HoldKind { Asked, PolicyLimit { policy: String }, StepFailed, Uncertain, Quarantined,
                    Crossed { boundary: String } }   // crossed work a publish could not place (7.3)
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

Every field of today's `Effect` (`state.rs:252-283`), `Member`
(`state.rs:125`), `Archived` and `ArchivedGroup` (`state.rs:150-166`) has a
place above or in an adapter's `data`; section 11.1 maps each one. In short:
`candidate`, `Member::candidate`, `tree`, `ci_retry`, `Archived::candidate`
and `ArchivedGroup::pushed` all become coding data; `definition_pin` and
`policy_pin` become `op.spec`, a record of what ran, not a pin on the token;
`operation: OperationSpec` (`definition.rs:54-58`) is the published
operation, named by `op` and digested into `op.spec`.

A CI check in the repository (`scripts/`, run in CI) greps every type and
field name in the core module (`crates/workflow/src/pipeline/`, including
`Effect`, `Membership`, `Archived`, `Crossing`, `Outcome` and `Input`) for `branch`,
`commit`, `candidate`, `base`, `push`, `pull_request`, `pr`, `ci`, `git`,
`tree`, `fold` and `gate` as whole segments of a snake_case or CamelCase
identifier (so `pr` matches `open_pr` but not `priority`), so the line
stays where it is drawn. It is mutation-tested: adding `branch: String` to
`Token`, `candidate: String` to `Membership` or `pushed` to `ArchivedBatch`
must each turn it red.

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
  settle-before-move rule, boundaries (an opaque name per outward
  operation, which crossings and rounds belong to), `repeat: once` versus
  `fresh`, rounds, the standard of proof for settling (3.2), and "a
  crossing is never undone".
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
    /// Every operation: the station kinds it may run under, the labels it can answer, whether it is outward
    /// and if so its `boundary` (a function of the op and the config, e.g. `coding:target:<ref>`), `repeat`
    /// (`once` or `fresh`) and `dedupe` (`server` only if the outside world itself refuses a second act with
    /// the same idempotency key; otherwise `none`) (5.3), the resources it holds while it runs, and what it
    /// needs and makes (for validation).
    fn catalog(&self) -> &[OpSpec];
    /// A layout's use of this adapter's operations, in order: today's coding.rs:204-280 order rules.
    fn validate(&self, layout: &Layout, config: &serde_json::Value) -> Result<(), Vec<Problem>>;
    /// Whether an operation may start for this token or batch now, and with what arguments, resources and
    /// effect data (`Effect::data`: everything the execution half needs, such as coding's tree and the
    /// members' candidates). Sees the visits and crossings, so it can accept a skipped station (5.1) as the
    /// orchestrator's choice and refuse a `fresh` outward start whose evidence predates the last crossing.
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

**An outward operation must be reconcilable, and its answers have a
standard of proof.** The adapter declares an operation outward only if
`reconcile` and `stop` can look at the outside world for it (the remote's ref
for a push; the sent folder and the message id for an email). Each answer is
one of three, and the adapter may give only what its evidence supports:

| Answer | Required evidence | The core then |
|---|---|---|
| **Crossed** (`Labeled(passed)`) | *Positive* evidence naming this exact round: the remote's ref is the effect's commit; a sent message carrying this round's id | Appends the crossing with the effect's `boundary` and `round` |
| **Not crossed** (`Labeled(refused)`) | Both: *authoritative non-acceptance* (the remote rejected the push; the mail server answered a permanent error to this round's submission, recorded in the journal before the crash) **and** *quiescence* (no attempt for this idempotency key can still complete: the connection or request that carried it is known closed, and no other sender holds it) | Frees the token; `then` applies; a retry may start the same round again |
| **Unknown** (`Uncertain`) | Anything else, **including absence**: no sent copy found, a ref read failed, an attempt that may still be in flight | Holds `Uncertain`; the token keeps place, resources and ownership; nothing is retried |

Absence is never proof. "Not in the sent folder" says nothing on its own:
the server may accept a message without filing a copy, file it late, or
still be accepting it. The adapter's look-up is not assumed complete or
atomic with acceptance.

An identifier is not a deduplication guarantee. A round's `Message-ID`
identifies that round when it is found; it doesn't stop a second submission
from being delivered. So for an operation with `dedupe: none` (mail, a
deploy command, a push to a remote without a lease), the runtime never runs
an `uncertain` round's effect again; it only reconciles it. A new attempt of
the same round is allowed only after a *Not crossed* answer or a `Settle
{ crossed: false }` that meets the same standard (Q5). `dedupe: server` is
for an outside API that does refuse a repeated key; there a retry of the
same round is safe and may be how it reconciles.

While a token holds `Uncertain`, the runtime re-runs `reconcile` on the
`Advance` tick with backoff. A later *Crossed* or *Not crossed* answer
settles it as if it had come first; *Unknown* changes nothing.

An adapter that cannot reconcile at all (a plain shell command that sends
something) still runs, but every unknown outcome of it holds for a `Settle`
(Q5).

### 3.3 What the coding adapter owns

Everything below leaves the core and becomes `coding/v1`:

| Today, in the core | In `coding/v1` |
|---|---|
| `RunTree` (`state.rs:110-114`): worktree name, branch, path, base, adopted | Token data |
| `candidate`, `pushed` (`state.rs:34,54`) | Token and batch data. The core keeps only `Evidence::artifact` (an opaque id) and `Crossing` |
| `Member::candidate` (`state.rs:125`) | `Membership::data["coding"]`: the candidate the ticket joined with |
| `Archived::candidate`, `ArchivedGroup::pushed` (`state.rs:150-166`) | `Archived::data["coding"]`; the core keeps the `Crossing` with its opaque artifact |
| `Effect::candidate`, `members`, `tree`, `ci_retry` (`state.rs:261-283`) | `Effect::data["coding"]`, filled by `start`; the core's `Effect::members` is keys only |
| `StageKind` coding verbs (`definition.rs:21-52`) | Operations: `coding.provision`, `.build`, `.check`, `.review`, `.route`, `.base`, `.fold`, `.gate`, `.group_review`, `.push`, `.ci`, `.push_branch`, `.open_pr`, `.pr_ready`, `.pr_reviews`, `.merged`, `.land` |
| `lands()` (`definition.rs:488`) | `outward: once` on `.push`, `.land`, `.merged` (5.3) |
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
which kinds each operation may run under.

**Routing, exactly.** When a token or batch leaves a station with label `L`:

1. If the station's `routes` has `L`, it goes to that target.
2. Otherwise, if `L` is `passed`, it goes to the next station in the list.
   `skipped` (5.1) goes wherever `passed` would, or at a decision station
   wherever `approved` would; the visit says `skipped`. Past the last station of the top-level list it goes
   to `done`; past the last inner station of a batch, to `finish`.
3. Any other label holds it at that station (`HoldKind::StepFailed`, the
   label as its reason). Nothing closes by falling through.

A decision's verdicts (`approved`, `rejected`, `edited`, ...) are labels
like any other, so a decision station must route each verdict it should act
on; an unrouted verdict holds. A route target is a station id or one of
these reserved targets (`validate` refuses any other name):

| Target | Where | Means |
|---|---|---|
| `done` | top level | Close the token: `Closed { crossed }`, `Archived { closed: Done }`, with its crossings and adapter data kept. Reopen brings it back under the same key |
| `finish` | inside a batch | The batch is complete: its members leave together with label `passed`, routed by the batch station's routes (rule 1-3 again, one level up) |
| `rejected` | inside a batch | Start the batch's `on_reject` policy (4.5) |
| `returned` | inside a batch | The members the adapter names (a fold's conflicting tickets) leave along the batch station's `returned` route; the rest carry on to the next inner station |

`validate` also refuses a layout where a decision station has no route for
`approved`, or where a `repeat: once` outward station can be reached again
from any of its **crossing** labels (5.3). The catalog splits an outward
operation's labels into crossing ones (`passed` for a push) and not-crossed
ones (`refused`); a route from a not-crossed label may loop back, because
nothing went out.

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
the rest pass the station together and go on as one batch. Under
`return_suspects` there is no bisect: the suspects leave along `returned` at
once and the rest re-enter the rejecting station without them (a decision
asks its deciders again, about the smaller batch). Under `hold` the batch
holds whole for the orchestrator. For coding: an
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
| Send anywhere before where it is, any number of times | `Send { token, to, reason }` | Queues there by priority, brief = reason; through `Stopping` if working; never back across a `repeat: once` crossing; back across a `repeat: fresh` crossing starts a new round (5.3) |
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

**Rule 1: a crossing is never undone, and only a `fresh` one is repeated.**
Every crossing is appended to the token's (or batch's) `crossings` and is
never removed: no command, route or publish can make the engine believe an
outward step didn't happen. A `Cancel` after a crossing closes the token as
`Closed { crossed: true }` with its crossings kept; there is no command that
undoes a crossing. A crossing belongs to a **boundary**, not to a station:
the adapter's catalog names, for each outward operation, what in the world it
changes (`coding:target:main` for `.push`, `.land` and `.merged` to `main`;
`mail:send` for `mail.send`; `cms:issue` for `cms.publish`). Every station
whose operation has boundary K is "a K station"; rounds, freshness and
repeat-once are all counted per boundary. `validate` refuses a layout or
catalog in which two operations share a boundary but differ in `repeat`.
Beyond that, each boundary declares how it may be repeated:

- **`repeat: once`** (`coding.push`, `coding.land`, `coding.merged`,
  `cms.publish`). The thing that crossed can't cross again: a pushed commit
  is on the target, a published issue is live. So a token or batch that
  crossed boundary K is never sent back to a K station or before one, by
  any means. Routes: `validate` refuses a layout where any K station is
  reachable from a K station's crossing labels (4.1). Interventions: a
  `Send` to a station from which any K station is reachable is refused
  `Crossed`. Because the check names the boundary, it still holds after the
  station that crossed was renamed, replaced or removed (7.3). A further
  change is a `Reopen` under the same key, which the
  adapter starts afresh (for coding, on the new base; today's `Reopen`,
  `reducer.rs:56-80`). A batch that crossed can't be pulled from
  (`reducer.rs:274-279`). This is the first pass's lost-push protection,
  unchanged.
- **`repeat: fresh`** (`mail.send`, `deploy.production`). Each crossing is a
  new act with new content: a second reply, a follow-up, the next deploy.
  Routes may loop back before the station (an answer arrives, the thread goes
  back to triage), and a `Send` back before it is allowed. Either starts a
  new **round**; three rules keep a round from being a repeat of the last:
  1. *Fresh evidence.* The adapter's `start` refuses (`Evidence`) unless the
     token's evidence was made after its last crossing of this boundary (for
     mail, a draft approved since the last send). A retry loop or a stale
     route can't send the old message again.
  2. *Fresh idempotency.* The effect's `idempotency` is
     `(token, boundary, round)`, with `round` = the highest round the token
     has crossed on this boundary + 1, computed once when the effect is made
     and stored in it. Reconciles of one round share it (for mail it becomes
     the `Message-ID`); round 2 can never be mistaken for round 1 in the sent
     folder, and round 1's message can never settle round 2. The settled
     crossing takes its `round` from the effect, not from a recount. While
     a round is admitted or uncertain no next round can start (rule 2 holds
     the token), so two effects never share a round.
  3. *History, not state.* The new round appends a crossing; earlier ones
     stay. Interventions never name a crossing: `Send` back always means "do
     another round", never "undo the last one", and the board shows rounds
     as a list (sent 1, sent 2) so the orchestrator and the person can see
     which happened.

Rule 2 applies to every round on its own: while a round's effect is
admitted or uncertain, the token settles before it moves.

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
4. **Unknown** (by the standard of 3.2, which includes "no record found"):
   `Held { Uncertain }` at the station, everything still owned, `then` kept
   in the hold, the effect never run again. A later reconcile that answers
   crossed or not crossed releases it, as does `Settle`: `crossed: true`
   continues as in 3, `crossed: false` as in 5. A `Settle` carries its
   evidence as a typed record (a ref read, a sent copy, a recorded rejection
   plus a closed attempt, a person's judgment), which the adapter's
   `observe` checks against the same standard; an orchestrator's `Settle`
   whose only evidence is absence is refused `Evidence`. Whether `Settle` is
   the orchestrator's (with evidence) or a person's is Q5.
5. **Proved not crossed** (authoritative non-acceptance and quiescence, 3.2:
   the target refused the push, the mail server permanently rejected this
   round's submission and the connection that carried it is closed, the pull
   request closed unmerged), or `Settle { crossed: false }`: only now does
   `then` apply, in the same event that frees place and resources.

A pending outward effect (never claimed) cannot have crossed: the command's
transaction suppresses it (`store/src/workflow.rs:346-351`), and 5.2 applies.

**Walk-through: a push that landed, its `Observe` lost.** Kept from the first
pass. Batch B7 (T1-T3) at `land.push`, push effect E admitted. The push lands
(`main` = `c7`); the daemon dies before `Observe(E)`. On restart E is
`uncertain`, no crossing recorded. The orchestrator sends `Pull { T2 }`,
`Cancel { T3 }`, `Send { T1, build }`: B7 goes to `Settling`, all three are
recorded, nothing moves. Another crash; recovery reconciles E first, then the
stop; `stop` answers `Settled { original: Labeled(passed, c7) }`. B7's
crossing (round 1, `c7`) is appended, T1-T3 stay members, B7 goes to `land.ci`, the three
requests are recorded as overruled `Crossed`. Had the remote refused the push,
T2 would leave, B7 refold without it, T3 close, T1 queue at build. Had the
remote been unreadable, B7 would hold for `Settle`.

**The same for an email, over several rounds.** Thread M4 in the assistant
workflow (8.3), where `send` is `mail.send`, `repeat: fresh`.

1. *Round 1.* Triage `reply`, draft D1, the person approves, `send` runs
   with idempotency `(M4, mail:send, 1)`, `Message-ID` derived from it. It
   answers `passed`; crossing 1 (artifact D1, the message id) is appended;
   M4 waits at `await-reply`.
2. *A reply arrives.* `await-reply` answers `replied`, routed to `triage`:
   round 2 begins by an ordinary route, no `Reopen`. Triage `reply`, draft
   D2 (evidence made after crossing 1), approved, `send` starts with
   `(M4, mail:send, 2)`. Had a stale route or a retry tried to start `send` with
   D1, `start` would refuse `Evidence`.
3. *Crash after the second send.* The daemon dies while round 2's
   submission is out, before `Observe`. On restart round 2's effect is
   `uncertain`. Meanwhile the person, seeing a wrong recipient in D2,
   cancels M4: M4 goes to `Settling`, nothing moves. Recovery reconciles the
   effect first, and what happens next depends only on what `mail/v1` can
   prove (3.2):
   - *A sent copy with round 2's `Message-ID` is found:* `Settled
     { original: passed }`. Crossing 2 is appended with the effect's
     boundary and round (1 is untouched), the cancel is overruled
     `Crossed`, and M4 goes to `await-reply`. The board shows "sent 1, sent
     2", so the person sends a correction (round 3) or cancels again, which
     now closes M4 `crossed: true`. Round 1's sent copy can't settle round
     2, because its id differs.
   - *The server accepted D2 but no sent copy is visible* (it doesn't file
     submissions, or files them late): the sent folder holds only round 1's
     message. That is absence, not proof: the answer is *Unknown*. M4 holds
     `Uncertain` with the cancel kept in the hold, keeps its place and
     ownership, and round 2 is not sent again. A later reconcile that finds
     the copy settles it crossed; otherwise the orchestrator or a person
     `Settle`s with evidence (a reply quoting D2, the provider's delivery
     log; Q5).
   - *The submission may still be in flight* (the sending connection's
     state is unknown, or a relay still holds it): *Unknown*, the same hold.
     Even a later permanent rejection only settles it not crossed once the
     attempt is also known closed.
   - *Proved not crossed:* the journal recorded the server's permanent
     rejection of round 2's submission and the connection is closed. Only
     now does the cancel apply; M4 closes with crossing 1 only.
4. *A follow-up.* No reply in 3 days: `await-reply` answers `timeout`,
   routed to `follow-up`, which drafts D3 and goes to `approve`, then
   `send` as round 3 with `(M4, mail:send, 3)`. Same rules.

A push could never do step 2: `coding.push` is `repeat: once`, so no route
leads back before it, and the walkthrough above is unchanged.

### 5.4 What still refuses, and why none is a count

| Refusal | Why it stays |
|---|---|
| `Stale`, `Duplicate` | The token moved, or the input was seen |
| `Invalid`, `Missing`, `Evidence`, `Definition` | Malformed input; `Evidence` also for a plain `Stopped` on an outward stop |
| `Unauthorized` | The grant doesn't match (`reducer.rs:590-596`) |
| `Crossed` | Rule 1 of 5.3: a send back to or before a `repeat: once` station the token crossed, a release in place of a `Crossed` hold (7.3), or a pull from a batch that crossed |
| `Stopping` | Work on a tree, batch or resource whose last effect has no stop receipt yet (5.2) |
| `PersonOnly` | Only for what the owner keeps on the list in section 9; empty if every question is answered "no" |
| `Stranded` | A publish that would leave a token at no station (7.3) |

Every refusal today that exists because a ticket carries a budget is gone:
`Retry` past `MAX_RETRIES`, `SendBack` past `send_backs` or a back-edge limit,
rework past `max_reworks`, a routed back edge failing the run, `SendBack`
while a step runs, `Leave` from a group that has left, `Cancel` of a
follower, `Resource` while a resource is held (section 11.4 maps each).

## 6. Limits: per workspace, with an optional host limit

**Per workspace.** Each workspace declares its pools in its **workspace
settings**, a document beside the workflow, not inside it (so a later second
workflow shares them, 2.4; section 8 shows one per walkthrough):

```yaml
workspace:
  limits: { pools: { agents: 4, work: 2, landing: 1 } }
```

Stations name a pool; each station also has its own capacity (default: its
pool's; a station may set `capacity` lower or higher, and the pool still
binds). Pools and capacities are in the reducer state, set from the settings
and the layout, and changed by `SetCapacity { pool: agents, n }` or
`SetCapacity { station: build, n }`, which the orchestrator may send at any
time.
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

**Who sets which.** A workspace's pools are its orchestrator's (in the
settings or by `SetCapacity`). The host limit is the user's, because it spans workspaces and
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
    review: { rename: code-review }           # same station, new id
    land.checks:                              # removed: replaced by another station
      to: land.group-review                   # where its tokens go, and where its passes go
      labels: { red: rejected }               # its other labels; unlisted ones hold at `to`
    send: { to: send-relay, boundary: keep }  # outward replaced: say whether the boundary carries on (7.3)
  reason: "replace cheap checks with group review before the gate"
  preview: <digest of the preview it was checked against>
```

A publish also carries `after`, one continuation per `repeat: once`
boundary, saying where work that already crossed it resumes (7.3).

The `Publish` event stores the whole layout, `moves` and `after` in the log (content
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
  revision, or held there with its hold reason kept. Crossed work is checked
  by the migration rule (7.3) first.
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
- **Its station was renamed** (`{ rename: new-id }`): nothing retires. A
  rename is allowed only when the kind and `op` are unchanged (otherwise it is
  a replacement, below, and `validate` says so). In the publish event the
  station's queue, working set, holds, every visit's station id and every
  crossing's `station` are re-keyed to the new id; the running step answers
  at the new id and is routed by the **new** layout's routes for it, exactly
  as if it had always had that name. A crossing's identity (`boundary`,
  `round`, `effect`) and a running effect's `idempotency` are untouched: they
  never contained the station id.
- **Its station was removed** (`{ to, labels }`): the station becomes
  **retired**. It stays in `Pipeline::retired`, takes no new arrivals, starts
  nothing, and keeps **only its translation**, never its old routes. The
  token finishes there; when it answers with label `L`:
  1. `passed` or `skipped`: the token **arrives at `to`** and queues there
     by priority, as any arrival does, unless it has crossed a
     `repeat: once` boundary that `to` leads back to (the migration rule,
     below). It does not jump to where the old
     station's `passed` used to lead: the new process from `to` onward runs
     in full, so a station the publish added after it (group review) is not
     bypassed.
  2. `L` listed in `labels`: it goes to that target (a live station or a
     reserved target of 4.1, such as `rejected` inside a batch), by the
     routing rules of 4.1.
  3. Any other label: held at `to` with `L` as its reason
     (`HoldKind::StepFailed`), as an unrouted label is anywhere.

  When the retired station is empty it is deleted. The board shows it greyed
  with its working tokens, so nothing is hidden.
- **Its station is a decision or wait:** as a queued token (7.2): it moves to
  the `moves` destination at once, and a wait's probe effect is stopped
  (5.2), unless the wait is outward, in which case it settles first (5.3).
- A step stopping or settling at publish time (5.2, 5.3) keeps doing so; its
  `then` is mapped through `moves` (a rename re-keys it; a removal maps it to
  `to`).

**Outward stations: rename versus replacement.** Because crossings, rounds
and idempotency belong to the boundary (5.3), neither a rename nor a
replacement can reset them:

- A **rename** keeps the op, so it keeps the boundary. A thread that crossed
  `mail:send` twice at `send` and then reaches the renamed `send-reply`
  starts round 3; its old draft is refused `Evidence` there as it would have
  been at `send`.
- A **replacement** whose new op has the **same** boundary (a `mail.send`
  through another relay; `coding.land` in place of `coding.push` to the same
  target) carries on: rounds continue, and a token that crossed a
  `repeat: once` boundary still can't be sent before the new station.
  `moves` says `boundary: keep`, and `validate` checks that the boundaries
  really match.
- A replacement whose op has a **different** boundary (pushing to `release`
  instead of `main`), or a publish that leaves no station on a boundary
  that a live or retired token has crossed, must say `boundary: new` on that
  `moves` entry, and the preview lists every token that crossed the old
  boundary. Without it the publish is refused `Definition`. The old
  crossings stay on the tokens either way; what `boundary: new` records is
  that the author knows the new station starts its own round 1.
- An outward effect admitted or uncertain in a renamed or retired station
  keeps the idempotency it was made with; its reconcile looks for that
  round's evidence, and the crossing it settles takes the effect's boundary
  and round.

**Crossed work never re-enters its boundary.** The translations above say
where work goes by station. One rule, checked after them, says where crossed
work may not go, and it is the same rule for a removed, renamed, split or
merged station:

> **The migration rule.** Whenever a publish places a token or batch that
> has crossed a `repeat: once` boundary K since its last `Reopen` (which
> starts it afresh, 5.3), counting its batch's crossings for a member and a
> crossing recorded by the very answer being translated, the place is
> checked exactly as a `Send` is (5.3, rule 1): it must not be a K station, nor a station from which the new layout can
> reach one. A place that fails goes to the boundary's **continuation**
> `after[K]` instead: queued there for `passed`, `skipped` or a moved queued
> token, held there with the label as its reason for any other label. With
> no `after[K]`, the token holds where it is with
> `HoldKind::Crossed { boundary }` for the orchestrator.

"Places" means every placement a publish makes: a queued or held token
moved by `moves` (7.2); a renamed station re-keyed where the new layout
puts it; a retired station's answer translated by `retired_route` (`to`,
or a `labels` target); a stopping or settling step's `then`; a member
leaving a removed batch station (7.4); and a crossed token that stays at a
live station the new layout moved before a K station. A token at a
retired outward station whose effect is still admitted or uncertain settles
first (5.3, rule 2); the rule applies to the place it settles into. A
push's `passed` is a crossing label (4.1), so the case the rule exists for,
a removed push replaced by another push on the same target, never reaches
the replacement.

How each station change meets the rule:

| Change | Where the old translation would put crossed work | What the rule does |
|---|---|---|
| **Removed**, replaced on the same boundary (`boundary: keep`) | At `to`, itself a K station: a second push | `after[K]`, the station after the replacement |
| **Renamed** | At the new id. The op is unchanged, so a renamed K station's crossing labels are already checked by `validate`; but a renamed post-K station the new layout placed before a K station would lead back to it | Nothing for a renamed K station; `after[K]` for the reordered one |
| **Split** in two (`to` is the first half) | At the first half. `validate` forbids two K stations in a row on one path, so at most one half is a K station; if the K half comes second, the first half leads to it | `after[K]`, past the K half, whichever half it is |
| **Merged** (both old ids name the merged station in `moves`) | At the merged station, a K station if either part was | `after[K]`, the station after the merged one |

**How the mapping expresses it.** The publish carries one continuation per
`repeat: once` boundary the new layout has a station for:

```yaml
  after: { coding:target:main: land.ci }   # where crossed work resumes
```

`after[K]` is a live station or a reserved target (4.1), and `validate`
checks it as the rule checks a place: neither a K station nor one from which
a K station is reachable. The preview proposes it (7.5): follow the new
layout's routes from the K stations' crossing labels to the first station
from which no K station is reachable. That is always the station
immediately after the boundary, since `validate` already forbids reaching a
K station from a crossing label.

**When the new layout has nothing after the boundary.**

- *The K station is the last station.* The proposed `after[K]` is the
  reserved target its `passed` reaches: `finish` inside a batch (so the
  batch station's `passed` route applies, `done` in 8.1) or `done` at the
  top level. Crossed work then closes `crossed: true`, as a fresh token
  would under the new process; the preview lists every token and batch that
  will close this way without running a station it would have run before
  the publish (say `land.ci`), and the author may write `hold` instead.
- *The new layout has no K station at all* (the boundary was dropped, or
  replaced under `boundary: new`). The rule then refuses nothing: no
  station can reach K. Crossed work follows `to` and the routes as written.
- *No continuation can be proposed* (the K stations' continuations
  disagree; see 7.6). `after[K]` is left unset, or the author writes
  `after: { K: hold }`: crossed work that fails the check holds `Crossed`
  where it is. A retired station holding such a token is not deleted.

A `Crossed` hold has only checked exits: `Release { to }` and `Send`, both
checked by rule 1 of 5.3; `Release` in place is refused `Crossed`;
`Cancel` closes the token `crossed: true`; `Skip` leaves along the current
station's `passed` route only if that route passes the check.

**Worked example: a push replaced while a batch is pushing.** The 8.1
layout. Batch B7 (T1-T3) is working at `land.push`; its push effect E is
admitted. The orchestrator publishes a layout that replaces `land.push` by
`land.land` (op `coding.land`, the same target, so boundary
`coding:target:main`, followed by `land.ci`):

```yaml
moves:
  land.push: { to: land.land, boundary: keep }
after: { coding:target:main: land.ci }     # proposed by the preview
```

1. *Publish.* `land.push` is retired with E still running under its
   original spec. Batch B9, queued at `land.push` and never crossed, moves to
   `land.land`'s queue (7.2); the rule passes it, since it has crossed
   nothing.
2. *E answers `passed`* (`main` = `c7`). The crossing (round 1, `c7`) is
   recorded first. `retired_route` gives `land.land`, a K station; B7 has
   now crossed K, so the rule sends it to `after[K]`: **B7 queues at
   `land.ci`**. It does not push again and is not stranded before CI.
3. *A `Send { B7, to: land.land }` from the orchestrator* is refused
   `Crossed`, as before the publish.
4. *Variants.* E's answer is unknown: B7 holds `Uncertain` at the retired
   `land.push` and, once settled crossed, goes to `land.ci` as in 2; settled
   not crossed, it follows `retired_route` to `land.land`, which passes the
   check because nothing crossed. The new layout ends at `land.land` (no
   `land.ci`): `after[K]` is proposed `finish`, B7's members close
   `crossed: true`, and the preview said so. The author wrote
   `after: { coding:target:main: hold }`: B7 holds `Crossed` at the retired
   `land.push` until the orchestrator sends it on.

**Successive publishes compose.** A retired station's translation names live
stations only, always. Each later publish rewrites every entry in
`Pipeline::retired` through its own `moves`: a renamed target is re-keyed, a
removed target is replaced by that removal's `to` (and a `labels` target by
the same rule), and a retired station that is itself named again is
unchanged. So if publish 1 retires `checks` with `to: group-review` and
publish 2, while `checks` still has a running batch, replaces `group-review`
with `review-2`, the batch's eventual `passed` arrives at `review-2`. Nothing
can route into a deleted station, however late the answer.

**One function, three uses.** The translation above is one pure function,
`retired_route(&Retired, label) -> Target`. The reducer uses it on `Observe`;
the preview (7.5) uses it to list where each running step's possible labels
will go; the rehearsal (10.3) asserts against it.

**Stranding is refused.** The publish is refused `Stranded` if any token,
batch, `then`, or retired translation would end at a station that neither
exists nor is in `moves`. Rename is a `moves` entry; there is no other way to
rename.

### 7.4 A batch mid-gate

A batch is a token for these rules, plus:

- **The station it is at still exists:** the gate finishes; its result is
  routed by the new layout. Stations added before it (the owner's group
  review) are not run for this batch; stations added after it are.
- **The batch station's inner stations were reordered or removed:** as 7.3,
  station by station. A retired inner station finishes its batch, which then
  arrives at its `to` (on `passed`), follows `labels` (`red: rejected` starts
  `on_reject`), or holds at `to`.
- **Batch formation changed** (size 8 to 4): applies to the next batch
  formed; a formed batch keeps its members.
- **The batch station itself was removed** (the workspace stops batching):
  `moves` must name where its members go, written `land: { unbatch: review }`.
  A forming batch's members go there at once. A working batch finishes its
  current inner station, then its members leave one by one to the
  destination, each with the batch's result in its visit. A batch that has
  crossed finishes its post-crossing stations first (5.3, rule 1), and each
  member's destination is checked by the migration rule (7.3).
- **A bisect in progress** finishes under the batch's current membership; a
  changed `on_reject` applies to the next rejection.

### 7.5 Who writes the mapping, and how a change is previewed

**Who writes `moves`:** whoever publishes (the orchestrator, or a person if
Q1 reserves it). `workflow preview` proposes it, and the author accepts or
edits it:

- A station whose id changed but whose kind and op did not is proposed as a
  `rename`.
- A removed station's `to` is the station that now stands in its place: where
  the **new** layout sends a `passed` token from the removed station's old
  predecessor. For `land.checks` (after `land.fold`) in a layout that put
  `group-review` after `fold`, that is `land.group-review`, not `land.gate`.
  With no predecessor, the new layout's first station after it in list order.
- Its `labels` are its old non-`passed` routes, each target mapped through
  the same `moves` (`red: rejected` stays `red: rejected`; `rejected: build`
  becomes `rejected: code-build` if `build` was renamed). A label whose old
  target was removed with no `to` is left out, so it would hold, and the
  preview says so.
- A removed outward station's entry is proposed `boundary: keep` when its
  replacement's op has the same boundary. Otherwise the preview leaves it
  unset, which refuses the publish, so the author must write
  `boundary: new` (7.3).
- `after[K]` for each `repeat: once` boundary the new layout has a station
  for: the first station past the boundary (7.3), or the reserved target
  that ends there. When the K stations' continuations disagree it is left
  unset, so crossed work that needs it holds (7.6).

**Preview** is the reducer itself: `step` is pure, so
`preview(&Pipeline, &Publish) -> Report` applies the publish to a copy of the
live pipeline and diffs the result. Nothing is written. The report lists:

- every token and batch whose location changes, from and to;
- each queue before and after, in order;
- retired stations, the work finishing in them, and their translations after
  composing with any earlier retired entries (7.3);
- what each running step's possible labels will do: by the new routes for a
  live or renamed station, by `retired_route` for a retired one;
- policies that will hold something at its next count;
- outward boundaries: each one kept, added or left with no station, and
  the tokens that crossed it, with their next round;
- every placement the migration rule changed (7.3), from the translation's
  place to `after[K]` or to a `Crossed` hold, and every crossed token or
  batch that will close at a terminal `after[K]` without running a station
  it would have run before;
- the adapter's `validate` problems;
- refusals (`Stranded`, adapter problems, capacity above the host limit).

`Publish` carries the preview's digest and the revision it was taken at. If
the pipeline moved since, the publish recomputes the preview inside the same
transaction and is refused only if the new result has a refusal; otherwise it
goes through and the event stores the final report. The orchestrator's
playbook: preview, read, publish.

### 7.6 Open migration questions

The migration rule (7.3) decides every case above. These it does not; each
defaults to holding the work for the orchestrator (`HoldKind::Crossed`,
with checked exits only) until the owner or a later design decides it.

| # | Case | Default |
|---|---|---|
| O1 | **The K stations' continuations disagree.** Two paths each end in a push to `main`, continuing at different stations, so there is no one `after[K]`. Resuming on the path the token was on is plausible, but the old path may no longer exist | Hold for the orchestrator; the preview lists the candidates |
| O2 | **A token crossed two `repeat: once` boundaries whose continuations differ** (a release pushed to `main` and published to the CMS), and the place fails the check for both | Hold for the orchestrator |
| O3 | **`after[K]` is an inner station of a batch, but the crossed work is a lone token** (a top-level push replaced by a batched landing). A token can't be at an inner station without a batch, and forming a batch of one after a crossing is undesigned | Hold for the orchestrator |
| O4 | **A boundary's `repeat` changes between versions** (an adapter upgrade makes `cms:issue` `fresh`, or makes a `fresh` boundary `once`). Whether earlier crossings count under the new rule is undecided | Hold for the orchestrator any crossed work the publish places at, or before, a station on that boundary |
| O5 | **An adapter upgrade renames a boundary** (same target, new name). The engine can't tell it is the same boundary, so the rule can't apply on its own | The upgrade must name the old boundary; until then, crossed work the publish places at, or before, a station on the new name holds for the orchestrator |

## 8. Walkthroughs

Each is two documents in the schema-3 format, shown as YAML (the stored form
is JSON): the **workspace settings** (limits, and the person-only `publish`
setting of Q1, which the orchestrator can't change) and the **workflow** (what
`Publish` carries). Routing follows 4.1 exactly: a routed label goes to its
target; an unrouted `passed` goes to the next station, or `done` / `finish`
past the last; every other unrouted label holds. Each layout below is followed
by a trace of every label each station can answer, so nothing relies on
prose.

### 8.1 Far Cooler: fully agentic

```yaml
workspace:
  limits: { pools: { agents: 4, work: 2, landing: 1 } }
  publish: orchestrator
---
schema: 3
name: agentic-batched-push
adapters:
  coding: { version: coding/v1, config: { repository: ., target: main, ci: ci.yml, checks: [...] } }
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
    routes: { returned: build, passed: done }
    stations:
      - { id: base, kind: wait, op: coding.base, policy: { patience: 3h } }
      - { id: fold, kind: command, op: coding.fold, routes: { conflict: returned } }
      - { id: checks, kind: command, op: coding.check, routes: { red: rejected } }
      - { id: group-review, kind: agent, op: coding.group_review, models: [claude:opus],
          routes: { rejected: rejected } }
      - { id: gate, kind: command, op: coding.gate, routes: { red: rejected } }
      - { id: push, kind: command, op: coding.push, routes: { refused: base } }   # outward, repeat: once
      - { id: ci, kind: wait, op: coding.ci, policy: { patience: 3h, again: 1 } }
```

Trace:

- `provision` passed → `build` → `review`; `review` rejected → `build`
  (a third rejection holds by `rework: 2`), passed → `route`.
- `route` passed → `land`; `needs_review` → `decide`; `decide` approved →
  `land`, rejected → `build`.
- `land.base` passed → `fold`; `timeout` holds (the target hasn't moved in
  3 h: something is wrong). `fold` conflict → `returned` (those tickets go to
  `build`, the rest go on), passed → `checks`. `checks`, `group-review`,
  `gate`: a failure → `rejected` (bisect), passed → next.
- `push` passed → `ci`, with the crossing recorded. `refused` (the target
  moved; nothing crossed) → back to `base` to refold. `validate` accepts the
  loop because `refused` is a not-crossed label in the catalog (4.1); from
  `passed`, nothing leads back to `push`.
- `ci` passed → `finish` → `land` passed → `done`: every member closes
  `crossed: true`. `red` (after one adapter rerun) or `timeout` holds: the
  batch has crossed, so a fix is a new ticket or a `Reopen`; no route may lead
  back before `push`.

| Station | Who decides |
|---|---|
| review | An agent's receipt; two rejections hold for the orchestrator |
| decide | The orchestrator, for changes the route rule flags (security paths, migrations) |
| land.gate | The gate command; the orchestrator may skip it with a reason |
| land.ci red | Holds for the orchestrator after one adapter rerun |

Every skip, send, hold, publish and capacity change is the orchestrator's. No
station needs a person.

### 8.2 A company with human pull request review

Agents build; people review on GitHub; two approvals are required, one from
code owners; a person merges; a release manager signs off production.

```yaml
workspace:
  limits: { pools: { agents: 2, work: 2 } }
  publish: person                              # Q1: only a person publishes, so Q7's reservations stick
---
schema: 3
name: company-human-review
adapters:
  coding: { version: coding/v1, config: { repository: ., pull_request: { repository: acme/app, base: main } } }
  deploy: { version: command/v1, config: { staging: [./deploy, staging],
            production: { run: [./deploy, production], outward: fresh, reconcile: [./deploy, status] } } }
stations:
  - { id: provision, kind: command, op: coding.provision, pool: work }
  - { id: build, kind: agent, op: coding.build, pool: agents, models: [claude:opus] }
  - { id: self-review, kind: agent, op: coding.review, pool: agents, routes: { rejected: build },
      policy: { rework: 2 } }
  - { id: open-pr, kind: command, op: coding.open_pr, pool: work }      # push_branch + open/update + ready
  - { id: pr-ci, kind: wait, op: coding.ci, policy: { patience: 2h }, routes: { red: build } }
  - id: code-review
    kind: decision
    deciders: [group:codeowners, group:engineers]
    needs: { total: 2, from: { group:codeowners: 1 } }
    source: coding.pr_reviews                  # GitHub reviews arrive as verdicts
    routes: { approved: merged, changes_requested: build }
    policy: { skip: person }                   # Q7: only a person may waive review here
  - { id: merged, kind: wait, op: coding.merged, policy: { patience: 14d },   # outward, repeat: once
      routes: { passed: main-ci } }
  - { id: main-ci, kind: wait, op: coding.ci, policy: { patience: 2h } }
  - { id: staging, kind: command, op: deploy.staging, pool: work }
  - id: release
    kind: decision
    deciders: [person:release-manager]
    routes: { approved: production }
    policy: { skip: person }
  - { id: production, kind: command, op: deploy.production, pool: work, routes: { passed: done } }
```

Trace:

- `provision` → `build` → `self-review`; rejected → `build` (third holds),
  passed → `open-pr` → `pr-ci`.
- `pr-ci` red → `build` (the branch push is not outward; the PR is updated
  on the next `open-pr`), passed → `code-review`, `timeout` holds.
- `code-review` approved (two approvals, one a code owner) → `merged`;
  `changes_requested` → `build`, with the comments as the brief. Any other
  verdict holds.
- `merged` passed (a person merged; crossing recorded) → `main-ci`.
  `closed` (the PR closed unmerged; not crossed) holds for the orchestrator,
  which may `Send` it to `build` or `Cancel` it. `timeout` (14 days) holds.
- `main-ci` passed → `staging`; `red` or `timeout` holds: the merge crossed,
  so the fix is a new ticket or a `Reopen`.
- `staging` passed → `release`. `release` approved → `production`;
  `rejected` holds, so the release manager's reason is on the board for the
  orchestrator to act on (a new ticket, or `Cancel`).
- `production` passed → `done`; `failed` holds.

| Station | Who decides |
|---|---|
| self-review | An agent; the orchestrator overrides past two rejections |
| code-review | People on GitHub: two approvals, one from a code owner. `changes_requested` sends the ticket back to build with the review comments as its brief. Skipping is a person's (`skip: person`) |
| merged | A person merges on GitHub; outward, so interventions settle first (5.3) |
| release | The release manager. Skipping is a person's |
| publish | A person (`publish: person` in the workspace settings), so the two `skip: person` reservations can't be published away by the orchestrator |

Branch protection on GitHub still enforces the approvals. The engine's
station mirrors them so the board shows where each ticket waits and who it
waits on. `deploy` uses a generic `command/v1` adapter, whose config declares
which commands are outward: `production` is `outward: fresh` (every deploy is
a new act), so the command receives the effect's idempotency key and
`reconcile` answers from `./deploy status`; without a reconcile command, its
unknown outcomes hold for `Settle`.

### 8.3 A personal assistant: inbox triage, draft, approve, send

Tokens are mail threads, created by intake. A daily digest batches what needs
the person's attention but no reply.

```yaml
workspace:
  limits: { pools: { agents: 2, work: 2 } }
  publish: orchestrator
---
schema: 3
name: assistant-inbox
adapters:
  mail: { version: mail/v1, config: { account: me@example.com, folder: INBOX } }
  calendar: { version: calendar/v1, config: { calendar: primary } }
intake:
  - { op: mail.inbox, filter: "is:unread -category:promotions", to: triage }   # a new thread only; a reply on a
                                                                              # live thread answers its wait
stations:
  - id: triage
    kind: agent
    op: mail.triage
    pool: agents
    models: [claude:sonnet]
    routes: { reply: draft, schedule: propose-times, fyi: digest, ignore: archive }
  - { id: propose-times, kind: command, op: calendar.free_slots, pool: work, routes: { passed: draft } }
  - { id: draft, kind: agent, op: mail.draft, pool: agents, models: [claude:opus], routes: { passed: approve } }
  - id: approve
    kind: decision
    deciders: [person:me]
    routes: { approved: send, edited: send, rejected: draft, drop: archive }
    policy: { skip: orchestrator }              # the orchestrator may send routine replies for me
  - { id: send, kind: command, op: mail.send, pool: work,          # outward, repeat: fresh
      routes: { passed: await-reply } }
  - { id: await-reply, kind: wait, op: mail.reply, policy: { patience: 3d },
      routes: { replied: triage, timeout: follow-up } }
  - { id: follow-up, kind: agent, op: mail.draft_follow_up, pool: agents,
      routes: { passed: approve, not_needed: archive } }
  - { id: archive, kind: command, op: mail.archive, pool: work, routes: { passed: done } }
  - id: digest
    kind: batch
    form: { window: "daily 08:00" }
    routes: { passed: archive }
    stations:
      - { id: summarize, kind: agent, op: mail.summarize, models: [claude:sonnet] }
      - { id: deliver, kind: command, op: mail.send_digest }       # outward, repeat: once (per batch)
```

Trace:

- Intake creates a token at `triage`. `reply` → `draft`; `schedule` →
  `propose-times` → `draft`; `fyi` → `digest`; `ignore` → `archive`. Any
  other label holds.
- `draft` → `approve`. `approved` or `edited` (the person changed the draft;
  the edit is the new evidence) → `send`; `rejected` → `draft` with the
  person's note as brief; `drop` → `archive`.
- `send` passed (crossing appended) → `await-reply`; `refused` (the server
  rejected it; not crossed) holds.
- `await-reply` `replied` → `triage`: a new round on the same thread (5.3).
  `timeout` → `follow-up`, which answers `passed` → `approve` (a follow-up
  draft for the person) or `not_needed` → `archive`.
- `archive` passed → `done`: the thread closes, `crossed: true` if anything
  was sent. If a new message arrives on a closed thread, intake `Reopen`s
  the same token at `triage`.
- `digest`: `summarize` → `deliver` → `finish` → `digest` passed →
  `archive` → `done` for every FYI thread in it. `ignore` threads never
  enter the digest.

| Station | Who decides |
|---|---|
| triage | An agent's label |
| approve | The person, by default. The orchestrator may approve on their behalf with a reason (a reply confirming a meeting time it proposed), because `skip: orchestrator`. A stricter person sets `skip: person` |
| send | Outward, boundary `mail:send`, `repeat: fresh`, `dedupe: none`: each round needs a draft approved since the last send; an unknown result settles crossed only when a sent copy with that round's `Message-ID` is found, not crossed only on a recorded rejection with the connection closed, and otherwise holds without resending (5.3, with the second-reply, crash and follow-up walk-through) |
| deliver | Outward, `repeat: once` per digest; each day's digest is a new batch |

What this asks of the core, and gets: intake (tokens created by an adapter),
two adapters in one workflow, a batch formed by a time window rather than a
size, a wait with patience routing to a follow-up, an outward step that
isn't code, and an outward step that repeats by rounds. A window batch forms on the runtime's `Advance` tick, whose
`now_ms` is in the input, so replay forms the same batch.

### 8.4 Research and content: a weekly brief

Topics come from a board column; each becomes an article; articles for the
week publish together as one issue.

```yaml
workspace:
  limits: { pools: { agents: 3, work: 1 } }
  publish: orchestrator
---
schema: 3
name: weekly-research-brief
adapters:
  files: { version: files/v1, config: { root: content/ } }
  cms: { version: cms/v1, config: { site: blog, list: weekly } }
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
      routes: { rejected: write, passed: issue }, policy: { rework: 2 } }
  - id: issue
    kind: batch
    form: { window: "weekly fri 09:00", size: 6, order: priority }
    on_reject: return_suspects
    routes: { returned: write, passed: archive }
    stations:
      - { id: assemble, kind: command, op: cms.assemble }
      - { id: edit, kind: agent, op: cms.edit_issue, models: [claude:opus] }
      - { id: editor, kind: decision, deciders: [person:editor, orchestrator], needs: 1,
          routes: { approved: publish, rejected: rejected } }
      - { id: publish, kind: command, op: cms.publish,                 # outward, repeat: once
          routes: { passed: measure } }
      - { id: measure, kind: wait, op: cms.stats, policy: { patience: 7d },
          routes: { measured: finish, timeout: finish } }
  - { id: archive, kind: command, op: files.archive, routes: { passed: done } }
```

Trace:

- Intake creates a token at `research` → `outline` → `pick`. `approved` →
  `write`; `rejected` → `research` with the reason as brief; `drop` →
  `archive` → `done`.
- `write` → `fact-check`; `rejected` → `write` (third holds), passed →
  `issue`, where it waits for Friday's batch.
- `issue`: `assemble` → `edit` → `editor`. `approved` → `publish`;
  `rejected` → `rejected`, so `return_suspects`: the articles the editor
  marked leave along `returned` → `write`, and the rest re-enter `editor`
  as a smaller issue. Any other verdict holds the issue.
- `publish` passed (crossing appended to the batch and every member) →
  `measure`; a not-crossed `refused` holds. `measure` answers `measured` or,
  after 7 days, `timeout`; both → `finish` → `issue` passed → `archive` →
  `done`. `publish` is `repeat: once`: a correction is a new ticket.

| Station | Who decides |
|---|---|
| pick | The orchestrator, on the outline |
| fact-check | An agent of a different model from the writer's; two rejections hold |
| issue.editor | The editor or the orchestrator, whichever answers first (`needs: 1`) |
| issue.publish | Outward: a lost publish settles from the CMS before any pull or cancel |

### 8.5 What the walkthroughs changed in the model

Writing and then tracing these forced seven changes, each now in sections
2-5: intake (8.3, 8.4); several adapters per workflow (8.3); time-window
batch formation (8.3, 8.4); decisions with counts and groups, fed by an
adapter source (8.2); explicit terminal targets `done` and `finish`, with
"nothing closes by falling through" (all four); workspace settings as their
own document, so `publish: person` is stated rather than implied (8.2); and
outward operations that repeat by rounds (`repeat: fresh`, 8.3), while a
push stays `repeat: once`. Nothing in the four layouts needed a core concept
named after code.

## 9. Every action still reserved to a person

The engine rule is that the orchestrator may do anything above, including
skipping any station, with a recorded reason. These are the only exceptions
this design keeps. Each is a question for the owner.

**Q1. Publishing a workflow.** *Reason to keep it:* a workflow decides what
every later token goes through, including which stations a person must
decide (Q7); an orchestrator that can publish can remove those. *Reason to
drop it:* the owner wants the orchestrator to run the process, and preview
(7.5) makes a change's effect exact before it applies.
*Answered (owner, Nov 15, ov-1267): dropped.* A workspace's own orchestrator
publishes its workflow and re-pins it, from its pane, with
`workspace set-workflow --reason`. Builders, reviewers and other agents are
refused, and so is another workspace's orchestrator. A publish with no reason
is refused. Each receipt records the actor (person, device, pane, and that it
was the orchestrator), the reason, and the pins before and after. The owner
sees each one on an informational In Review card, and can restore the
previous bundle with `workflow rollback`. The owner reserves publishing to a
person with `workspace set <ws> --orchestrator-publishes-workflow off` (the
`publish: person` setting above). Only the owner in person can change that
setting.

What the orchestrator publishes still runs only what the owner already ran
(`docs/engine-hooks.md`, ov-1201): no new command, environment or hook, and no
approval in front of one taken away. Those changes go through a draft a person
approves. The one exception is a re-pin of the engine's own hook files after
an app update. This build writes those files, so the orchestrator publishes
that move itself (`workspace re-pin`). Without that exception, the engine
stopped for about 2.5 hours on Nov 15 while a draft waited. A re-pin that also
takes up an edit to any other pinned file still needs a person.

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
read outside the engine, which the event records, and to the standard of 3.2:
`crossed: true` with positive evidence of that exact round (the ref value, a
sent copy with the round's message id, a reply quoting it);
`crossed: false` only with an authoritative rejection and proof the attempt
is over. Absence ("not in the sent folder") is not evidence either way, so
with nothing better the token stays held. A person can always settle too,
and may settle on judgment (they asked the recipient); the event records
that it was a person's judgment.

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
   each item's coding fields into the token's `coding` data, each member's
   candidate into its `Membership::data`, each archive entry's `candidate` and
   `pushed` into `Archived::data`, and each kept effect's `candidate`,
   `members`, `tree` and `ci_retry` into `Effect::data` (11.1). Also adds
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

Kept from the first pass, with its limits renamed. A start needs room at
three levels (station, workspace pool, host), so there is one case per level
in which that level alone is full and the other two have room, and each
asserts both that nothing starts and that the start comes on the event that
frees that level.

- **Before.** One workspace: pool `agents: 4`, station `build` (pool
  `agents`) capacity 4. Twelve tickets `Running` at build: A1-A4 `admitted`,
  P1-P8 `pending` in that outbox order; P6 and P7 are speed tickets. A second
  workspace holds one admitted build. Host limit `agents: 5`.
- **Cutover.** Build `working` is exactly A1-A4 with their original effect
  ids, runs, fences and resource generations, outbox rows still `admitted`.
  The build queue is P1-P8 in order; their rows are `suppressed`, a claim
  returns nothing, an `Observe` is refused. `Genesis` emitted nothing.
  Counts: station 4 of 4, pool 4 of 4, host 5 of 5. Each case below starts
  from this state with `Prioritize { P6, 10 }` and `Prioritize { P7, 10 }`
  (queue P6, P7, P1-P5, P8), which emit nothing.
- **Only the workspace pool binds.** `SetCapacity { station: build, 8 }`:
  nothing (pool and host full). The other workspace's build ends: host 4 of
  5, station 4 of 8, pool 4 of 4, and still nothing is emitted: the pool
  alone blocks. `SetCapacity { pool: agents, 5 }`: exactly one new build
  effect, for P6, on that event, new id, fence `old + 1` (station 5 of 8,
  pool 5 of 5, host 5 of 5). `Observe` A2 passed: one effect, for P7.
- **Only the station binds.** A person clears the host limit (a logged
  input); `SetCapacity { pool: agents, 8 }`: nothing is emitted, because
  station `build` is 4 of 4 while the pool is 4 of 8 and there is no host
  limit: the station alone blocks. `SetCapacity { station: build, 5 }`: P6
  starts on that event; P7 waits (station 5 of 5). `Observe` A2 passed: P7.
- **Only the host limit binds.** `SetCapacity { pool: agents, 8 }`,
  `SetCapacity { station: build, 8 }`: nothing is emitted, though station and
  pool each have four free places: host 5 of 5 alone blocks. `Observe` A2
  passed: only P6 starts (host 5 of 5 again). The other workspace's build
  ends: exactly one start, P7, on that event; P1 still waits with station and
  pool places free.
- **No host limit, default limits.** A person clears the host limit: the
  other workspace's build no longer counts. `Observe` A2 passed starts P6 and
  nothing more (station and pool 4 of 4).
- **Crash** inside the cutover transaction leaves the old store under the old
  incarnation; after commit, before any claim, restart reconciles A1-A4 first
  and no P effect exists.
- **A push that landed, its `Observe` lost**: the walk-through of 5.3, run as
  written, plus the refused-push and unreadable-remote variants.
- **A publish mid-flight:** after the cutover, on a layout with `land.checks`
  routing `passed` to `land.gate`, publish one that puts `group-review`
  before `land.gate` and removes `land.checks` with
  `moves: { land.checks: { to: land.group-review, labels: { red: rejected } } }`,
  while batches B1 and B2 are working at `land.checks` and B3 is queued for
  it. Assert: B3 moves to `group-review`'s queue in the publish event; B1
  answers `passed` and **arrives at `group-review`** (queued, not at
  `gate`); B2 answers `red` and starts `on_reject: bisect`, not a hold; a
  `conflict` answer from a variant B2 would hold at `group-review`. The
  preview taken before publishing lists exactly these three outcomes from
  `retired_route` and equals the state after.
- **Two publishes over one retired station:** as above, but before B1
  answers, a second publish replaces `group-review` with `review-2`
  (`moves: { land.group-review: { to: land.review-2 } }`). Assert: the
  retired `land.checks` translation now reads `to: land.review-2`; B1's
  `passed` arrives at `review-2`; a publish that removed `review-2` without a
  `moves` entry is refused `Stranded` while `land.checks` is still occupied;
  the retired station is deleted when empty.
- **A rename mid-step:** rename `review` to `code-review` while a token is
  working there. Assert: no retired station; the working token and every
  visit are re-keyed; its `rejected` answer follows the new layout's
  `code-review` routes.
- **Rounds:** the 5.3 email walk-through with the `test/mail` adapter
  (`dedupe: none`; its fake server can accept, reject, file or not file a
  sent copy, and keep a submission open), including a `start` of `send`
  with round 1's draft refused `Evidence`, and a layout that routes from
  `land.push` `passed` back to `land.base` refused by `validate`. The crash
  after the second send runs in four variants, each with the person's
  `Cancel` recorded during `Settling`:
  - *sent copy found:* crossing 2 appended (boundary `mail:send`, round 2,
    the effect's id), cancel overruled `Crossed`, M4 at `await-reply`;
  - *accepted, no sent copy visible:* M4 `Held { Uncertain }` with the
    cancel in the hold; no new effect is emitted for round 2 (the outbox
    holds exactly one row for `(M4, mail:send, 2)`); no crossing 2; a
    `Settle { crossed: false }` from the orchestrator citing only the empty
    sent folder is refused `Evidence`. The fake server then files the copy;
    the next `Advance` reconcile settles crossed as in the first variant;
  - *submission still open:* the same hold, and a permanent rejection
    reported while the fake connection stays open still answers Unknown;
    once the connection closes, the rejection settles not crossed;
  - *rejected and closed:* the cancel applies; M4 closes with crossing 1
    only, `crossed: true`.
- **A rename between rounds:** M4 has crossed round 1 at `send` and waits at
  `await-reply`. Publish `moves: { send: { rename: send-reply } }`. Assert:
  crossing 1 still has boundary `mail:send`, round 1, its effect id and
  receipt, with `station` re-keyed to `send-reply`; on `replied`, the next
  `send-reply` effect has idempotency `(M4, mail:send, 2)`; a start with D1
  is refused `Evidence`.
- **A rename during an admitted send, then a crash:** round 2's effect at
  `send` is admitted; publish the same rename; the daemon dies. Assert: the
  effect's id and idempotency `(M4, mail:send, 2)` are unchanged by the
  publish; after restart reconcile looks for round 2's `Message-ID`; the
  settled crossing is boundary `mail:send`, round 2, at `send-reply`; the
  next round is 3.
- **Replacing an outward station:** replace `send` by `send-relay` (op
  `mail.send_relay`, boundary `mail:send`) with `boundary: keep`: rounds
  continue at 3. The same publish without a `boundary` entry, or with
  `boundary: keep` for an op of another boundary, is refused. For coding,
  replacing `land.push` by a `coding.land` station on the same target with
  `boundary: keep`: a `Send` of a crossed batch to `land.base` is still
  refused `Crossed`. Replacing it by a push to `release` without
  `boundary: new` is refused, and the preview lists every batch that
  crossed `coding:target:main`.
- **Crossed work across a replaced push:** the worked example of 7.3, run
  as written. Assert: B9 queues at `land.land`; B7's `passed` records
  crossing 1 and B7 queues at `land.ci`, never `land.land`, and no second
  push effect exists; `Send { B7, to: land.land }` and a `Release` in place
  of a `Crossed` hold are refused `Crossed`. Variants: unknown then settled
  crossed (to `land.ci`) and settled not crossed (to `land.land`); a layout
  ending at `land.land` (B7's members close `crossed: true`, listed in the
  preview); `after: hold` (B7 held `Crossed` at the retired `land.push`,
  which is not deleted). The same assertion for a crossed batch queued at a
  renamed `ci` the new layout puts before `land.land`, and for `land.push`
  split into `land.prepare` then `land.land` and merged with `land.gate`. A
  publish whose `after[K]` reaches a K station is refused by `validate`.
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
| `members` | `Batch::members`: `Membership { token, joined, data }`; each `Member::candidate` goes to `data["coding"]` | A candidate is coding's |
| `group_of` | Removed | A batch is not an item with a definition |
| `pushed` | `Batch::crossings` (core, the commit as an opaque artifact) plus coding data `pushed` | Crossing is core; what crossed is the adapter's |
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

**`Effect` (`state.rs:252-283`), `Member`, `Archived`, `ArchivedGroup`
(`state.rs:125-166`):**

| Today | In the new model | Why |
|---|---|---|
| `Effect::id`, `run`, `epoch`, `fence`, `created_ms`, `model`, `brief` | Same fields on the core `Effect` | Core |
| `Effect::item` | `Effect::subject` (`Token` or `Batch`) | A batch isn't an item |
| `Effect::resource` | `Effect::resources`, opaque names from the catalog | |
| `Effect::definition_pin`, `policy_pin` | `Effect::op.spec` | A record of what ran, not a pin |
| `Effect::kind` (`Execute(StageKind)`, `Stop`) | `EffectKind::{Execute(OpRef), Probe(OpRef), Stop}` | Operations are the adapter's |
| `Effect::operation` (`OperationSpec`, `definition.rs:54-58`) | The published station's operation, named by `op`, digested into `op.spec` | |
| `Effect::with` | `Effect::followers` | |
| `Effect::members` (`Vec<Member>`) | `Effect::members` (keys only) plus each candidate in `Effect::data["coding"]` | |
| `Effect::candidate`, `tree`, `ci_retry` | `Effect::data["coding"]`, filled by the adapter's `start` | Coding's |
| New | `Effect::idempotency`: `(subject, boundary, round)`, fixed when the effect is made | Outward rounds (5.3); no publish changes it (7.3) |
| `pushed`, the only crossing record today | `Crossing { boundary, round, effect, station, artifact, receipt, revision }` | Identity is the boundary and round, so renames and replacements keep it (7.3) |
| `Member { item, candidate }` | `Membership { token, joined, data }` | |
| `Archived::phase` | `Archived::closed` (`Done`, `Cancelled`) plus `reason` | |
| `Archived::candidate` | `Archived::data["coding"]` | Coding's |
| `Archived::group` (`ArchivedGroup { name, tickets, pushed, since_ms }`) | `Archived::batch` (`ArchivedBatch { name, members, since_ms }`); `pushed` becomes `Archived::crossings` plus `data["coding"]` | |

`State`: `schema` 3; `workspace`, `revision`, `epoch`, `seen`,
`source_sequences` unchanged; `archive` keeps its keys with the new
`Archived` shape above; `items` splits into `tokens` and
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
