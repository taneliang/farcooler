use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};
use crate::{Definition, StageKind, OperationSpec};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    Definition, Invalid, Duplicate, Stale, Unauthorized, Missing, Blocked, Resource,
    Phase, Evidence, Limit,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase { Blocked, WaitingApproval, WaitingReview, Running, Uncertain, Cancelling, Done, Failed, Cancelled,
    /// At a `Join` step, not yet queued into a group. Holds no resource.
    WaitingJoin,
    /// Queued into a group (`grouped_in`). Holds no resource.
    Joined }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub definition: Definition,
    pub pin: String,
    pub policy_pin: String,
    pub dependencies: BTreeSet<String>,
    pub resource: String,
    pub stage: usize,
    pub iteration: u8,
    pub attempt: u8,
    pub fence: u64,
    pub phase: Phase,
    pub run: String,
    pub pending: Option<String>,
    pub candidate: Option<String>,
    pub last_evidence: Option<Evidence>,
    pub stopping: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rework_history: Vec<ReworkDecision>,
    /// Back edges taken so far, by the id of the step that took them (schema 2).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub back_edges: BTreeMap<String, u8>,
    /// How many times a controller has retried this item after it failed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub retries: u8,
    /// A group item only: the tickets it runs, each pinned to the candidate it joined with.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<Member>,
    /// A group item only: the join name and the pin of the definition whose tickets it holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_of: Option<GroupOf>,
    /// A group item only: the commit its push put on the target. From here its members' commits are
    /// landed, whatever CI says, so the group can no longer be cancelled and never ends as `rejected`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pushed: Option<String>,
    /// A group item only: the tickets it handed back when it ended, so a finished group still names
    /// the tickets it ran after `members` has been emptied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub handed: Vec<String>,
    /// A group item only: when it last changed step or phase, in the clock of the inputs that moved it.
    #[serde(default, skip_serializing_if = "is_zero_ms")]
    pub since_ms: i64,
    /// A joined ticket only: the group item it is queued into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grouped_in: Option<String>,
    /// A wait step only: when its first effect was made. A restart looks again with the same deadline, so
    /// patience survives it and a wait that never answers still ends in `timeout`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_since: Option<i64>,
    /// The model each build step runs with, by step id: the orchestrator's pick at Start, or the step's
    /// default (ov-966). Empty when the definition lists none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, String>,
    /// The worktree the orchestrator named at Start, and main's commit then (ov-967). `None`: a private tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<RunTree>,
    /// Each time the orchestrator sent the run back to its build step, oldest first (ov-968).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sent_back: Vec<SendBack>,
    /// Why the run was last sent back to build, by the orchestrator or a review, until that build passes. Every
    /// build effect carries it, so the agent is told (ov-968).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    /// A ticket that follows another's run (ov-970): its card sits where the lead's run is, and it closes when the
    /// run does. It has no run of its own while it follows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follows: Option<String>,
    /// A run's other tickets, which follow it (ov-970). One agent, one branch, one pull request for all of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub followers: Vec<String>,
}
/// One send-back: who asked (the input), when (the revision), from which step to which, on which run, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendBack { pub id: String, pub revision: u64, pub run: String, pub from: String, pub to: String, pub reason: String }
/// A run's worktree, named by the orchestrator at Start: one the app lists, on `branch`, made at `path` from
/// `base`, which is main as it was at Start (ov-967).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunTree { pub name: String, pub branch: String, pub path: String, pub base: String,
    /// The branch existed before the run, with work on it, and the run carries on from it (ov-968). `path` is the
    /// worktree that already has it checked out, or where Provision makes one on it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub adopted: bool }
