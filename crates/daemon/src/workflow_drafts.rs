//! The orchestrator drafts a workflow change; a person approves it (ov-971).
//!
//! A draft is a definition and the adapter settings it is pinned to, with the pin of the workflow it was
//! drafted against. `propose` files it, renders its summary from the definition (`workflow_summary`), and opens
//! a card in Needs Decision that asks a person.
//!
//! **A person approves only what they read.** The card has no quick-answer options, so no Needs You row and
//! no notification carries an Approve button: those show the headline, never the change. The card's full
//! question ends with the words that approve it, `approve <code>`, where the code is cut from the digest of
//! the definition, the adapter and the summary. Only a client that shows the whole question shows the code,
//! and an answer approves only the draft whose content it was cut from.
//!
//! **Nothing publishes without the owner.** The only way out of `Pending` into a published workflow is
//! `after_note`: the owner's answer to the draft's own, latest question, not said from a pane (an
//! orchestrator's answer is the manager's and decides nothing). Nobody else can ask on a draft's card or
//! rewrite it (`refuse_on_card`). It publishes through the ordinary `Publish` command, admitted as that
//! person, so every check `workspace set-workflow` makes is made here too.
//!
//! Drafts live in one JSON file each under the runtime root. A new table would change the schema the engine
//! cutover pins exactly, and a draft isn't engine state: it is a request waiting on a person.
use crate::service::Service;
use crate::workflow_summary;
use farcooler_core::{DomainError, Result};
use farcooler_protocol::workflow_commands::{Action, Mutation};
use farcooler_protocol::workflow_config::AdapterConfiguration;
use farcooler_protocol::v1 as pb;
use farcooler_store::models::{Actor, NoteKind, TaskStatus};
use farcooler_workflow::Definition;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use uuid::Uuid;

/// The words that answer a draft's card. Approve takes the draft's code after it.
pub const APPROVE: &str = "approve";
pub const DECLINE: &str = "decline";
/// The key a draft's question carries in its note's `extra`. Only the daemon writes it.
pub const EXTRA: &str = "workflow_draft";
const MAX_REASON: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Waiting on a person.
    Pending,
    Approved,
    Declined,
    /// The live workflow changed after this was drafted, so approving it would undo that change.
    OutOfDate,
    /// A newer draft took its place.
    Replaced,
    /// The owner approved it and it is being published now.
    Publishing,
    /// A person approved it and publishing was refused. The card says why.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub id: String,
    pub workspace: Uuid,
    /// The card a person answers on.
    pub task: Option<Uuid>,
    pub state: State,
    /// The requirement, in the orchestrator's words. Shown as theirs, never as a fact about the definition.
    pub reason: String,
    /// What the definition does, rendered from the definition.
    pub summary: String,
    pub definition: Definition,
    pub adapter: AdapterConfiguration,
    /// The pin published when this was drafted, or none if nothing was.
    pub expected_pin: Option<String>,
    pub created_at: i64,
    pub decided_at: Option<i64>,
    /// The digest of the definition, the adapter and the summary. The card's question carries it too, in the
    /// database, so a draft file edited after the person read it no longer matches.
    #[serde(default)]
    pub digest: String,
    /// The daemon's question on the card, the one an answer must follow.
    #[serde(default)]
    pub question: Option<Uuid>,
    /// The answer that decided it.
    #[serde(default)]
    pub answer: Option<Uuid>,
}

/// What a draft's content digests to.
fn content_digest(definition: &Definition, adapter: &AdapterConfiguration, summary: &str) -> String {
    farcooler_workflow::digest(&("workflow-draft/v1", definition, adapter, summary))
}
/// The code a person types to approve: six characters of the content digest.
pub fn code(digest: &str) -> String { digest.trim_start_matches("wf1:").chars().take(6).collect() }

static LOCK: Mutex<()> = Mutex::new(());
fn now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64) }
fn dir(svc: &Service) -> PathBuf { svc.root_dir().join("workflow-drafts") }
fn path(svc: &Service, id: &str) -> PathBuf { dir(svc).join(format!("{id}.json")) }
fn valid_id(id: &str) -> bool { !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_hexdigit()) }

