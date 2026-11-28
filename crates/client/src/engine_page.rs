//! The Engine view with a person's actions (ov-1486): what the web draws, and the only commands it can send.
//!
//! Built on the runner's one-reply `workflow.get` (ov-1434), which fits one control frame whatever the board
//! holds. When a board is too large even for that, the runner drops tracker cards first and says how many.
//!
//! **Nothing here decides who may act.** The runner does, for every command (`engine_caller` and
//! `Admission::permits`), and it decides who wrote an answer on a draft's card (`task_ops`). What this offers
//! only keeps a button off a row where the runner would refuse it for the run's own state.
//!
//! A renderer forwards a [`Selection`] and shows an [`Outcome`]. The command is made here, from a read made
//! at the time of the press, and only if the row is still the one the person saw (`guard`).
use std::collections::BTreeMap;

use farcooler_protocol::v1::{self as pb, Plan, TaskNoteKind};
use farcooler_protocol::workflow_commands::{Action, Mutation};
use farcooler_workflow::{Item, Phase, StageKind, State, digest};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::engine_view::{self, EngineView};
use crate::session::SessionError;
use crate::session::workflow::WorkflowContext;
use crate::workflow_presentation::{Capabilities, Style, cancellable};

const STYLE: Style = Style::Web;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Cancel a run, or close a failed one.
    Cancel,
    /// Have the runner look at an uncertain build or worktree step, and end it if nothing is there.
    Resolve,
    /// The owner's word that a cancellation whose receipt was lost is over.
    ResolveCancellation,
    /// The owner's word that a push whose outcome was lost landed.
    SettleLanded,
    /// The owner's word that it didn't.
    SettleNotLanded,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub kind: Kind,
    pub label: String,
    /// What pressing it does, shown before the person confirms.
    pub confirm: String,
    pub destructive: bool,
    /// Set when the action records the person's own words.
    pub reason_label: Option<String>,
    /// Opaque. Names the row as it stood when it was shown.
    pub guard: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub item: String,
    pub kind: Kind,
    pub guard: String,
    #[serde(default)]
    pub reason: Option<String>,
}

/// A workflow change waiting for a person (ov-971).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub id: String,
    /// The runner's whole question, whose first line is its headline. An approval carries a code that is only
    /// in it, so a renderer shows all of it, never a cut or a summary.
    pub question: String,
    /// The note that asked, which a decision names. Empty when the card has no question to answer.
    pub note: String,
    /// Why there is nothing to press, when there isn't.
    pub problem: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub view: EngineView,
    /// By item: a ticket's row, or a landing group's.
    pub actions: BTreeMap<String, Vec<Offer>>,
    pub drafts: Vec<Draft>,
    pub drafts_problem: Option<String>,
    /// What this read couldn't hold.
    pub notice: Option<String>,
    pub words: BTreeMap<&'static str, String>,
}

/// What came of a press, in sentences.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Outcome {
    pub ok: bool,
    pub message: String,
    /// The runner's own words, shown apart from ours.
    pub detail: Option<String>,
}

impl Outcome {
    pub(crate) fn done(message: &str) -> Self { Self { ok: true, message: message.into(), detail: None } }
    pub(crate) fn no(message: &str, detail: Option<String>) -> Self { Self { ok: false, message: message.into(), detail } }
}

pub(crate) const CHANGED: &str = "This changed since you looked, so nothing was done. Check it and try again.";
pub(crate) const DRAFT_GONE: &str = "This draft is no longer waiting for an answer, so nothing was sent.";

fn invalid(message: &str) -> SessionError { SessionError::Protocol(message.into()) }

fn decode(workspace: Uuid, raw: &serde_json::Value) -> Result<(State, Capabilities), SessionError> {
    let context: WorkflowContext = serde_json::from_value(raw["context"].clone()).map_err(|_| invalid("Invalid workflow context"))?;
    let auth = context.capabilities;
    if !auth.inspect || auth.workspace != workspace.to_string() { return Err(invalid("Engine inspection is unavailable")); }
    let state: State = if raw["state"].is_null() { State::empty(workspace.to_string(), auth.epoch) }
        else { serde_json::from_value(raw["state"].clone()).map_err(|_| invalid("Invalid workflow state"))? };
    if state.workspace != auth.workspace || state.epoch != auth.epoch { return Err(invalid("wrong workflow workspace")); }
    Ok((state, auth))
}

