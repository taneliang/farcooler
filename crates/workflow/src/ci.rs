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
pub struct CiRun {
    pub id: u64,
    pub attempt: u32,
    pub failed_jobs: Vec<String>,
    /// Where each failed job failed (ov-1280). Absent from older receipts, which then never read as setup-only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_steps: Vec<FailedStep>,
}
impl CiRun {
    pub fn valid(&self) -> bool {
        self.id > 0 && self.attempt > 0 && self.attempt < u32::MAX && self.failed_jobs.len() <= 256
            && self.failed_jobs.iter().all(|j| valid_job(j))
            && self.failed_steps.len() <= 256
            && self.failed_steps.iter().all(|f| valid_job(&f.job) && valid_job(&f.step)
                && f.action.as_deref().is_none_or(valid_job))
    }
}
/// One failed job and where it failed: the job's step GitHub marks failed, and the innermost action the runner's log
/// marks `outcome=failure` in it (`__self.__dtolnay_rust-toolchain.__run_4`), when the log names one. A step that runs
/// a composite action hides which of its own steps failed; only the log's action id says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailedStep {
    pub job: String,
    pub step: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}
/// The policy name a setup-only rerun records. No published policy may take it.
pub const SETUP_RERUN: &str = "engine:setup-failure";
/// Actions that only fetch or prepare what the repository's own commands then use: a runner that can't reach the
/// network fails here before anything of the change has run. `owner/repo`; a trailing `-` is a prefix.
const SETUP_ACTIONS: &[&str] = &["actions/checkout", "actions/setup-", "actions/cache", "dtolnay/rust-toolchain",
    "actions-rust-lang/setup-rust-toolchain", "swatinem/rust-cache", "maxim-lobanov/setup-xcode",
    "android-actions/setup-android", "gradle/actions", "pnpm/action-setup", "oven-sh/setup-bun"];
const ARTIFACT_ACTION: &str = "actions/download-artifact";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failed { Fetch, Artifact, Own }
fn action_matches(action: &str, known: &str, separator: char) -> bool {
    let known = known.replace('/', &separator.to_string());
    match known.strip_suffix('-') {
        Some(prefix) => action.starts_with(prefix) && action.len() > prefix.len() + 1,
        None => action == known,
    }
}
/// What an action names: `actions/checkout` in a step called `Run actions/checkout@v4`, or the `__actions_checkout`
/// segment of a log's action id.
fn classify_action(action: &str, separator: char) -> Failed {
    let action = action.to_ascii_lowercase();
    if action_matches(&action, ARTIFACT_ACTION, separator) { return Failed::Artifact; }
    if SETUP_ACTIONS.iter().any(|known| action_matches(&action, known, separator)) { return Failed::Fetch; }
    Failed::Own
}
fn classify(failed: &FailedStep) -> Failed {
    if failed.step == "Set up job" { return Failed::Fetch; }
    if let Some(id) = &failed.action {
        // `__self` is the job's own step; each `__`-segment under it is an action it ran, outermost first. A failure
        // anywhere inside a setup action is that action's; a `__run…` segment outside one is the repository's command.
        for segment in id.split('.').skip_while(|s| *s == "__self") {
            let Some(name) = segment.strip_prefix("__") else { return Failed::Own };
            if name == "run" || name.starts_with("run_") { return Failed::Own; }
            let kind = classify_action(name, '_');
            if kind != Failed::Own { return kind; }
        }
        return Failed::Own;
    }
    match failed.step.strip_prefix("Run ").and_then(|s| s.split_once('@')) {
        Some((action, _)) => classify_action(action, '/'),
        None => Failed::Own,
    }
}
/// Every failed job failed in a setup step, before the repository's own commands ran (ov-1280): fetching a toolchain,
/// checking out, starting the job. An artifact download counts only beside such a failure in the same run, since
/// that's the producer whose products it wanted; alone, it may be a producer that ran and built nothing. A job with
/// no step on record is never setup-only.
pub fn setup_only(run: &CiRun) -> bool {
    let mut fetched = false;
    !run.failed_jobs.is_empty() && run.failed_jobs.iter().all(|job| {
        let mut steps = run.failed_steps.iter().filter(|f| &f.job == job);
        let (Some(step), None) = (steps.next(), steps.next()) else { return false };
        match classify(step) {
            Failed::Fetch => { fetched = true; true }
            Failed::Artifact => true,
            Failed::Own => false,
        }
    }) && fetched
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
            if !valid_id(name) || name == SETUP_RERUN || !valid_id(&policy.step) || !targets.insert(&policy.step)
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
/// Rerun the failed jobs once per CI step: when a published policy lists every failed job, or when every failed job
/// failed in setup (`setup_only`), which needs no policy. Either spends the step's one rerun.
pub(crate) fn rerun(item: &mut Item, id: &str, input: &Input, run: Option<&CiRun>, effects: &mut Vec<Effect>) -> Result<bool, Refusal> {
    let stage = &item.definition.stages[item.stage];
    if stage.kind != StageKind::AwaitCi { return Ok(false); }
    let Some(run) = run.filter(|r| r.valid() && !r.failed_jobs.is_empty()) else { return Ok(false) };
    let listed = item.definition.policies.iter().find(|(_, p)| p.step == stage.id)
        .filter(|(_, p)| run.failed_jobs.iter().all(|j| p.rule.rerun_failed.jobs.contains(j)))
        .map(|(name, p)| (name.clone(), p.rule.rerun_failed.times));
    let Some((name, times)) = listed.or_else(|| setup_only(run).then(|| (SETUP_RERUN.to_string(), 1))) else { return Ok(false) };
    let count = item.ci_reruns.get(&stage.id).copied().unwrap_or(0);
    if count >= times { return Ok(false); }
    item.ci_reruns.insert(stage.id.clone(), count + 1);
    item.ci_retry = Some(CiRetry { run: run.clone(), pending: true, policy: name,
        revision: input.expected_revision, input_digest: digest(&input.command) });
    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
    item.phase = Phase::Running;
    crate::reducer::emit(item, id, input, EffectKind::Execute(StageKind::RerunCi), effects);
    Ok(true)
}
