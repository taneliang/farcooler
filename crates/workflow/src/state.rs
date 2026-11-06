use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};
use crate::{Definition, StageKind, OperationSpec};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    Definition, Invalid, Duplicate, Stale, Unauthorized, Missing, Blocked, Resource,
    Phase, Evidence, Limit,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase { Blocked, WaitingApproval, WaitingReview, Running, Uncertain, Cancelling, Done, Failed, Cancelled }
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
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub schema: u32,
    pub workspace: String,
    pub revision: u64,
    pub epoch: u64,
    pub items: BTreeMap<String, Item>,
    pub seen: BTreeSet<String>,
    pub source_sequences: BTreeMap<String, u64>,
}
impl State {
    pub fn empty(workspace: String, epoch: u64) -> Self {
        Self { schema: 1, workspace, epoch, revision: 0, items: BTreeMap::new(),
            seen: BTreeSet::new(), source_sequences: BTreeMap::new() }
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
    Review { item: String, run: String, evidence: Evidence, passed: bool },
    Observe { item: String, effect: String, run: String, fence: u64, outcome: Outcome },
    Cancel { item: String },
    Resolve { item: String, effect: String, run: String, fence: u64, outcome: Outcome },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub candidate: String,
    pub manifest: String,
    pub receipt: String,
    pub policy_pin: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome { Passed(Evidence), Rejected(Evidence), SafeFailure, Uncertain, Stopped { operation: String, receipt: String } }
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
    pub kind: EffectKind,
    pub operation: OperationSpec,
    pub created_ms: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub state: State,
    pub effects: Vec<Effect>,
    /// Delivery metadata never participates in eligibility or approval.
    pub notices: Vec<String>,
}