/// What the row's own state leaves open. The runner refuses each of these again for itself, and for the caller.
fn offered(item: &Item, auth: &Capabilities) -> Vec<Kind> {
    if item.definition.is_ticket() || item.follows.is_some() { return Vec::new(); }
    let owner = auth.decision_principal.as_deref() == Some("owner");
    let step = item.definition.stages.get(item.stage).map(|s| &s.kind);
    let observed = item.execution.as_ref().map(|e| &e.phase);
    let uncertain = item.phase == Phase::Uncertain;
    let mut kinds = Vec::new();
    if auth.controller && item.pending.is_some() && item.members.is_empty()
        && matches!(step, Some(StageKind::Provision | StageKind::Build))
        // A build can be resolved while the engine still has it running, when what it observes is uncertain.
        && (uncertain || item.phase == Phase::Running && matches!(step, Some(StageKind::Build)) && observed == Some(&Phase::Uncertain))
    { kinds.push(Kind::Resolve); }
    if owner && uncertain && item.stopping.is_some() && item.pending.is_some() && item.run.starts_with("cancel:")
        && item.members.is_empty()
        && matches!(step, Some(StageKind::Provision | StageKind::Build | StageKind::Check | StageKind::Run | StageKind::Review(_)))
    { kinds.push(Kind::ResolveCancellation); }
    if owner && uncertain && item.pending.is_some() && matches!(step, Some(StageKind::Push)) {
        kinds.extend([Kind::SettleLanded, Kind::SettleNotLanded]);
    }
    if auth.controller && cancellable(item) { kinds.push(Kind::Cancel); }
    kinds
}

/// A busy board moves its revision constantly, so a press is held to its own row, not to the whole board.
fn guard(state: &State, id: &str, item: &Item, auth: &Capabilities) -> String {
    digest(&("engine-page/v1", &auth.workspace, state.epoch, id, &item.run, &item.pin, &item.phase, item.stage,
        &item.pending, &auth.decision_principal))
}

fn offer(kind: Kind, item: &Item, guard: String) -> Offer {
    let landing = item.group_of.is_some();
    let (label, confirm) = match kind {
        Kind::Cancel if item.phase == Phase::Failed => (if landing { "close landing" } else { "close run" },
            "Closes the failed run. Its history stays on the ticket."),
        Kind::Cancel if landing => ("cancel landing", "Stops this landing. Its tickets go back to wait for another one."),
        Kind::Cancel => ("cancel run", "Stops the run and its agent. This can’t be undone."),
        Kind::Resolve => ("resolve step",
            "The runner looks for anything this step left behind. It ends the step only if the step is safely over, and otherwise says what is in the way."),
        Kind::ResolveCancellation => ("resolve cancellation",
            "Ends a cancellation whose receipt was lost. First check that the step’s processes have ended and what it changed outside the runner."),
        Kind::SettleLanded => ("settle as landed",
            "The runner reads the target branch first, and refuses if the commit isn’t there. It pushes nothing."),
        Kind::SettleNotLanded => ("settle as not landed",
            "The runner reads the target branch first, and refuses if the commit is there. The landing then starts over."),
    };
    Offer { kind, label: STYLE.label(label), confirm: confirm.into(),
        destructive: matches!(kind, Kind::Cancel | Kind::ResolveCancellation),
        reason_label: (kind == Kind::ResolveCancellation).then(|| STYLE.label("what you checked")), guard }
}

/// The view's own words. A renderer has them before the first read and while it is offline.
pub fn words() -> BTreeMap<&'static str, String> {
    let mut words = engine_view::interface_words(crate::plan_view::Style::Apple);
    words.extend([
        ("drafts", STYLE.label("drafts waiting for approval")),
        ("close", STYLE.label("close")),
        ("offline", "Reconnect to see the Engine view.".into()),
        ("earlierLandings", STYLE.label("earlier landings")),
        ("noDrafts", "No workflow changes are waiting for approval.".into()),
        ("approve", STYLE.label("approve")),
        ("decline", STYLE.label("decline")),
        ("confirmApprove", "Publishes exactly the change shown above.".into()),
        ("confirmDecline", "Leaves the workflow as it is.".into()),
        ("keep", STYLE.label("don’t do it")),
        ("refresh", STYLE.label("refresh")),
        ("runs", STYLE.label("workflow runs")),
        ("unconfirmed", "The action wasn’t confirmed. Check the refreshed view before trying again.".into()),
        ("working", "Sending…".into()),
    ]);
    words
}

