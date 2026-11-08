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
//! - `app`: the caller process satisfies the code signature of the app bundle
//!   this daemon ships in (`caller_seat::is_own_app`, `code_identity`). The
//!   bundle's CLI is another program, so `farcooler task answer` is not it;
//! - `device`: the answer carries a step-up (ov-856) signed by the owner's
//!   phone's trusted key over this very request (`step_up::OPTIONAL`), the key
//!   that asks for Face ID or the passcode; a browser's passkey doesn't count;
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

/// Where a person's answer claiming `user` was said, by the daemon: `app`,
/// `device` or `cli`. `stepped` is the request's verified step-up, if any.
/// See this module's docs.
pub fn said_from(svc: &crate::service::Service, peer: &Peer, caller_pid: Option<i32>, stepped: Option<&crate::step_up::Stepped>) -> &'static str {
    said_from_with(peer, caller_pid, stepped,
        || crate::people::person_of(&svc.store, peer).is_ok_and(|p| p == crate::people::Person::Owner),
        &crate::caller_seat::is_own_app)
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
    match caller_pid {
        Some(pid) if is_app(pid) => "app",
        _ => "cli",
    }
}

/// An answer that grants, on its card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub answer: Uuid,
    pub task: Uuid,
}

/// Held from finding a grant to spending it, so two Prepares never spend one.
static SPENDING: Mutex<()> = Mutex::new(());

/// Spend the newest grant on a card of `workspace` for `attempt`, or refuse.
pub fn spend(store: &Store, workspace: Uuid, attempt: &str, now_ms: i64) -> Result<Grant> {
    let _held = SPENDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(grant) = find(store, workspace, now_ms)? else {
        return Err(DomainError::refused(format!(
            "The owner hasn't allowed this cutover. Ask them on a card, with the option “{ALLOW}”, and they answer it from the Far Cooler app or their phone."
        )));
    };
    store.add_note(grant.task, NoteKind::Progress, Actor::Runner, &format!("The owner's “{ALLOW}” was spent on cutover attempt {attempt}."),
        serde_json::json!({ SPENDS: { "answer": grant.answer.to_string(), "attempt": attempt } }))?;
    Ok(grant)
}

/// The newest unspent grant on a card of `workspace`, if any.
pub fn find(store: &Store, workspace: Uuid, now_ms: i64) -> Result<Option<Grant>> {
    let repository = store.get_workspace(workspace)?.repository_id;
    for hit in store.search_notes(repository, ALLOW, Some(NoteKind::Answer))?.into_iter().rev() {
        if hit.superseded || !grants(&hit.note, now_ms) || store.get_task(hit.note.task_id)?.workspace_id != workspace {
            continue;
        }
        let notes = store.notes_for(hit.note.task_id, None)?;
        if asked(&notes, &hit.note) && !spent(&notes, hit.note.id) {
            return Ok(Some(Grant { answer: hit.note.id, task: hit.note.task_id }));
        }
    }
    Ok(None)
}

fn same(a: &str, b: &str) -> bool { a.trim().to_lowercase() == b.trim().to_lowercase() }

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

/// Whether a question before `answer` on its card offered `ALLOW`.
fn asked(notes: &[TaskNote], answer: &TaskNote) -> bool {
    notes.iter().take_while(|n| n.id != answer.id).any(|n| {
        n.kind == NoteKind::Question
            && n.extra.get("options").and_then(|o| o.as_array()).is_some_and(|o| o.iter().any(|v| v.as_str().is_some_and(|v| same(v, ALLOW))))
    })
}

/// Whether the runner has spent `answer`.
fn spent(notes: &[TaskNote], answer: Uuid) -> bool {
    let id = answer.to_string();
    notes.iter().any(|n| n.actor == Actor::Runner && n.extra.get(SPENDS).and_then(|s| s.get("answer")).and_then(|a| a.as_str()) == Some(id.as_str()))
}

#[cfg(test)]
#[path = "cutover_grant_tests.rs"]
mod tests;