fn is_false(b: &bool) -> bool { !*b }
impl RunTree {
    /// Bounded and plain. The runner checks the names as `worktree.create` does before it sends one.
    pub fn valid(&self) -> bool {
        let plain = |s: &str| !s.is_empty() && s.len() <= 256 && !s.contains('\0') && !s.bytes().any(|b| b.is_ascii_control());
        plain(&self.name) && plain(&self.branch) && plain(&self.path) && self.path.starts_with('/')
            && self.base.len() == 40 && self.base.bytes().all(|b| b.is_ascii_hexdigit())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member { pub item: String, pub candidate: String }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupOf { pub name: String, pub pin: String }
fn is_zero_ms(n: &i64) -> bool { *n == 0 }
fn is_zero(n: &u8) -> bool { *n == 0 }
/// A failed item can be retried this many times, so a bad step cannot loop forever.
pub const MAX_RETRIES: u8 = 3;
/// Live items a workspace holds; every way of adding one refuses at this count, so a step never commits past it.
pub const MAX_ITEMS: usize = 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub schema: u32,
    pub workspace: String,
    pub revision: u64,
    pub epoch: u64,
    pub items: BTreeMap<String, Item>,
    pub seen: BTreeSet<String>,
    pub source_sequences: BTreeMap<String, u64>,
    /// Items that finished and left `items`. A name stays taken, and a dependency on a
    /// finished item stays met. The whole record is in the event log (ov-953).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub archive: BTreeMap<String, Archived>,
}
/// What a later step needs to know about an item that has left live state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Archived {
    pub phase: Phase,
    pub candidate: Option<String>,
}
impl State {
    pub fn empty(workspace: String, epoch: u64) -> Self {
        Self { schema: 1, workspace, epoch, revision: 0, items: BTreeMap::new(),
            seen: BTreeSet::new(), source_sequences: BTreeMap::new(), archive: BTreeMap::new() }
    }
}
/// Admission is trusted only after the runtime authenticates this capability.
/// Attribution strings on untrusted wire payloads must never construct this value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Grant {
    Controller,
    Decision { principal: String },
    Operation { effect: String, run: String, fence: u64 },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    pub schema: u32,
    pub id: String,
    pub workspace: String,
    pub expected_revision: u64,
    pub epoch: u64,
    pub source: String,
    pub source_sequence: u64,
    pub now_ms: i64,
    pub grant: Grant,
    pub command: Command,
    /// Store-stamped occupancy of other workspaces, persisted for deterministic replay.
    #[serde(default)]
    pub busy_resources: BTreeSet<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    Advance,
    Enqueue { item: String, definition: Definition, policy_pin: String,
        resource: String, dependencies: BTreeSet<String>, initial_candidate: Option<String> },
    Approve { item: String, run: String, pin: String, candidate: Option<String> },
    Review { item: String, run: String, evidence: Evidence, passed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")] reason: Option<String> },
    Observe { item: String, effect: String, run: String, fence: u64, outcome: Outcome },
    Cancel { item: String },
    /// A controller runs a failed item's step again, within `MAX_RETRIES`.
    Retry { item: String },
    /// The orchestrator queues a ticket waiting at its `Join` step into the open group.
    Queue { item: String },
    /// The orchestrator pulls a queued ticket out of a group that has not left yet.
    Leave { item: String },
    /// The orchestrator moves a tracker-only ticket to a column: one of its steps, or `done` or `cancelled`.
    Move { item: String, to: String },
    /// The orchestrator starts a run on a tracker-only ticket: the same item takes a real definition.
    /// `models` picks a model for a build step, by step id, from the ones the definition lists (ov-966).
    Start { item: String, definition: Definition, policy_pin: String, resource: String,
        dependencies: BTreeSet<String>, initial_candidate: Option<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")] models: BTreeMap<String, String>,
        /// The worktree the run makes and works in (ov-967). `None` keeps the runtime's private tree.
        #[serde(default, skip_serializing_if = "Option::is_none")] tree: Option<Box<RunTree>>,
        /// Tickets at rest that follow this run instead of having runs of their own (ov-970).
        #[serde(default, skip_serializing_if = "Vec::is_empty")] with: Vec<String> },
    Resolve { item: String, effect: String, run: String, fence: u64, outcome: Outcome },
    /// The orchestrator sends a run back to its last build step, with a reason the agent is told (ov-968).
    SendBack { item: String, reason: String },
    /// The orchestrator reopens a closed ticket: it comes back under its own name as a ticket at rest, on
    /// `definition`, the tracker-only template at the column it reopens in (ov-968).
    Reopen { item: String, definition: Definition, policy_pin: String, resource: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub candidate: String,
    pub manifest: String,
    pub receipt: String,
    pub policy_pin: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome { Passed(Evidence), Rejected(Evidence), Labeled(String, Evidence), SafeFailure, Interrupted, Uncertain, Stopped { operation: String, receipt: String } }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectKind { Execute(StageKind), Stop { operation: String } }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effect {
    pub id: String,
    pub item: String,
    pub run: String,
    pub epoch: u64,
    pub fence: u64,
    pub resource: String,
    pub definition_pin: String,
    pub policy_pin: String,
    pub candidate: Option<String>,
    /// A group's tickets and the candidates they joined with, so none is resolved again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<Member>,
    pub kind: EffectKind,
    pub operation: OperationSpec,
    pub created_ms: i64,
    /// A build step's model, from the item's record (ov-966), so a launch reads it from the effect alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The item's named worktree (ov-967), so every step finds the tree from the effect alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<RunTree>,
    /// A build step only: why the run was sent back to it (ov-968), so the launch says so from the effect alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    /// The tickets that follow this item's run (ov-970), so the agent is told every ticket it works.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub with: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub state: State,
    pub effects: Vec<Effect>,
    /// Delivery metadata never participates in eligibility or approval.
    pub notices: Vec<String>,
}

/// Accepted borrowed text and the exact authenticated observation it describes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReworkDecision {
    pub id: String,
    pub principal: String,
    pub revision: u64,
    pub run: String,
    pub definition_pin: String,
    pub evidence: Evidence,
    pub iteration: u8,
    pub reason: String,
}
pub const MAX_REWORK_REASON_BYTES: usize = 4096;
pub fn valid_rework_reason(reason: &str) -> bool {
    !reason.trim().is_empty() && reason.len() <= MAX_REWORK_REASON_BYTES
        && !reason.chars().any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n'))
}
