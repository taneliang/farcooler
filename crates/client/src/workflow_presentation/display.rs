//! Finite workflow copy belongs beside its semantics so feature additions need no shell copy maps.
use super::*;

pub(super) fn phase_label(phase: &Phase, style: Style) -> String {
    style.label(match phase {
        Phase::Blocked => "Waiting", Phase::WaitingApproval => "Awaiting approval",
        Phase::WaitingReview => "Awaiting review", Phase::Running => "Running",
        Phase::Uncertain => "Outcome uncertain", Phase::Cancelling => "Cancelling",
        Phase::Done => "Completed", Phase::Failed => "Failed", Phase::Cancelled => "Cancelled",
        Phase::WaitingJoin => "Waiting to land", Phase::Joined => "Queued in a batch",
    })
}
pub(super) fn stage_label(kind: &StageKind) -> &'static str {
    match kind {
        StageKind::Provision => "Prepare workspace", StageKind::Build => "Build",
        StageKind::Check => "Run checks", StageKind::Review(_) => "Review",
        StageKind::Land => "Land changes", StageKind::Ci => "Check CI",
        StageKind::Tracker => "Update tracker", StageKind::Action(_) => "Run action",
        StageKind::Evaluate(_) => "Evaluate result", StageKind::Join { .. } => "Join batch",
        StageKind::AwaitBase => "Wait for base", StageKind::Fold => "Combine changes",
        StageKind::Run => "Run gate", StageKind::Push => "Push changes", StageKind::AwaitCi => "Wait for CI", StageKind::RerunCi => "Rerun failed CI jobs",
        StageKind::Route => "Choose route",
        StageKind::PushBranch => "Push branch", StageKind::OpenPr => "Open pull request",
        StageKind::PrReady => "Mark ready", StageKind::AwaitPr => "Wait for merge",
    }
}
pub(super) fn stage_card(stage: &farcooler_workflow::Stage, style: Style) -> StageCard {
    let progression_label = match &stage.progression {
        Progression::Automatic => style.label("Automatic"),
        // A phrase around a name reads the same in every style: the name's case is its own.
        Progression::AwaitApproval { principal } => format!("Approval from {principal}"),
    };
    StageCard { id: stage.id.clone(), kind: stage.kind.clone(), progression: stage.progression.clone(),
        label: style.label(stage_label(&stage.kind)), progression_label }
}
pub(super) fn gate_card(kind: GateKind, principal: &str, style: Style) -> GateCard {
    let (label, text) = match kind {
        GateKind::Approval => ("Approval required", format!("Waiting for approval from {principal}.")),
        GateKind::Review => ("Review required", format!("Waiting for a review decision from {principal}.")),
    };
    GateCard { kind, principal: principal.into(), label: style.label(label), text }
}
pub(super) fn reason_card(kind: Reason, style: Style) -> ReasonCard {
    let (label, text) = match kind {
        Reason::Automatic => ("Running", "The engine is executing this stage."),
        Reason::ApprovalRequired => ("Approval required", "This stage is waiting for its configured approval."),
        Reason::ReviewRequired => ("Review required", "This stage is waiting for its configured review decision."),
        Reason::DependenciesOrResource => ("Waiting", "This run is waiting for prerequisites or an available resource."),
        Reason::EffectUncertain => ("Outcome uncertain", "The operation outcome needs inspection before execution can continue."),
        Reason::Reworking => ("Rework iteration", "This run has entered a rework iteration."),
        Reason::EvidenceInvalidated => ("Evidence invalidated", "The latest evidence no longer matches the current candidate or policy."),
        Reason::Completed => ("Completed", "All stages have completed."),
        Reason::Failed => ("Failed", "This run has failed."),
        Reason::Cancelling => ("Cancelling", "The engine is waiting for the operation to stop."),
        Reason::Cancelled => ("Cancelled", "This run has been cancelled."),
        Reason::WorktreeChanged => ("Worktree has changes", "The run stopped on changes in its worktree."),
    };
    ReasonCard { kind, label: style.label(label), text: text.into() }
}
/// A cancelled run says why, when the cancel gave a reason (ov-1053), so every app shows the same string. The reason
/// is a whole sentence with the cause first, so it stands in for "This run has been cancelled." and says it once; a
/// run still stopping keeps its own sentence after it. A blank reason says nothing.
pub(super) fn with_cancel_reason(mut card: ReasonCard, cancel_reason: Option<&str>) -> ReasonCard {
    if matches!(card.kind, Reason::Cancelled | Reason::Cancelling) {
        if let Some(why) = cancel_reason.map(str::trim).filter(|w| !w.is_empty()) {
            card.text = if card.kind == Reason::Cancelling { format!("{why} {}", card.text) } else { why.to_owned() };
        }
    }
    card
}
pub(super) fn action_label(kind: &ActionKind, style: Style) -> String {
    style.label(match kind {
        ActionKind::InspectLatestEvidence => "Inspect latest evidence…",
        ActionKind::InspectUncertainty => "Inspect uncertain operation…",
        ActionKind::Approve => "Approve", ActionKind::AcceptReview => "Accept review",
        ActionKind::RequestRework => "Request rework", ActionKind::RejectReview => "Reject review",
        ActionKind::Cancel => "Cancel run", ActionKind::Retry => "Try again",
    })
}
/// What cancelling says on a run that has already failed: the engine closes it, and nothing is running to cancel.
pub(super) fn close_label(style: Style) -> String { style.label("Close run") }
pub(super) fn run_label(style: Style) -> String { style.label("Workflow run") }
pub(super) fn rework_form(max_bytes: usize, style: Style) -> ActionForm {
    ActionForm {
        action: ActionKind::RequestRework, label: style.label("Rework reason"),
        help: "Explain what needs to change. This text is retained with your decision.".into(),
        submit_label: action_label(&ActionKind::RequestRework, style), max_bytes,
    }
}
