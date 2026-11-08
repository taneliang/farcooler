//! ov-974: the owner's grant for one cutover by their orchestrator.
//!
//! The orchestrator asks on a card, with the option [`ALLOW`]; the owner
//! answers it from the Mac app or their phone. A live Prepare by the owner's
//! orchestrator is admitted only on such an answer, under [`MAX_AGE_MS`] old,
//! and spends it: the runner writes a note naming the answer and the attempt,
//! so the next Prepare needs a new one. The recovery steps after it ride the
//! attempt the Prepare pinned (`cutover_delegate`).
//!
//! **Where the seat is recorded.** An answer is the person's only when its
//! caller is walked to a person (`task_ops::answer_actor`, ov-630); an agent's
//! goes in as `manager`, `agent:…` or `unknown`, whatever `--actor` it claims.
//! That walk is a guardrail: an orphan an agent gives a terminal of its own
//! (`script`) reads as a person. So `task.note` also stamps where a person's
//! answer was said, [`SAID_FROM`], in the daemon's own words, never a
//! caller's (`task_ops::note_from` refuses one a caller sends), each from an
//! authenticated seat, never from the process walk:
//! - `app`: the answer carries a step-up signed by the Mac app's own key
//!   (`step_up::LOCAL_APP`), a Secure Enclave key that asks for Touch ID or
//!   the login password, over this very request; and the caller process
//!   satisfies the code signature of the app bundle this daemon ships in
//!   (`caller_seat::is_own_app`, `code_identity`). The signature is the
//!   owner's presence: anything may run the app's binary, and its
//!   accessibility can be driven, but neither puts a finger on the sensor.
//!   The bundle's CLI is another program, so `farcooler task answer` is not
//!   it;
//! - `device`: the answer carries a step-up (ov-856) signed by the owner's
//!   phone's trusted key over this very request (`step_up::OPTIONAL`), the key
//!   that asks for Face ID or the passcode; a browser's passkey doesn't count.
//!   The device must be bound to the owner (`owner_devices`, confirmed), and
//!   its key vouched for with the owner present: approved by the Mac app with
//!   Touch ID, or by a device of theirs already vouched for, with its own
//!   step-up (`step_up_rpc::vouch_of`). A key trusted on the walk alone (the
//!   CLI's `people approve`, the socket's `set_step_up_key` or enroll) signs
//!   admin actions as before, and never a grant;
//! - `cli`: anything else, the CLI in any terminal included.
//!
//! Only `app` and `device` grant.
use std::sync::Mutex;

use farcooler_core::{DomainError, Result};
use farcooler_store::Store;
use farcooler_store::models::{Actor, NoteKind, TaskNote};
use farcooler_transport::Peer;
use uuid::Uuid;

/// The option the owner picks.
pub const ALLOW: &str = "Allow cutover once";
/// How long an answer grants for.
pub const MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;
/// The key in a person's answer's `extra` that says where it was said.
pub const SAID_FROM: &str = "said_from";
/// The key in the runner's note that spends a grant.
const SPENDS: &str = "cutover_grant";
/// The key in the runner's note that gives one back.
const RETURNED: &str = "cutover_grant_returned";

/// Where a person's answer claiming `user` was said, by the daemon: `app`,
/// `device` or `cli`. `stepped` is the request's verified step-up, if any.
/// See this module's docs.
pub fn said_from(svc: &crate::service::Service, peer: &Peer, caller_pid: Option<i32>, stepped: Option<&crate::step_up::Stepped>) -> &'static str {
    said_from_with(peer, caller_pid, stepped, || peer.client_id.as_deref().is_some_and(|device| owners_vouched_device(&svc.store, device)), &crate::caller_seat::is_own_app)
}

/// Whether `device` is the owner's by the positive binding, and its key is
/// one the owner vouched for with presence (B1 of the second review): a key
/// trusted from the socket on the process walk alone never grants, and nor
/// does a device nobody bound ("unknown, so the owner's").
pub(crate) fn owners_vouched_device(store: &Store, device: &str) -> bool {
    let owners = crate::people::person_of_device(store, Some(device)).is_ok_and(|p| p == crate::people::Person::Owner);
    owners && store.is_confirmed_owner_device(device).unwrap_or(false) && store.step_up_vouched_by(device).ok().flatten().is_some()
}

/// `said_from` with its reads given. A device says it with its own step-up
/// key, over this very answer: the line's client id alone is a preamble any
/// local process may write, and an ssh session one any process of the
/// owner's may open.
pub(crate) fn said_from_with(
    peer: &Peer, caller_pid: Option<i32>, stepped: Option<&crate::step_up::Stepped>, owners_device: impl FnOnce() -> bool, is_app: &dyn Fn(i32) -> bool,
) -> &'static str {
    if let Some(device) = peer.client_id.as_deref() {
        let signed = stepped.is_some_and(|s| s.method == "task.note" && s.device == device && !s.browser);
        return if signed && owners_device() { "device" } else { "cli" };
    }
    // The app says it with its own key too, which asks for Touch ID or the
    // login password: running the app's binary passes its code signature.
    let signed = stepped.is_some_and(|s| s.method == "task.note" && s.device == crate::step_up::LOCAL_APP && !s.browser);
    match caller_pid {
        Some(pid) if signed && is_app(pid) => "app",
        _ => "cli",
    }
}

/// An answer that grants, on its card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub answer: Uuid,
    pub task: Uuid,
}

/// Held from checking a grant to spending it, so two Prepares never spend one.
static SPENDING: Mutex<()> = Mutex::new(());

