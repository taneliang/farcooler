//! The pipeline's state (docs/engine-pipeline.md 2.3). No type here names a kind of work: what a token
//! is about lives in `data`, owned by an adapter, stored and passed but never read by the core.
use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};
use super::layout::{Kind, Layout};

/// A full station id: a top-level id (`triage`) or `<batch station>.<inner id>` (`digest.summarize`).
pub type StationId = String;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pipeline {
    pub schema: u32,
    pub workspace: String,
    pub revision: u64,
    pub epoch: u64,
    /// The layout in force; `None` until the first `Publish`.
    pub workflow: Option<Published>,
    pub stations: BTreeMap<StationId, Station>,
    /// Stations a publish removed while work was still running in them (7.3).
    pub retired: BTreeMap<StationId, Retired>,
    pub pools: BTreeMap<String, Pool>,
    pub tokens: BTreeMap<String, Token>,
    pub batches: BTreeMap<String, Batch>,
    pub seen: BTreeSet<String>,
    pub source_sequences: BTreeMap<String, u64>,
    pub archive: BTreeMap<String, Archived>,
    /// Numbers batch ids and effect ids; part of the state so replay names them the same.
    pub sequence: u64,
}
impl Pipeline {
    /// An empty pipeline with the workspace's pools (section 6). Nothing runs until a `Publish`.
    pub fn new(workspace: &str, epoch: u64, pools: &BTreeMap<String, u16>) -> Self {
        Self { schema: 3, workspace: workspace.into(), revision: 0, epoch, workflow: None,
            stations: BTreeMap::new(), retired: BTreeMap::new(),
            pools: pools.iter().map(|(k, n)| (k.clone(), Pool { capacity: *n, held_by: BTreeSet::new() })).collect(),
            tokens: BTreeMap::new(), batches: BTreeMap::new(), seen: BTreeSet::new(),
            source_sequences: BTreeMap::new(), archive: BTreeMap::new(), sequence: 0 }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Published {
    pub digest: String,
    pub layout: Layout,
    pub adapters: BTreeMap<String, AdapterRef>,
    /// Where work that crossed a `repeat: once` boundary resumes when a placement fails the check (7.3).
    pub after: BTreeMap<String, After>,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterRef { pub version: String, pub config_digest: String }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum After { Station(StationId), Done, Hold }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Station {
    pub kind: Kind,
    /// Token keys or batch ids, head first.
    pub queue: Vec<String>,
    pub working: BTreeSet<String>,
    pub capacity: u16,
    pub held: Option<Hold>,
    pub draining: bool,
    /// A batch station formed by a window: the last window index it formed in.
    pub window: Option<i64>,
}
/// A station a publish removed while work was still running in it: only its translation, never its old routes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retired {
    pub kind: Kind,
    pub op: Option<String>,
    /// The pool its running work still holds places in until it answers.
    pub pool: Option<String>,
    pub working: BTreeSet<String>,
    pub to: StationId,
    pub labels: BTreeMap<String, Target>,
    /// A batch station removed with `unbatch`: its batch's members leave one by one to `to`.
    pub unbatch: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target { Station(StationId), Done, Finish, Rejected, Returned }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pool { pub capacity: u16, pub held_by: BTreeSet<String> }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub at: Location,
    pub priority: i32,
    pub data: BTreeMap<String, Opaque>,
    pub evidence: Option<Evidence>,
    pub crossings: Vec<Crossing>,
    /// Crossings before this index happened before the last `Reopen` and no longer bind (5.3).
    pub fresh_from: usize,
    pub brief: Option<String>,
    pub failure: Option<String>,
    pub models: BTreeMap<StationId, String>,
    pub dependencies: BTreeSet<String>,
    pub visits: Vec<Visit>,
    pub fence: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Batch {
    /// The batch station it was formed at.
    pub station: StationId,
    pub at: Location,
    pub members: Vec<Membership>,
    pub data: BTreeMap<String, Opaque>,
    pub crossings: Vec<Crossing>,
    /// The members the last rejecting or returning answer pointed at (the adapter's `suspects`, or a verdict's).
    pub suspects: Vec<String>,
    pub bisect: Option<Bisect>,
    pub visits: Vec<Visit>,
    pub fence: u64,
    pub since_ms: i64,
}
/// One token in a batch; what it joined with is the adapter's, copied from the token's data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership { pub token: String, pub joined: u64, pub data: BTreeMap<String, Opaque> }
/// A rejected batch re-running its rejecting station on halves of its members (4.5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bisect {
    pub station: StationId,
    pub trying: Vec<String>,
    pub waiting: Vec<Vec<String>>,
    pub cleared: Vec<String>,
    pub found: Vec<String>,
    /// The label the cleared members leave the station with once the search ends.
    pub passing: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opaque { pub version: String, pub value: serde_json::Value, pub digest: String }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence { pub artifact: String, pub manifest: String, pub receipt: String, pub spec: String }
/// One outward act that happened. Identity is `boundary` + `round`; `station` is history for the board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crossing { pub boundary: String, pub round: u32, pub effect: String, pub station: StationId,
    pub artifact: String, pub receipt: String, pub revision: u64 }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Archived {
    pub closed: Closure,
    pub reason: Option<String>,
    pub crossings: Vec<Crossing>,
    pub batch: Option<ArchivedBatch>,
    pub data: BTreeMap<String, Opaque>,
    /// A closed token's visits, so a `Reopen` keeps its history.
    pub visits: Vec<Visit>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Closure { Done, Cancelled }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchivedBatch { pub name: String, pub members: Vec<String>, pub since_ms: i64 }
/// One unit of work for the runtime. Whatever an adapter needs to run it is in `data`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effect {
    pub id: String,
    pub subject: Subject,
    pub station: StationId,
    pub run: String,
    pub epoch: u64,
    pub fence: u64,
    pub resources: Vec<String>,
    pub kind: EffectKind,
    pub created_ms: i64,
    pub model: Option<String>,
    pub brief: Option<String>,
    pub members: Vec<String>,
    pub idempotency: String,
    pub data: BTreeMap<String, Opaque>,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Subject { Token(String), Batch(String) }
impl Subject {
    pub fn key(&self) -> &str { match self { Subject::Token(k) | Subject::Batch(k) => k } }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Location {
    Resting { column: String },
    Queued { station: StationId },
    Working { station: StationId, work: Work },
    Stopping { station: StationId, work: Work, then: Box<Then> },
    Settling { station: StationId, work: Work, then: Box<Then> },
    Awaiting { station: StationId, decision: Decision },
    InBatch { batch: String },
    Held { station: StationId, hold: Hold },
    Closed { crossed: bool },
}
impl Location {
    /// The station this location is at, if any.
    pub fn station(&self) -> Option<&str> {
        match self {
            Location::Queued { station } | Location::Working { station, .. } | Location::Stopping { station, .. }
            | Location::Settling { station, .. } | Location::Awaiting { station, .. }
            | Location::Held { station, .. } => Some(station),
            _ => None,
        }
    }
}
/// Where an intervention sends work once its running step has stopped (5.2) or settled (5.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Then {
    Arrive { station: StationId, by: String, reason: Option<String> },
    Hold(Hold),
    Leave { label: String, by: String, reason: Option<String> },
    Close { by: String, reason: Option<String> },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Work {
    pub run: String,
    pub attempt: u8,
    /// The effect whose answer is awaited: the operation, or its stop.
    pub pending: Option<String>,
    pub op: OpRef,
    /// The original effect a stop is stopping.
    pub stopping: Option<String>,
    /// The answer was unknown: the step keeps its place until a later answer or a `Settle`.
    pub uncertain: bool,
    pub wait_since: Option<i64>,
    pub again: Option<Opaque>,
    /// Adapter-requested re-runs spent in this visit.
    pub reruns: u8,
    /// An outward step's round on its boundary, fixed when the effect was made.
    pub round: Option<u32>,
    pub resources: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpRef { pub adapter: String, pub op: String, pub spec: String }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision { pub run: String, pub deciders: Vec<String>, pub needs: u8, pub given: Vec<Verdict> }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict { pub by: String, pub label: String }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hold {
    pub reason: String,
    pub by: String,
    pub revision: u64,
    pub kind: HoldKind,
    /// Where the work was going when the hold stopped it; a `Release` in place sends it there.
    pub then: Option<Target>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HoldKind { Asked, PolicyLimit { policy: String }, StepFailed, Quarantined, Crossed { boundary: String } }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visit {
    pub station: StationId,
    pub arrived: u64,
    pub started: Option<u64>,
    pub left: Option<u64>,
    pub ended: Option<String>,
    pub by: String,
    pub reason: Option<String>,
    pub overrode: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Labeled { label: String, evidence: Evidence, data: Option<Opaque> },
    Failed { reason: Option<String> },
    Interrupted,
    Uncertain,
    Stopped { operation: String, receipt: String },
    Settled { operation: String, original: Box<Outcome>, receipt: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectKind { Execute(OpRef), Probe(OpRef), Stop { operation: String } }
