//! Admission is pure: the runtime supplies the current load and durable item inventory.
use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Budget {
    Fixed(usize),
    Unlimited,
    /// One-minute load average in thousandths, avoiding floating-point policy pins.
    Load { below_milli: u64 },
}
impl Budget {
    pub fn permits(&self, active: usize, load_milli: Option<u64>) -> bool {
        match self {
            Self::Fixed(n) => active < *n,
            Self::Unlimited => true,
            Self::Load { below_milli } => load_milli.is_some_and(|load| load < *below_milli),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartPolicy {
    pub columns: BTreeSet<String>,
    pub budget: Budget,
    /// Declared file knowledge: intersecting paths serialize whole runs until they finish.
    #[serde(default)]
    pub files: BTreeMap<String, BTreeSet<String>>,
    /// Additional dependencies, alongside the board's blockers.
    #[serde(default)]
    pub dependencies: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    pub rotate_models: bool,
    #[serde(default)]
    pub auto_queue: bool,
}
impl StartPolicy {
    pub fn valid(&self) -> bool {
        !self.columns.is_empty() && self.columns.iter().all(|c| crate::TICKET_COLUMNS.contains(&c.as_str()))
            && self.files.keys().chain(self.dependencies.keys()).all(|k| crate::valid_id(k))
            && self.dependencies.values().flatten().all(|k| crate::valid_id(k))
            && self.files.values().flatten().all(|p| !p.is_empty() && !p.contains('\0'))
    }
    pub fn conflicts(&self, a: &str, b: &str) -> bool { self.conflicts_with(a, self, b) }
    /// Compare a waiting ticket's current file claims with an active run's pinned claims.
    pub fn conflicts_with(&self, a: &str, other: &Self, b: &str) -> bool {
        match (self.files.get(a), other.files.get(b)) {
            (Some(a), Some(b)) => !a.is_disjoint(b),
            _ => false,
        }
    }
    pub fn models(&self, definition: &crate::Definition, ordinal: usize) -> BTreeMap<String, String> {
        definition.models.iter().filter_map(|(step, list)| {
            list.get(if self.rotate_models { ordinal % list.len().max(1) } else { 0 })
                .map(|m| (step.clone(), m.clone()))
        }).collect()
    }
}
/// Defaults preserve old definitions. These are execution policy, distinct from wire size/identity bounds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub retries: u8,
    pub back_edges: u8,
    pub send_backs: u8,
    pub models: usize,
    pub reworks: u8,
    pub attempts: u8,
    pub stages: usize,
    pub routes: usize,
    pub group_members: u8,
}
impl Default for Limits {
    fn default() -> Self {
        Self { retries: crate::MAX_RETRIES, back_edges: crate::MAX_BACK_EDGE_LIMIT,
            send_backs: crate::MAX_SEND_BACKS, models: crate::MAX_MODELS, reworks: 2,
            attempts: 5, stages: 16, routes: 8, group_members: crate::MAX_GROUP_MEMBERS }
    }
}
