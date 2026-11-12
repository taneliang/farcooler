//! CI rules only decide. The reducer spends the rerun before emitting its outside side effect.
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CiPoint { #[serde(rename = "ci.result")] Result }
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiFallback { #[default] Accept }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CiPolicy {
    pub point: CiPoint,
    pub step: String,
    pub rule: CiRule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<CiFallback>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CiRule { pub rerun_failed: RerunFailed }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RerunFailed { pub jobs: Vec<String>, pub times: u8 }
/// Facts bound to the CI observation. An absent or empty jobs list never authorizes a rerun.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CiRun { pub id: u64, pub attempt: u32, pub failed_jobs: Vec<String> }
impl CiRun {
    pub fn valid(&self) -> bool {
        self.id > 0 && self.attempt > 0 && self.attempt < u32::MAX && self.failed_jobs.len() <= 256
            && self.failed_jobs.iter().all(|j| valid_job(j))
    }
}
fn valid_job(job: &str) -> bool { !job.trim().is_empty() && job.len() <= 256 && !job.chars().any(char::is_control) }
/// Kept on the item and effect, so a restart cannot replace the requested run or refund the count.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiRetry {
    pub run: CiRun,
    pub pending: bool,
    pub policy: String,
    pub revision: u64,
    pub input_digest: String,
}
impl Definition {
    pub(crate) fn validate_ci_policies(&self) -> Result<(), Refusal> {
        if self.policies.is_empty() { return Ok(()); }
        if self.schema != 2 || self.policies.len() > 16 { return Err(Refusal::Definition); }
        let mut targets = BTreeSet::new();
        for (name, policy) in &self.policies {
            let rule = &policy.rule.rerun_failed;
            if !valid_id(name) || !valid_id(&policy.step) || !targets.insert(&policy.step)
                || rule.times != 1 || rule.jobs.is_empty() || rule.jobs.len() > 64
                || rule.jobs.iter().any(|j| !valid_job(j))
                || rule.jobs.iter().collect::<BTreeSet<_>>().len() != rule.jobs.len()
            { return Err(Refusal::Definition); }
            let stages: Vec<_> = self.stages.iter().chain(self.groups.values().flat_map(|g| g.steps.iter()))
                .filter(|s| s.id == policy.step).collect();
            if stages.is_empty() || stages.iter().any(|s| s.kind != StageKind::AwaitCi) {
                return Err(Refusal::Definition);
            }
        }
        Ok(())
    }
}
pub(crate) fn rerun(item: &mut Item, id: &str, input: &Input, run: Option<&CiRun>, effects: &mut Vec<Effect>) -> Result<bool, Refusal> {
    let stage = &item.definition.stages[item.stage];
    let Some((name, policy)) = item.definition.policies.iter().find(|(_, p)| p.step == stage.id) else { return Ok(false) };
    let Some(run) = run.filter(|r| r.valid() && !r.failed_jobs.is_empty()) else { return Ok(false) };
    let rule = &policy.rule.rerun_failed;
    let count = item.ci_reruns.get(&stage.id).copied().unwrap_or(0);
    if count >= rule.times || !run.failed_jobs.iter().all(|j| rule.jobs.contains(j)) { return Ok(false); }
    item.ci_reruns.insert(stage.id.clone(), count + 1);
    item.ci_retry = Some(CiRetry { run: run.clone(), pending: true, policy: name.clone(),
        revision: input.expected_revision, input_digest: digest(&input.command) });
    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
    item.phase = Phase::Running;
    crate::reducer::emit(item, id, input, EffectKind::Execute(StageKind::RerunCi), effects);
    Ok(true)
}