/// Whether some row of the view has no title in `plan`. The plan carries cards only for the tickets its
/// themes, queue and agents name, so a ticket outside all three needs the board's own list.
pub fn lacks_titles(raw: &serde_json::Value, plan: &Plan) -> bool {
    let named: std::collections::BTreeSet<&str> = plan.cards.iter().map(|c| c.key.as_str()).collect();
    raw["state"]["items"].as_object().is_some_and(|items| items.iter()
        .any(|(id, item)| item["group_of"].is_null() && !named.contains(id.as_str())))
}

/// `plan` with a card for each of `tasks` it doesn't already have, for the view's titles.
pub fn with_titles(mut plan: Plan, tasks: &[pb::Task]) -> Plan {
    let named: std::collections::BTreeSet<String> = plan.cards.iter().map(|c| c.key.clone()).collect();
    plan.cards.extend(tasks.iter().filter(|t| !named.contains(&t.key)).map(|t| pb::PlanCard {
        task_id: t.id.clone(), key: t.key.clone(), title: t.title.clone(), status: t.status }));
    plan
}

pub fn page(workspace: Uuid, raw: &serde_json::Value, plan: Option<&Plan>, drafts: Vec<Draft>, drafts_problem: Option<String>)
    -> Result<Page, SessionError>
{
    let view = engine_view::from_workflow_reply(workspace, raw, plan, STYLE)?;
    let (state, auth) = decode(workspace, raw)?;
    let actions = state.items.iter().filter(|(_, item)| !matches!(item.phase, Phase::Done | Phase::Cancelled))
        .filter_map(|(id, item)| {
            let guard = guard(&state, id, item, &auth);
            let offers: Vec<_> = offered(item, &auth).into_iter().map(|kind| offer(kind, item, guard.clone())).collect();
            (!offers.is_empty()).then(|| (id.clone(), offers))
        }).collect();
    let omitted = raw["items_omitted"].as_u64().unwrap_or_default();
    let notice = (omitted > 0).then(|| format!(
        "This board is too large for one read, so {omitted} {} shown here. Cards with no run are left out first. Every ticket is on the Board.",
        if omitted == 1 { "ticket isn’t" } else { "tickets aren’t" }));
    Ok(Page { view, actions, drafts, drafts_problem, notice, words: words() })
}

/// The command a press means, from a read made now. `None` when the row is no longer what the person saw.
pub fn command(workspace: Uuid, raw: &serde_json::Value, selection: &Selection) -> Result<Option<Action>, SessionError> {
    let (state, auth) = decode(workspace, raw)?;
    let Some(item) = state.items.get(&selection.item) else { return Ok(None) };
    if selection.guard != guard(&state, &selection.item, item, &auth) || !offered(item, &auth).contains(&selection.kind) {
        return Ok(None);
    }
    let principal = auth.decision_principal.clone().ok_or_else(|| invalid("missing workflow caller source"))?;
    let sequence = state.source_sequences.get(&format!("wire:{principal}")).copied().unwrap_or(0)
        .checked_add(1).ok_or_else(|| invalid("workflow source sequence exhausted"))?;
    let mutation = Mutation { id: Uuid::now_v7().to_string(), revision: state.revision, epoch: state.epoch, sequence };
    let (item_id, run, pin) = (selection.item.clone(), item.run.clone(), item.pin.clone());
    Ok(Some(match selection.kind {
        Kind::Cancel => Action::Cancel { mutation, item: item_id, run, pin },
        Kind::Resolve => Action::Resolve { mutation, item: item_id, run, pin },
        Kind::ResolveCancellation => Action::ResolveCancellation { mutation, item: item_id, run, pin,
            reason: selection.reason.clone().unwrap_or_default() },
        Kind::SettleLanded => Action::Settle { mutation, item: item_id, run, pin, landed: true },
        Kind::SettleNotLanded => Action::Settle { mutation, item: item_id, run, pin, landed: false },
    }))
}

