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
pub struct PageGate { pub label: String, pub text: String }
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
    /// What a person must decide, when the run waits on a gate; "you" when the viewer is the one to decide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<PageGate>,
    /// The other reasons, as sentences: a rework, stale evidence, the reworks left, a runner's stop.
    pub more: Vec<String>,
    pub steps: Vec<StepRow>,
    pub details_label: String,
    pub details: Vec<DetailRow>,
}

/// An id without its scheme ("wf1:") and cut to eight characters; a short value is left as it is.
pub(super) fn short(id: &str) -> String {
    let bare = id.rsplit(':').next().unwrap_or(id);
    if bare.chars().count() > 12 && bare.chars().all(|c| c.is_ascii_hexdigit()) { bare.chars().take(8).collect() } else { bare.to_owned() }
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
pub(super) fn run_page(id: &str, item: &farcooler_workflow::Item, reasons: &[ReasonCard], gate: Option<&GateCard>,
    viewer: Option<&str>, style: Style) -> RunPage
{
    let stage = item.definition.stages.get(item.stage).map(|s| style.label(display::stage_label(&s.kind))).unwrap_or_default();
    let kept = item.failure.as_deref().map(str::trim).filter(|f| !f.is_empty()).map(str::to_owned);
    let reason = kept.clone().or_else(|| match item.phase {
        Phase::Running => Some(format!("Running: {stage}.")),
        Phase::Uncertain => Some(format!("{stage} may or may not have finished. Check it before it continues.")),
        _ => None,
    }).or_else(|| reasons.first().map(|r| r.text.clone())).unwrap_or_default();
    let mut more: Vec<String> = reasons.iter().skip(1).map(|r| r.text.clone()).collect();
    if item.iteration > 0 {
        let left = item.definition.max_reworks - item.iteration;
        more.push(format!("{left} rework{} left.", if left == 1 { "" } else { "s" }));
    }
    let gate = gate.map(|g| {
        let who = if viewer == Some(g.principal.as_str()) { "you".to_owned() } else { g.principal.clone() };
        let verb = if g.kind == GateKind::Approval { "approve" } else { "review" };
        PageGate { label: g.label.clone(), text: format!("Waiting for {who} to {verb}.") }
    });
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
    RunPage { ticket: id.to_owned(), outcome: display::phase_label(&item.phase, style), reason, gate, more, steps,
        details_label: style.label("Details"), details }
}
