//! Finite workflow copy belongs beside its semantics so feature additions need no shell copy maps.
use super::*;

pub(super) fn phase_label(phase: &Phase) -> &'static str {
    match phase {
        Phase::Blocked => "Waiting", Phase::WaitingApproval => "Awaiting Approval",
        Phase::WaitingReview => "Awaiting Review", Phase::Running => "Running",
        Phase::Uncertain => "Outcome Uncertain", Phase::Cancelling => "Cancelling",
        Phase::Done => "Completed", Phase::Failed => "Failed", Phase::Cancelled => "Cancelled",
        Phase::WaitingJoin => "Waiting to Land", Phase::Joined => "Queued in a Batch",
    }
}
pub(super) fn stage_card(stage: &farcooler_workflow::Stage) -> StageCard {
    let label = match stage.kind {
        StageKind::Provision => "Prepare Workspace", StageKind::Build => "Build",
        StageKind::Check => "Run Checks", StageKind::Review(_) => "Review",
        StageKind::Land => "Land Changes", StageKind::Ci => "Check CI",
        StageKind::Tracker => "Update Tracker", StageKind::Action(_) => "Run Action",
        StageKind::Evaluate(_) => "Evaluate Result", StageKind::Join { .. } => "Join Batch",
        StageKind::AwaitBase => "Wait for Base", StageKind::Fold => "Combine Changes",
        StageKind::Run => "Run Gate", StageKind::Push => "Push Changes", StageKind::AwaitCi => "Wait for CI", StageKind::Route => "Choose Route",
    };
    let progression_label = match &stage.progression {
        Progression::Automatic => "Automatic".into(),
        Progression::AwaitApproval { principal } => format!("Approval from {principal}"),
    };
    StageCard { id: stage.id.clone(), kind: stage.kind.clone(), progression: stage.progression.clone(),
        label: label.into(), progression_label }
}
pub(super) fn gate_card(kind: GateKind, principal: &str) -> GateCard {
    let (label, text) = match kind {
        GateKind::Approval => ("Approval Required", format!("Waiting for approval from {principal}.")),
        GateKind::Review => ("Review Required", format!("Waiting for a review decision from {principal}.")),
    };
    GateCard { kind, principal: principal.into(), label: label.into(), text }
}
pub(super) fn reason_card(kind: Reason) -> ReasonCard {
    let (label, text) = match kind {
        Reason::Automatic => ("Running", "The engine is executing this stage."),
        Reason::ApprovalRequired => ("Approval Required", "This stage is waiting for its configured approval."),
        Reason::ReviewRequired => ("Review Required", "This stage is waiting for its configured review decision."),
        Reason::DependenciesOrResource => ("Waiting", "This run is waiting for prerequisites or an available resource."),
        Reason::EffectUncertain => ("Outcome Uncertain", "The operation outcome needs inspection before execution can continue."),
        Reason::Reworking => ("Rework Iteration", "This run has entered a rework iteration."),
        Reason::EvidenceInvalidated => ("Evidence Invalidated", "The latest evidence no longer matches the current candidate or policy."),
        Reason::Completed => ("Completed", "All stages have completed."),
        Reason::Failed => ("Failed", "This run has failed."),
        Reason::Cancelling => ("Cancelling", "The engine is waiting for the operation to stop."),
        Reason::Cancelled => ("Cancelled", "This run has been cancelled."),
    };
    ReasonCard { kind, label: label.into(), text: text.into() }
}
pub(super) fn action_label(kind: &ActionKind) -> &'static str {
    match kind {
        ActionKind::InspectLatestEvidence => "Inspect Latest Evidence…",
        ActionKind::InspectUncertainty => "Inspect Uncertain Operation…",
        ActionKind::Approve => "Approve", ActionKind::AcceptReview => "Accept Review",
        ActionKind::RequestRework => "Request Rework", ActionKind::RejectReview => "Reject Review",
        ActionKind::Cancel => "Cancel Run",
    }
}