/// Why `selection` can't be sent as it is, before anything is read: a reason is the person's to give.
pub fn missing_reason(selection: &Selection) -> Option<Outcome> {
    (selection.kind == Kind::ResolveCancellation
        && !selection.reason.as_deref().is_some_and(farcooler_workflow::valid_rework_reason))
        .then(|| Outcome::no("Say what you checked, in at most 4096 bytes.", None))
}

/// The engine's receipt, as a person reads it. `Err` is the runner's word for why it did nothing.
pub fn receipt_outcome(kind: Kind, reply: &serde_json::Value) -> Outcome {
    let Some(result) = reply.pointer("/receipt/result") else {
        return Outcome::no("The runner’s answer couldn’t be read. Check the refreshed view.", None);
    };
    if result.get("Ok").is_some() {
        return Outcome::done(match kind {
            Kind::Cancel => "Cancellation accepted.",
            Kind::Resolve => "The step was resolved.",
            Kind::ResolveCancellation => "The cancellation was resolved.",
            Kind::SettleLanded | Kind::SettleNotLanded => "Settled. The engine carries on from here.",
        });
    }
    match result.get("Err").and_then(|e| e.as_str()) {
        Some("Stale") => Outcome::no(CHANGED, None),
        Some("Unauthorized") => Outcome::no("The runner didn’t allow this action. Nothing was done.", None),
        Some("Phase") => Outcome::no("The runner can’t do this at the run’s current step. Nothing was done.", None),
        other => Outcome::no("The runner didn’t do it.", Some(other.map_or_else(|| result["Err"].to_string(), str::to_owned))),
    }
}

/// A pending draft with the question its card asks, or why it has none to answer. The same test the runner
/// applies to an answer (`workflow_drafts::after_note`): the daemon's latest question, for this draft.
pub fn draft(id: &str, detail: &pb::TaskDetail) -> Draft {
    match asked(id, detail) {
        Some((note, question, _)) => Draft { id: id.into(), question, note, problem: None },
        None => Draft { id: id.into(), question: String::new(), note: String::new(),
            problem: Some("This draft’s card has no question to answer. Open it on the Board.".into()) },
    }
}

/// The card's question for draft `id`: its note, the whole text, and the exact answer that approves it.
fn asked(id: &str, detail: &pb::TaskDetail) -> Option<(String, String, String)> {
    let note = detail.notes.iter().rfind(|n| n.kind == TaskNoteKind::Question as i32)?;
    if note.actor != "manager" { return None; }
    let extra: serde_json::Value = serde_json::from_str(&note.extra_json).ok()?;
    if extra.pointer("/workflow_draft/id")?.as_str()? != id { return None; }
    let options: Vec<&str> = extra["options"].as_array()?.iter().filter_map(|o| o.as_str()).collect();
    let [approve, "decline"] = options.as_slice() else { return None };
    // The words that approve it must be in what the person is shown.
    if !approve.starts_with("approve ") || !note.body.contains(&format!("`{approve}`")) { return None; }
    Some((Uuid::from_slice(&note.id).ok()?.to_string(), note.body.clone(), (*approve).to_owned()))
}

/// The answer to send for a decision on the question the person saw, or `None` when the card has moved on.
pub fn decision(id: &str, note: &str, approve: bool, detail: &pb::TaskDetail) -> Option<String> {
    let (current, _, approving) = asked(id, detail)?;
    (current == note).then(|| if approve { approving } else { "decline".into() })
}

/// What an answer did, from the draft's state afterwards and the card's latest finding.
pub fn decision_outcome(approve: bool, state: &str, detail: &pb::TaskDetail) -> Outcome {
    let said = detail.notes.iter().rfind(|n| n.kind == TaskNoteKind::Finding as i32).map(|n| n.body.clone());
    match (state, approve) {
        ("approved", true) => Outcome::done("Approved and published."),
        ("declined", false) => Outcome::done("Declined. The workflow is unchanged."),
        ("publishing", true) => Outcome::done("Approved. The runner is publishing it."),
        ("pending", _) => Outcome::no("Your answer didn’t decide this draft.", said),
        _ => Outcome::no("Nothing was published.", said),
    }
}

#[cfg(test)]
mod tests;