fn write(svc: &Service, draft: &Draft) -> Result<()> {
    let dir = dir(svc);
    std::fs::create_dir_all(&dir).map_err(|_| DomainError::OperationFailed)?;
    let tmp = dir.join(format!("{}.tmp", draft.id));
    std::fs::write(&tmp, serde_json::to_vec_pretty(draft).map_err(|_| DomainError::OperationFailed)?).map_err(|_| DomainError::OperationFailed)?;
    std::fs::rename(&tmp, path(svc, &draft.id)).map_err(|_| DomainError::OperationFailed)
}
pub fn get(svc: &Service, id: &str) -> Result<Option<Draft>> {
    if !valid_id(id) { return Ok(None); }
    match std::fs::read(path(svc, id)) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
        Err(_) => Ok(None),
    }
}
/// Every draft, newest first, optionally one workspace's.
pub fn list(svc: &Service, workspace: Option<Uuid>) -> Vec<Draft> {
    let mut all: Vec<Draft> = std::fs::read_dir(dir(svc)).into_iter().flatten().flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read(e.path()).ok()).filter_map(|b| serde_json::from_slice::<Draft>(&b).ok())
        .filter(|d| workspace.is_none_or(|w| d.workspace == w)).collect();
    all.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
    all
}
fn for_task(svc: &Service, task: Uuid) -> Option<Draft> { list(svc, None).into_iter().find(|d| d.task == Some(task)) }

/// Move a draft from `from` to `to`, once, under the lock. False when it isn't in `from` any more.
fn settle(svc: &Service, id: &str, from: State, to: State, answer: Option<Uuid>) -> Result<bool> {
    let _held = LOCK.lock().map_err(|_| DomainError::OperationFailed)?;
    let Some(mut draft) = get(svc, id)? else { return Ok(false) };
    if draft.state != from { return Ok(false); }
    draft.state = to;
    draft.decided_at = Some(now());
    if answer.is_some() { draft.answer = answer; }
    write(svc, &draft)?;
    Ok(true)
}

/// A draft, as the orchestrator reads it.
pub fn wire(draft: &Draft) -> serde_json::Value {
    serde_json::json!({ "id": draft.id, "state": draft.state, "reason": draft.reason, "summary": draft.summary,
        "card": draft.task, "drafted_against": draft.expected_pin, "created_at": draft.created_at, "decided_at": draft.decided_at,
        "source": draft.definition.source, "answer": draft.answer })
}

/// What the card asks, in the person's terms. The summary is the daemon's; the reason is labeled the orchestrator's.
/// The words that approve it come last, after the whole change, so only a view that shows all of it shows them.
fn question(draft: &Draft) -> String {
    let bundle = crate::landing_templates::Bundle { definition: draft.definition.clone(), adapter: draft.adapter.clone() };
    format!("{}\n\n{}\n\nTo publish exactly this, answer `{APPROVE} {}`. To leave things as they are, answer `{DECLINE}`. Nothing changes until you answer.",
        workflow_summary::headline(&bundle), draft.summary, code(&draft.digest))
}

/// Ask the draft's question on its card, as the daemon, and remember which note asked it.
fn ask(svc: &Service, draft: &Draft, card: Uuid) -> Result<()> {
    let note = svc.store.add_note(card, NoteKind::Question, Actor::Manager, &question(draft),
        serde_json::json!({ EXTRA: { "id": draft.id, "digest": draft.digest } }))?;
    let _held = LOCK.lock().map_err(|_| DomainError::OperationFailed)?;
    let mut now = get(svc, &draft.id)?.ok_or(DomainError::NotFound)?;
    now.task = Some(card);
    now.question = Some(note.id);
    write(svc, &now)
}

/// Whether `card` is a draft's card: one the daemon asked a draft's question on.
pub fn is_draft_card(svc: &Service, card: Uuid) -> bool {
    svc.store.notes_for(card, Some(NoteKind::Question)).unwrap_or_default().iter().any(|n| n.actor == Actor::Manager && n.extra.get(EXTRA).is_some())
}

