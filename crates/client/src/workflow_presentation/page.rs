//! What a run's page says, written once for every app (ov-1060): the ticket, how it ended and why, the steps as
//! plain words with their state, and the machine ids behind a "Details" disclosure, shortened, with the whole
//! value to copy. No renderer reads a stage id, a counter or a hash to make a sentence of it.
use super::*;

#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepRow { pub label: String, pub state: String }
/// One line under "Details". `whole` is the whole value when `value` is a shortened id.
#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailRow {
    pub title: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whole: Option<String>,
}
#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPage {
    /// The ticket the run is for.
    pub ticket: String,
    /// How the run stands or ended: "Cancelled", "Failed", "Running".
    pub outcome: String,
    /// Why, as a sentence.
    pub reason: String,
    pub steps: Vec<StepRow>,
    pub details_label: String,
    pub details: Vec<DetailRow>,
}

/// An id without its scheme ("wf1:") and cut to eight characters; a short value is left as it is.
pub(super) fn short(id: &str) -> String {
    let bare = id.rsplit(':').next().unwrap_or(id);
    if bare.chars().count() > 12 { bare.chars().take(8).collect() } else { bare.to_owned() }
}
fn id_row(title: &str, id: &str, style: Style) -> DetailRow {
    let value = short(id);
    DetailRow { title: style.label(title), whole: (value != id).then(|| id.to_owned()), value }
}
fn step_state(index: usize, current: usize, phase: &Phase) -> &'static str {
    if *phase == Phase::Done || index < current { return "Done"; }
    if index > current { return "Not started"; }
    match phase {
        Phase::Running => "Running", Phase::Cancelling => "Stopping",
        Phase::WaitingApproval => "Awaiting approval", Phase::WaitingReview => "Awaiting review",
        Phase::Blocked | Phase::WaitingJoin | Phase::Joined => "Waiting",
        Phase::Uncertain => "Outcome uncertain", Phase::Failed => "Failed", Phase::Cancelled => "Cancelled",
        Phase::Done => "Done",
    }
}
pub(super) fn run_page(id: &str, item: &farcooler_workflow::Item, reasons: &[ReasonCard], style: Style) -> RunPage {
    let reason = item.failure.as_deref().map(str::trim).filter(|f| !f.is_empty()).map(str::to_owned)
        .or_else(|| reasons.first().map(|r| r.text.clone())).unwrap_or_default();
    let steps = item.definition.stages.iter().enumerate().map(|(i, s)| StepRow {
        label: style.label(display::stage_label(&s.kind)),
        state: style.label(step_state(i, item.stage, &item.phase)) }).collect();
    let mut details = vec![id_row("Run", &item.run, style), id_row("Workflow", &item.pin, style)];
    if let Some(c) = &item.candidate { details.push(id_row("Candidate", c, style)); }
    if let Some(p) = &item.pending { details.push(id_row("Operation", p, style)); }
    details.push(DetailRow { title: style.label("Attempt"), value: item.attempt.to_string(), whole: None });
    if let Some(e) = &item.last_evidence {
        details.push(id_row("Evidence", &e.receipt, style));
    }
    RunPage { ticket: id.to_owned(), outcome: display::phase_label(&item.phase, style), reason, steps,
        details_label: style.label("Details"), details }
}