/// The refusal when the owner hasn't allowed it.
fn not_allowed() -> DomainError {
    DomainError::refused(format!(
        "The owner hasn't allowed this cutover. Ask them on a card, with the option “{ALLOW}”, and they answer it from the Far Cooler app or their phone."
    ))
}

/// The newest grant on a card of `workspace`, or the refusal. Writes nothing:
/// the Prepare spends it last, just before the hold (`spend`).
pub fn required(store: &Store, workspace: Uuid, now_ms: i64) -> Result<Grant> {
    find(store, workspace, now_ms)?.ok_or_else(not_allowed)
}

/// Spend `grant` on `attempt`, if it still grants: the runner writes a note
/// naming the answer and the attempt (M1 of the second review: the last thing
/// before the hold mark, after the drain and the caller's recheck; a hold
/// that then fails gives it back, `give_back`).
pub fn spend(store: &Store, grant: &Grant, attempt: &str, now_ms: i64) -> Result<()> {
    let _held = SPENDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let notes = store.notes_for(grant.task, None)?;
    let Some(answer) = notes.iter().find(|n| n.id == grant.answer) else { return Err(not_allowed()) };
    if !grants(answer, now_ms) || !asked(&notes, answer) || !spent_on(&notes, grant.answer).is_empty() {
        return Err(not_allowed());
    }
    store.add_note(grant.task, NoteKind::Progress, Actor::Runner, &format!("The owner's “{ALLOW}” was spent on cutover attempt {attempt}."),
        serde_json::json!({ SPENDS: { "answer": grant.answer.to_string(), "attempt": attempt } }))?;
    Ok(())
}

/// Give `grant` back from `attempt`, whose hold failed: nothing was held, so
/// the owner's word is still good for the next Prepare.
pub fn give_back(store: &Store, grant: &Grant, attempt: &str) -> Result<()> {
    store.add_note(grant.task, NoteKind::Progress, Actor::Runner, &format!("The owner's “{ALLOW}” was given back: cutover attempt {attempt} wasn't held."),
        serde_json::json!({ RETURNED: { "answer": grant.answer.to_string(), "attempt": attempt } }))?;
    Ok(())
}

/// The newest unspent grant on a card of `workspace`, if any. Read by a plain
/// query, not the full-text index (M5 of the second review).
pub fn find(store: &Store, workspace: Uuid, now_ms: i64) -> Result<Option<Grant>> {
    for answer in store.owner_answers_since(workspace, now_ms.saturating_sub(MAX_AGE_MS))? {
        if !grants(&answer, now_ms) {
            continue;
        }
        let notes = store.notes_for(answer.task_id, None)?;
        if asked(&notes, &answer) && spent_on(&notes, answer.id).is_empty() {
            return Ok(Some(Grant { answer: answer.id, task: answer.task_id }));
        }
    }
    Ok(None)
}

fn same(a: &str, b: &str) -> bool { a.trim().to_lowercase() == b.trim().to_lowercase() }

/// Whether `body` is the owner's [`ALLOW`], as the apps send it.
pub fn is_grant(body: &str) -> bool { same(body, ALLOW) }

/// The owner's own `ALLOW`, said from the app or a device, young enough.
fn grants(note: &TaskNote, now_ms: i64) -> bool {
    note.kind == NoteKind::Answer
        && note.actor == Actor::User
        && same(&note.body, ALLOW)
        && matches!(note.extra.get(SAID_FROM).and_then(|v| v.as_str()), Some("app" | "device"))
        && note.extra.get("message").is_none()
        && note.at <= now_ms
        && now_ms - note.at <= MAX_AGE_MS
}

/// Whether `answer` is the one answer to its card's latest question, and that
/// question offered `ALLOW` (M2 of the second review). A question asked since
/// replaces it; another answer of the person's to it, before or after (a
/// "Wait for me", or the same words again), means this one no longer stands.
/// An answer a later note corrects is not counted; the correction is.
fn asked(notes: &[TaskNote], answer: &TaskNote) -> bool {
    let Some(question) = notes.iter().rposition(|n| n.kind == NoteKind::Question) else { return false };
    let offered = notes[question].extra.get("options").and_then(|o| o.as_array())
        .is_some_and(|o| o.iter().any(|v| v.as_str().is_some_and(|v| same(v, ALLOW))));
    let corrected = |id: Uuid| notes.iter().any(|n| n.supersedes == Some(id));
    let answers: Vec<Uuid> = notes[question + 1..].iter()
        .filter(|n| n.kind == NoteKind::Answer && n.actor == Actor::User && !corrected(n.id))
        .map(|n| n.id)
        .collect();
    offered && answers == [answer.id]
}

/// The attempts the runner spent `answer` on and didn't give back.
fn spent_on(notes: &[TaskNote], answer: Uuid) -> Vec<String> {
    let id = answer.to_string();
    let by = |key: &str| -> Vec<String> {
        notes.iter()
            .filter(|n| n.actor == Actor::Runner)
            .filter_map(|n| n.extra.get(key))
            .filter(|s| s.get("answer").and_then(|a| a.as_str()) == Some(id.as_str()))
            .map(|s| s.get("attempt").and_then(|a| a.as_str()).unwrap_or_default().to_string())
            .collect()
    };
    let mut returned = by(RETURNED);
    by(SPENDS).into_iter().filter(|attempt| match returned.iter().position(|r| r == attempt) {
        Some(at) => { returned.remove(at); false }
        None => true,
    }).collect()
}

#[cfg(test)]
#[path = "cutover_grant_tests.rs"]
mod tests;