/// A draft's card says only what the daemon wrote: nobody else asks on it, decides on it, replaces one of its
/// notes, or rewrites its title or description. And no caller writes a draft's mark on any card.
pub fn refuse_on_card(svc: &Service, card: Uuid, kind: Option<NoteKind>, extra: &serde_json::Value, supersedes: Option<Uuid>) -> Result<()> {
    let refused = Err(DomainError::Conflict { what: EXTRA });
    if extra.get(EXTRA).is_some() { return refused; }
    let asks = matches!(kind, None | Some(NoteKind::Question | NoteKind::Decision)) || supersedes.is_some();
    if asks && is_draft_card(svc, card) { return refused; }
    Ok(())
}

/// File a proposal and ask a person. Returns the draft, already pending on its card.
pub fn propose(svc: &Service, watcher: &crate::watch::Watcher, workspace: Uuid, reason: &str, definition: Definition, adapter: AdapterConfiguration) -> Result<Draft> {
    let reason = reason.trim();
    if reason.is_empty() || reason.chars().count() > MAX_REASON { return Err(DomainError::InvalidArgument { what: "reason" }); }
    // A draft that couldn't publish is refused now, not when someone taps Approve.
    definition.validate().map_err(|_| DomainError::InvalidArgument { what: "definition" })?;
    let AdapterConfiguration::Coding(config) = &adapter;
    crate::workflow_runtime::CodingConfigExt::validate_definition(config, &definition).map_err(|_| DomainError::InvalidArgument { what: "definition" })?;
    let current = svc.store.workflow_configuration(&workspace.to_string()).map_err(|_| DomainError::OperationFailed)?;
    let bundle = crate::landing_templates::Bundle { definition, adapter };
    let summary = workflow_summary::summarize(reason, &bundle, current.as_ref().map(|c| &c.definition));
    let crate::landing_templates::Bundle { definition, adapter } = bundle;
    let digest = content_digest(&definition, &adapter, &summary);
    let draft = Draft { id: Uuid::now_v7().simple().to_string()[20..].to_string(), workspace, task: None, state: State::Pending,
        reason: reason.into(), summary, definition, adapter, expected_pin: current.map(|c| c.pin), created_at: now(), decided_at: None,
        digest, question: None, answer: None };
    // One question at a time: an earlier pending draft of this workspace is replaced, and its card closed.
    let mut replaced = Vec::new();
    {
        let _held = LOCK.lock().map_err(|_| DomainError::OperationFailed)?;
        for mut old in list(svc, Some(workspace)).into_iter().filter(|d| d.state == State::Pending) {
            old.state = State::Replaced;
            old.decided_at = Some(draft.created_at);
            write(svc, &old)?;
            replaced.extend(old.task);
        }
        write(svc, &draft)?;
    }
    for card in replaced { close_card(svc, watcher, card, TaskStatus::Cancelled, "A newer proposal replaced this one."); }
    let title = card_title(&draft.definition.source);
    let card = svc.store.create_task(workspace, &title, Actor::Manager)?;
    ask(svc, &draft, card.id)?;
    let card = svc.store.set_task_status(card.id, TaskStatus::NeedsDecision, Actor::Manager)?;
    watcher.announce_task_changed(&card, None, Actor::Manager);
    watcher.announce_needs_you();
    get(svc, &draft.id)?.ok_or(DomainError::NotFound)
}

/// Close the draft's card, through the engine when the board's workflow holds it.
fn close_card(svc: &Service, watcher: &crate::watch::Watcher, card: Uuid, status: TaskStatus, why: &str) {
    let _ = svc.store.add_note(card, NoteKind::Finding, Actor::Manager, why, serde_json::json!({}));
    let moved = svc.store.task_is_tracked_ticket(card).unwrap_or(false);
    if moved {
        if let (Some(handle), Ok(task)) = (svc.workflow_handle(), svc.store.get_task(card)) {
            let column = crate::ticket_moves::column(status);
            let (workspace, key) = (task.workspace_id.to_string(), task.key.clone());
            tokio::spawn(async move { let _ = handle.move_ticket(&workspace, &key, column).await; });
        }
    } else if let Ok(task) = svc.store.set_task_status(card, status, Actor::Manager) {
        watcher.announce_task_changed(&task, None, Actor::Manager);
    }
    watcher.announce_needs_you();
}

