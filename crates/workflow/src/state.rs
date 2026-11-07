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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rework_history: Vec<ReworkDecision>,
    /// Back edges taken so far, by the id of the step that took them (schema 2).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub back_edges: BTreeMap<String, u8>,
    /// How many times a controller has retried this item after it failed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub retries: u8,
}
fn is_zero(n: &u8) -> bool { *n == 0 }
/// A failed item can be retried this many times, so a bad step cannot loop forever.
pub const MAX_RETRIES: u8 = 3;
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
pub enum Outcome { Passed(Evidence), Rejected(Evidence), Labeled(String, Evidence), SafeFailure, Uncertain, Stopped { operation: String, receipt: String } }
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
