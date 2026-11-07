//! Finite workflow semantics shared by every renderer. No I/O or execution authority.
//! Hosts must supply capabilities from an authenticated session, never from page/user JSON.
use farcooler_workflow::{digest, Command, Evidence, Phase, Progression, Reviewer, StageKind, State};
use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 2;

mod display;

/// This is an explicit trusted input, not a credential or a transport grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub workspace: String,
    pub epoch: u64,
    pub inspect: bool,
    pub controller: bool,
    pub decision_principal: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reason {
    Automatic, ApprovalRequired, ReviewRequired, DependenciesOrResource,
    EffectUncertain, Reworking, EvidenceInvalidated, Completed, Failed, Cancelling, Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionKind { InspectLatestEvidence, InspectUncertainty, Approve, AcceptReview, RequestRework, RejectReview, Cancel }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub kind: ActionKind,
    pub label: String,
    /// Binds the entire snapshot and capability context; not a secret or an authorization token.
    pub guard: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageCard {
    pub id: String,
    pub label: String,
    pub progression_label: String,
    pub kind: StageKind,
    pub progression: Progression,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateKind { Approval, Review }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateCard {
    pub kind: GateKind,
    pub principal: String,
    pub label: String,
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasonCard { pub kind: Reason, pub label: String, pub text: String }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionForm {
    pub action: ActionKind,
    pub label: String,
    pub help: String,
    pub submit_label: String,
    pub max_bytes: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputError { pub code: String, pub text: String }
/// Validation is shared with every port; renderers forward text without policy.
pub fn validate_input(kind: &ActionKind, reason: Option<&str>) -> Result<(), InputError> {
    let text = if *kind == ActionKind::RequestRework {
        if reason.is_some_and(farcooler_workflow::valid_rework_reason) { return Ok(()); }
        "Enter a nonblank rework reason of at most 4096 UTF-8 bytes, without control characters except tabs and line breaks."
    } else {
        if reason.is_none() { return Ok(()); }
        "This action does not accept a rework reason."
    };
    Err(InputError { code: "workflow_reason".into(), text: text.into() })
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCard {
    pub label: String,
    pub phase_label: String,
    pub item: String,
    pub run: String,
    pub definition_pin: String,
    pub policy_pin: String,
    pub phase: Phase,
    pub stages: Vec<StageCard>,
    /// None after the last stage. The definition's approval policies remain visible.
    pub stage: Option<StageCard>,
    pub gate: Option<GateCard>,
    pub candidate: Option<String>,
    pub latest_evidence: Option<Evidence>,
    pub pending_effect: Option<String>,
    pub iteration: u8,
    pub reworks_remaining: u8,
    pub attempt: u8,
    pub fence: u64,
    pub reasons: Vec<ReasonCard>,
    pub actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forms: Vec<ActionForm>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rework_history: Vec<farcooler_workflow::ReworkDecision>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presentation {
    pub schema: u32,
    pub workspace: String,
    pub epoch: u64,
    pub revision: u64,
    pub runs: Vec<RunCard>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Error { Invalid, Unauthorized, Missing, Stale, Unavailable, ReasonRequired }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Intent {
    InspectLatestEvidence { evidence: Evidence },
    InspectUncertainty { effect: String, run: String, fence: u64 },
    /// Transport must reauthenticate, supply Input identity/revision/epoch, and run the reducer.
    Command(Command),
}

pub fn project(state: &State, auth: &Capabilities) -> Result<Presentation, Error> {
    if state.schema != 1 || state.epoch == 0 || state.items.len() > 1024 {
        return Err(Error::Invalid);
    }
    if auth.workspace != state.workspace || auth.epoch != state.epoch || !auth.inspect {
        return Err(Error::Unauthorized);
    }
    let snapshot_guard = digest(&("workflow-presentation/v2", state, auth));
    let mut runs = Vec::new();
    for (id, item) in &state.items {
        let guard = digest(&(&snapshot_guard, id));
        if item.definition.validate().is_err() || item.pin != item.definition.pin()
            || item.iteration > item.definition.max_reworks
            || (if item.phase == Phase::Done { item.stage != item.definition.stages.len() }
                else { item.stage >= item.definition.stages.len() })
        { return Err(Error::Invalid); }
        // A tracker-only ticket is a board card, not a run: it has no gate to approve and nothing to
        // stop, and listing every open card as an awaiting-approval run would bury the real ones (ov-962).
        if item.definition.is_ticket() { continue; }
        let stage = item.definition.stages.get(item.stage);
        let gate = match (&item.phase, stage) {
            (Phase::WaitingApproval, Some(s)) => match &s.progression {
                Progression::AwaitApproval { principal } => Some(display::gate_card(GateKind::Approval, principal)),
                _ => return Err(Error::Invalid),
            },
            (Phase::WaitingReview, Some(s)) => match &s.kind {
                StageKind::Review(Reviewer::Person(p) | Reviewer::Orchestrator(p)) => Some(display::gate_card(GateKind::Review, p)),
                _ => return Err(Error::Invalid),
            },
            _ => None,
        };
        let mut reasons = vec![match item.phase {
            Phase::Blocked | Phase::WaitingJoin | Phase::Joined => Reason::DependenciesOrResource,
            Phase::WaitingApproval => Reason::ApprovalRequired,
            Phase::WaitingReview => Reason::ReviewRequired,
            Phase::Running => Reason::Automatic,
            Phase::Uncertain => Reason::EffectUncertain,
            Phase::Cancelling => Reason::Cancelling,
            Phase::Done => Reason::Completed,
            Phase::Failed => Reason::Failed,
            Phase::Cancelled => Reason::Cancelled,
        }];
        let evidence_valid = item.last_evidence.as_ref().is_some_and(|e|
            item.candidate.as_ref() == Some(&e.candidate) && item.policy_pin == e.policy_pin);
        if item.iteration > 0 { reasons.push(Reason::Reworking); }
        if item.last_evidence.is_some() && !evidence_valid { reasons.push(Reason::EvidenceInvalidated); }
        let mut kinds = Vec::new();
        // Inspection returns the retained evidence identity even when it no longer authorizes action.
        if item.last_evidence.is_some() { kinds.push(ActionKind::InspectLatestEvidence); }
        if item.phase == Phase::Uncertain && item.pending.is_some() { kinds.push(ActionKind::InspectUncertainty); }
        if item.phase == Phase::WaitingApproval && stage.is_some_and(|s|
            matches!(&s.progression, Progression::AwaitApproval { principal }
                if auth.decision_principal.as_ref() == Some(principal)))
        { kinds.push(ActionKind::Approve); }
        if item.phase == Phase::WaitingReview && evidence_valid && stage.is_some_and(|s|
            matches!(&s.kind, StageKind::Review(Reviewer::Person(p) | Reviewer::Orchestrator(p))
                if auth.decision_principal.as_ref() == Some(p)))
        {
            kinds.push(ActionKind::AcceptReview);
            if item.iteration < item.definition.max_reworks { kinds.push(ActionKind::RequestRework); }
            else { kinds.push(ActionKind::RejectReview); }
        }
        if auth.controller && !matches!(item.phase, Phase::Done | Phase::Cancelled | Phase::Cancelling)
            && !item.run.starts_with("cancel:")
        { kinds.push(ActionKind::Cancel); }
        runs.push(RunCard { label: "Workflow Run".into(), phase_label: display::phase_label(&item.phase).into(),
            item: id.clone(), run: item.run.clone(), definition_pin: item.pin.clone(),
            policy_pin: item.policy_pin.clone(), phase: item.phase.clone(),
            stages: item.definition.stages.iter().map(display::stage_card).collect(),
            stage: stage.map(display::stage_card), gate, candidate: item.candidate.clone(),
            latest_evidence: item.last_evidence.clone(), pending_effect: item.pending.clone(),
            iteration: item.iteration, reworks_remaining: item.definition.max_reworks - item.iteration,
            attempt: item.attempt, fence: item.fence,
            forms: if kinds.contains(&ActionKind::RequestRework) { vec![ActionForm {
                action: ActionKind::RequestRework, label: "Rework Reason".into(),
                help: "Explain what needs to change. This text is retained with your decision.".into(),
                submit_label: "Request Rework".into(), max_bytes: farcooler_workflow::MAX_REWORK_REASON_BYTES,
            }] } else { Vec::new() },
            rework_history: item.rework_history.clone(),
            reasons: reasons.into_iter().map(display::reason_card).collect(),
            actions: kinds.into_iter().map(|kind| Action { label: display::action_label(&kind).into(), kind, guard: guard.clone() }).collect() });
    }
    Ok(Presentation { schema: SCHEMA, workspace: state.workspace.clone(), epoch: state.epoch,
        revision: state.revision, runs })
}

/// Reproject against the latest snapshot AND current authenticated context on every click.
/// The resulting command is only an intent; it cannot bypass reducer/store authorization.
pub fn action(state: &State, auth: &Capabilities, item: &str, requested: &Action) -> Result<Intent, Error> {
    action_with_reason(state, auth, item, requested, None)
}
pub fn action_with_reason(state: &State, auth: &Capabilities, item: &str, requested: &Action,
    reason: Option<&str>) -> Result<Intent, Error>
{
    let view = project(state, auth)?;
    let card = view.runs.iter().find(|c| c.item == item).ok_or(Error::Missing)?;
    if requested.guard != digest(&(digest(&("workflow-presentation/v2", state, auth)), item)) { return Err(Error::Stale); }
    if !card.actions.contains(requested) { return Err(Error::Unavailable); }
    validate_input(&requested.kind, reason).map_err(|_| Error::ReasonRequired)?;
    Ok(match requested.kind {
        ActionKind::InspectLatestEvidence => Intent::InspectLatestEvidence {
            evidence: card.latest_evidence.clone().ok_or(Error::Unavailable)? },
        ActionKind::InspectUncertainty => Intent::InspectUncertainty {
            effect: card.pending_effect.clone().ok_or(Error::Unavailable)?, run: card.run.clone(), fence: card.fence },
        ActionKind::Approve => Intent::Command(Command::Approve { item: item.into(), run: card.run.clone(),
            pin: card.definition_pin.clone(), candidate: card.candidate.clone() }),
        ActionKind::AcceptReview | ActionKind::RequestRework | ActionKind::RejectReview => Intent::Command(Command::Review {
            item: item.into(), run: card.run.clone(), evidence: card.latest_evidence.clone().ok_or(Error::Unavailable)?,
            passed: requested.kind == ActionKind::AcceptReview, reason: reason.map(str::to_owned) }),
        ActionKind::Cancel => Intent::Command(Command::Cancel { item: item.into() }),
    })
}

/// A local bridge DTO. Context is supplied by the host's authenticated adapter, never trusted on receipt.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: u32,
    pub snapshot: State,
    pub capabilities: Capabilities,
    pub selection: Option<Selection>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection { pub item: String, pub action: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub reason: Option<String> }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub schema: u32,
    pub card: Option<Presentation>,
    pub intent: Option<Intent>,
    pub error: Option<Error>,
}

/// All three bridges use these exact bytes, including stable error codes, without platform reducers.
pub fn json(request: &str) -> String {
    let result = (|| {
        if request.len() > 4 * 1024 * 1024 { return Err(Error::Invalid); }
        let request: Request = serde_json::from_str(request).map_err(|_| Error::Invalid)?;
        if request.schema != SCHEMA { return Err(Error::Invalid); }
        let card = project(&request.snapshot, &request.capabilities)?;
        let intent = request.selection.as_ref().map(|s|
            action_with_reason(&request.snapshot, &request.capabilities, &s.item, &s.action, s.reason.as_deref())).transpose()?;
        Ok((card, intent))
    })();
    let response = match result {
        Ok((card, intent)) => Response { schema: SCHEMA, card: Some(card), intent, error: None },
        Err(error) => Response { schema: SCHEMA, card: None, intent: None, error: Some(error) },
    };
    serde_json::to_string(&response).expect("closed workflow presentation types serialize")
}

#[cfg(test)]
mod tests;