/// The wire's answer to `Propose` and `Drafts`.
pub(crate) fn handle(svc: &Service, watcher: &crate::watch::Watcher, workspace: Uuid, action: &Action) -> Result<farcooler_protocol::v1::result::Value> {
    let value = answer(svc, watcher, workspace, action)?;
    Ok(farcooler_protocol::v1::result::Value::WorkflowReply(pb::WorkflowReply { schema: 1, json: value.to_string() }))
}
fn answer(svc: &Service, watcher: &crate::watch::Watcher, workspace: Uuid, action: &Action) -> Result<serde_json::Value> {
    match action {
        Action::Propose { reason, definition, adapter } => {
            let draft = propose(svc, watcher, workspace, reason, definition.clone(), (**adapter).clone())?;
            Ok(serde_json::json!({ "schema": 1, "draft": wire(&draft) }))
        }
        Action::Drafts { draft: Some(id) } => match get(svc, id)?.filter(|d| d.workspace == workspace) {
            Some(d) => Ok(serde_json::json!({ "schema": 1, "draft": wire(&d) })),
            None => Err(DomainError::NotFound),
        },
        Action::Drafts { draft: None } => Ok(serde_json::json!({ "schema": 1, "drafts": list(svc, Some(workspace)).iter().map(wire).collect::<Vec<_>>() })),
        _ => Err(DomainError::NotFound),
    }
}

/// `task.note`'s reply, after a person's answer on a draft's card has had its say.
pub(crate) async fn note(rpc: &crate::rpc::Rpc, value: farcooler_protocol::v1::result::Value) -> Result<farcooler_protocol::v1::result::Value> {
    if let farcooler_protocol::v1::result::Value::TaskNote(note) = &value { after_note(rpc, note).await; }
    Ok(value)
}

/// The owner's word on a draft's card. Called after every `task.note`; does nothing unless the note is an
/// answer by the owner themself (not said from a pane) to a pending draft's card, after the daemon's own,
/// latest question, whose content still digests to what the card was asked with. `decline` declines it;
/// `approve <code>` with this draft's code approves it. Any other answer asks again, since an answer takes
/// the card out of Needs You.
pub(crate) async fn after_note(rpc: &crate::rpc::Rpc, note: &pb::TaskNote) {
    if note.kind != pb::TaskNoteKind::Answer as i32 || note.actor != "user" { return; }
    let svc = rpc.service();
    let (Ok(task), Ok(answer)) = (crate::task_ops::required_id(&note.task_id), crate::task_ops::required_id(&note.id)) else { return };
    let Some(draft) = for_task(svc, task).filter(|d| d.state == State::Pending) else { return };
    // The question it follows is the daemon's, and the draft is what that question was asked with.
    let questions = svc.store.notes_for(task, Some(NoteKind::Question)).unwrap_or_default();
    let Some(asked) = questions.last().filter(|q| Some(q.id) == draft.question && q.actor == Actor::Manager) else { return };
    let asked_with = asked.extra.pointer(&format!("/{EXTRA}/digest")).and_then(|d| d.as_str());
    if asked_with != Some(draft.digest.as_str()) || content_digest(&draft.definition, &draft.adapter, &draft.summary) != draft.digest {
        // The draft isn't what the card was asked with: something edited it after. It can never be approved.
        if settle(svc, &draft.id, State::Pending, State::Failed, Some(answer)).unwrap_or(false) {
            close_card(svc, rpc.watcher(), task, TaskStatus::Cancelled, "Not published. The draft changed after you were asked, so it isn’t what you read. Ask the orchestrator to draft it again.");
        }
        return;
    }
    // A member's answer is not the owner's: it decides nothing, and the owner is asked again.
    let owner = matches!(rpc.caller(), Ok(crate::people::Person::Owner));
    let words = note.body.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    if owner && words == DECLINE {
        if settle(svc, &draft.id, State::Pending, State::Declined, Some(answer)).unwrap_or(false) { close_card(svc, rpc.watcher(), task, TaskStatus::Cancelled, "Declined. The workflow is unchanged."); }
    } else if owner && words == format!("{APPROVE} {}", code(&draft.digest)) {
        approve(rpc, draft, task, answer).await;
    } else {
        let why = if owner { format!("Nothing was published. To publish this change, read it on the card and answer with the words at its end. To leave things as they are, answer `{DECLINE}`.") }
            else { "Nothing was published. Only the owner can approve or decline a workflow change.".to_string() };
        let _ = svc.store.add_note(task, NoteKind::Finding, Actor::Manager, &why, serde_json::json!({}));
        if ask(svc, &draft, task).is_ok() && let Ok(card) = svc.store.get_task(task) { rpc.watcher().announce_task_changed(&card, None, Actor::Manager); }
        rpc.watcher().announce_needs_you();
    }
}

