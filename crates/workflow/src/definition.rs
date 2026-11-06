use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, BTreeMap};
use crate::{digest, Refusal};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Progression { Automatic, AwaitApproval { principal: String } }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reviewer { Dispatch, Orchestrator(String), Person(String) }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StageKind {
    Provision, Build, Check, Review(Reviewer), Land, Ci, Tracker,
    Action(String), Evaluate(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationSpec {
    pub adapter_version: String,
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage {
    pub id: String,
    pub kind: StageKind,
    pub progression: Progression,
    pub max_attempts: u8,
    pub operation: OperationSpec,
}
/// A finite forward sequence; rejected evaluators may take one bounded, pinned rework edge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    pub schema: u32,
    pub source: String,
    pub stages: Vec<Stage>,
    pub max_reworks: u8,
    pub rework_target: Option<String>,
}
impl Definition {
    pub fn validate(&self) -> Result<(), Refusal> {
        let mut ids = BTreeSet::new();
        if self.schema != 1 || self.source.is_empty() || self.source.len() > 256
            || self.stages.is_empty() || self.stages.len() > 16 || self.max_reworks > 2
        { return Err(Refusal::Definition); }
        let mut candidate_available = matches!(self.stages[0].kind, StageKind::Tracker);
        for stage in &self.stages {
            if !valid_id(&stage.id) || !ids.insert(&stage.id)
                || stage.max_attempts == 0 || stage.max_attempts > 5
            { return Err(Refusal::Definition); }
            if !valid_id(&stage.operation.adapter_version) || stage.operation.arguments.len() > 32
                || stage.operation.arguments.iter().any(|a| a.len() > 4096 || a.contains('\0'))
                || stage.operation.environment.len() > 32
                || stage.operation.environment.iter().any(|(k, v)| !valid_id(k) || v.len() > 4096 || v.contains('\0'))
            { return Err(Refusal::Definition); }
            match stage.kind {
                StageKind::Build | StageKind::Action(_) => candidate_available = true,
                StageKind::Check | StageKind::Review(_) | StageKind::Land | StageKind::Ci
                    | StageKind::Tracker | StageKind::Evaluate(_) if !candidate_available => {
                    return Err(Refusal::Definition);
                }
                _ => {}
            }
            if let Progression::AwaitApproval { principal } = &stage.progression {
                if !valid_id(principal) { return Err(Refusal::Definition); }
            }
            if let StageKind::Action(binding) | StageKind::Evaluate(binding) = &stage.kind {
                if !valid_id(binding) { return Err(Refusal::Definition); }
            }
            if let StageKind::Review(Reviewer::Person(p) | Reviewer::Orchestrator(p)) = &stage.kind {
                if !valid_id(p) { return Err(Refusal::Definition); }
            }
        }
        // Sequence order is definition policy, not a global coding/reviewer policy.
        let target = self.rework_target.as_ref().map(|id| self.stages.iter().position(|s| &s.id == id));
        if self.max_reworks == 0 {
            if target.is_some() { return Err(Refusal::Definition); }
        } else {
            let Some(Some(target)) = target else { return Err(Refusal::Definition); };
            if !matches!(self.stages[target].kind, StageKind::Build | StageKind::Action(_)) {
                return Err(Refusal::Definition);
            }
            let evaluators: Vec<_> = self.stages.iter().enumerate().filter(|(_, s)|
                matches!(s.kind, StageKind::Review(_) | StageKind::Evaluate(_))).collect();
            if evaluators.is_empty() || evaluators.iter().any(|(index, _)| target >= *index) {
                return Err(Refusal::Definition);
            }
        }
        Ok(())
    }
    pub fn pin(&self) -> String { digest(self) }
    pub fn coding() -> Self {
        let kinds = [StageKind::Provision, StageKind::Build, StageKind::Check,
            StageKind::Review(Reviewer::Dispatch), StageKind::Land, StageKind::Ci];
        Self { schema: 1, source: "coding/v1".into(), max_reworks: 2, rework_target: Some("stage-1".into()),
            stages: kinds.into_iter().enumerate().map(|(i, kind)| Stage {
                id: format!("stage-{i}"), kind, progression: Progression::Automatic,
                max_attempts: 2,
                operation: OperationSpec { adapter_version: "fixture/v1".into(),
                    arguments: if i == 2 { vec!["cargo".into(), "test".into(), "--locked".into()] }
                        else { vec![format!("coding-operation-{i}")] }, environment: BTreeMap::new() },
            }).collect() }
    }
    pub fn tracker(principal: &str) -> Self {
        Self { schema: 1, source: "tracker/v1".into(), max_reworks: 0, rework_target: None,
            stages: vec![Stage { id: "checklist".into(), kind: StageKind::Tracker,
                progression: Progression::AwaitApproval { principal: principal.into() },
                max_attempts: 1, operation: OperationSpec { adapter_version: "tracker/v1".into(),
                    arguments: vec![], environment: BTreeMap::new() } }] }
    }
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_:./".contains(&b))
}
