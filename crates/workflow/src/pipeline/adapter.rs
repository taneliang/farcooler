//! The pure half of an adapter (docs/engine-pipeline.md 3.2). The core asks an adapter what its operations
//! are, whether one may start, and what an answer means; it never reads the adapter's data itself.
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use super::layout::{Kind, Layout};
use super::step::Refusal;
use super::types::{Crossing, Evidence, Opaque, Outcome, Subject, Visit};

/// Pure. Chosen by the version string in the published workflow; replay runs the same rules.
pub trait AdapterRules {
    fn version(&self) -> &'static str;
    /// Every operation this adapter offers under `config`.
    fn catalog(&self, config: &serde_json::Value) -> Vec<OpSpec>;
    /// A layout's use of this adapter's operations, beyond what the catalog already lets the core check.
    fn validate(&self, layout: &Layout, config: &serde_json::Value) -> Result<(), Vec<Problem>>;
    /// Whether `op` may start for this subject now, and with what resources and effect data.
    fn start(&self, op: &str, subject: &View, config: &serde_json::Value) -> Result<StartPlan, Refusal>;
    /// An answer becomes a label, new adapter data, and optionally "run me again with these facts".
    fn observe(&self, op: &str, subject: &View, outcome: &Outcome) -> Result<Applied, Refusal>;
    /// Which members of a batch an answer points at.
    fn suspects(&self, batch: &View, outcome: &Outcome) -> Vec<String>;
    /// Fields the boards show, read from this adapter's data.
    fn project(&self, data: &Opaque) -> Vec<(String, String)>;
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpSpec {
    pub name: String,
    pub kinds: Vec<Kind>,
    pub labels: Vec<String>,
    pub outward: Option<Outward>,
    /// Exclusive resources held while the operation runs.
    pub resources: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outward {
    /// What in the world the operation changes (`mail:send`); crossings and rounds belong to it.
    pub boundary: String,
    pub repeat: Repeat,
    pub dedupe: Dedupe,
    /// The labels that mean "it went out"; every other label means nothing crossed.
    pub crossing: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Repeat { Once, Fresh }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dedupe { Server, None }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartPlan {
    pub resources: Vec<String>,
    pub data: Option<Opaque>,
    /// Digest of the exact operation spec this start runs (`OpRef::spec`).
    pub spec: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applied { pub label: String, pub data: Option<Opaque>, pub again: Option<Opaque> }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem { pub station: Option<String>, pub message: String }
/// What an adapter may see of a token or batch: never another adapter's reasoning, only core facts and data.
#[derive(Clone, Copy, Debug)]
pub struct View<'a> {
    pub subject: &'a Subject,
    pub station: &'a str,
    pub data: &'a BTreeMap<String, Opaque>,
    pub evidence: Option<&'a Evidence>,
    pub crossings: &'a [Crossing],
    pub visits: &'a [Visit],
    pub members: &'a [String],
}
/// The adapters linked into this build, by version string.
#[derive(Default)]
pub struct Adapters<'a> { by_version: BTreeMap<&'static str, &'a dyn AdapterRules> }
impl<'a> Adapters<'a> {
    pub fn new(rules: &[&'a dyn AdapterRules]) -> Self {
        Self { by_version: rules.iter().map(|r| (r.version(), *r)).collect() }
    }
    pub fn get(&self, version: &str) -> Option<&'a dyn AdapterRules> { self.by_version.get(version).copied() }
}