async fn approve(rpc: &crate::rpc::Rpc, draft: Draft, card: Uuid, answer: Uuid) {
    let svc = rpc.service();
    let ws = draft.workspace.to_string();
    let live = svc.store.workflow_configuration(&ws).ok().flatten().map(|c| c.pin);
    if live != draft.expected_pin {
        if settle(svc, &draft.id, State::Pending, State::OutOfDate, Some(answer)).unwrap_or(false) {
            close_card(svc, rpc.watcher(), card, TaskStatus::Cancelled, "Not published. The workflow changed after this was drafted. Ask the orchestrator to draft it again.");
        }
        return;
    }
    // Claimed under the lock, so a proposal replacing it now can't, and a second answer can't publish it twice.
    if !settle(svc, &draft.id, State::Pending, State::Publishing, Some(answer)).unwrap_or(false) { return; }
    match publish(rpc, &draft).await {
        Ok(()) => { if settle(svc, &draft.id, State::Publishing, State::Approved, None).unwrap_or(false) { close_card(svc, rpc.watcher(), card, TaskStatus::Done, "Approved and published."); } }
        Err(why) => { if settle(svc, &draft.id, State::Publishing, State::Failed, None).unwrap_or(false) { close_card(svc, rpc.watcher(), card, TaskStatus::Cancelled, &format!("Not published. {why}")); } }
    }
}

/// Publish through the ordinary command, admitted as the person who answered. The mutation id is the draft's,
/// so a retry is the same receipt.
async fn publish(rpc: &crate::rpc::Rpc, draft: &Draft) -> std::result::Result<(), String> {
    fn refused<E>(_: E) -> String { "Far Cooler couldn’t publish it just now.".to_string() }
    let handle = rpc.service().workflow_handle().ok_or("The workflow engine isn’t running.".to_string())?;
    let admit = || rpc.admit(draft.workspace);
    let state = handle.wire_command(admit().await.map_err(refused)?, Action::Get).await.map_err(refused)?;
    let (revision, epoch) = (state.pointer("/state/revision").and_then(|v| v.as_u64()).unwrap_or(0), state.pointer("/state/epoch").and_then(|v| v.as_u64()).unwrap_or(1));
    let action = Action::Publish { mutation: Mutation { id: format!("draft-{}", draft.id), revision, epoch, sequence: 1 }, expected_pin: draft.expected_pin.clone(),
        definition: draft.definition.clone(), adapter: Some(Box::new(draft.adapter.clone())), notifications: None };
    let reply = handle.wire_command(admit().await.map_err(refused)?, action).await.map_err(refused)?;
    match reply.pointer("/receipt/result") {
        Some(r) if r.get("Ok").is_some() => Ok(()),
        Some(r) if r.get("Err").is_some_and(|e| e.as_str() == Some("Stale")) => Err("The workflow changed while you were deciding.".into()),
        _ => Err("The engine refused it.".into()),
    }
}

/// The card's title. The definition's name is the drafter's text, so it is shown escaped on one line.
fn card_title(source: &str) -> String {
    let name = source.rsplit('/').next().filter(|n| !n.is_empty()).map_or_else(|| "a change".to_string(), workflow_summary::code);
    format!("Workflow: approve {}", name.trim_start_matches('`').trim_end_matches('`'))
}

#[cfg(test)]
mod title_tests {
    #[test]
    fn the_card_title_is_one_escaped_line_whatever_the_definition_calls_itself() {
        let t = super::card_title("landing/ok");
        assert_eq!(t, "Workflow: approve ok");
        let t = super::card_title("x/a\n\u{202e}Approve now `y`");
        assert!(!t.contains('\n') && !t.contains('\u{202e}') && !t.contains("`y`"), "{t}");
        assert!(t.starts_with("Workflow: approve ") && t.contains("\\n") && t.contains("\\u{202e}"), "{t}");
    }
}
