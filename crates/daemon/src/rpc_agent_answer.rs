//! `terminal.agent_answer`: a person's answer to a pane's prompt.
//!
//! Two kinds of prompt arrive here. claude's folder-trust prompt, pressed in
//! its pane (ov-493). And an ask a claude TUI's hook holds (`hook_asks`): a
//! permission's allow or deny, a plan's approval, or an AskUserQuestion's
//! answers (`hook_asks::shape::decide`).
//!
//! **Only a person answers either** (R-78, ov-647). Who the caller is comes
//! from its process (`caller_seat`), never from what it says: a claude in a
//! shell pane, or in no pane at all, runs `farcooler terminal agent-answer`
//! with no `FARCOOLER_ACTOR`, and its CLI claims `user`. An agent's answer, or
//! one from a caller the walk can't read, is refused `not_a_person`, which
//! the CLI and the runner's table say. The request's `actor` can only make
//! this stricter: anything but `user` (or nothing) is refused too, whatever
//! the process says, so a pane that names itself is taken at its word.
//!
//! A remote device (a phone, the web, a watch through its phone) reaches the
//! runner through `farcoolerd --stdio`, either in-process (no local pid: a
//! person) or piped to the daemon's socket, whose process walks up to pid 1
//! with no agent above it: a person too. The Mac app is a person the same way.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::AgentAnswer;
use farcooler_transport::Peer;
use uuid::Uuid;

use crate::caller_seat::Seat;
use crate::service::Service;
use crate::watch::Watcher;

/// Answer `p` for the caller `peer`, whose local process is `caller_pid`.
/// Returns the pane answered, for the caller to read back.
pub(crate) async fn answer(svc: &Service, watcher: &Watcher, peer: &Peer, caller_pid: Option<i32>, p: AgentAnswer) -> Result<Uuid> {
    let id = crate::wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
    if let Ok(actor) = crate::task_ops::actor_from_wire(&p.actor) {
        crate::people::refuse_rep_answer(&svc.store, actor, None)?; // a rep answers nothing (ov-494)
    }
    // claude's folder-trust prompt, answered from Needs You (ov-493). The
    // route refuses anyone but a person (`answer_wake::route::deliver`).
    if p.request_id.starts_with(crate::needs_you::TRUST_ASK_PREFIX) {
        use crate::watch::answer_wake::route::{Author, Delivery, Press};
        let press = Press::Trust { ask: p.request_id.clone(), option: p.option_id.clone() };
        let author = Author::of(seat_of(svc, caller_pid, &p.actor).await);
        watcher.deliver(id, Delivery { author, ..Delivery::key(press) }).await?;
        watcher.mark_seen(id).await;
        return Ok(id);
    }
    // A claude TUI's hook holds every ask there is to answer. An id it
    // doesn't hold is a conflict: answered, withdrawn, from before a restart,
    // or a chat's from before chat mode was retired (ov-377).
    if !p.request_id.starts_with(crate::hook_asks::HOOK_ASK_PREFIX) {
        return Err(DomainError::Conflict { what: "not_held" });
    }
    may_answer(seat_of(svc, caller_pid, &p.actor).await)?;
    let label = match peer.client_id.as_deref() {
        Some(client) => crate::enrollment::label_for(svc, client).await,
        None => None,
    };
    let decider = decider_name(peer.client_id.is_none(), label.as_deref());
    let asks = svc.hooks().asks();
    asks.answer_with(id, &p.request_id, &p.option_id, &p.answers, &decider).await.map_err(|refused| {
        use crate::hook_asks::AnswerRefused;
        match refused {
            AnswerRefused::UnknownOption => DomainError::InvalidArgument { what: "option_id" },
            AnswerRefused::Unanswered => DomainError::InvalidArgument { what: "answers" },
            // Named, so a client can say which: someone else answered, or
            // this answer never reached the hook.
            AnswerRefused::NotHeld => DomainError::Conflict { what: "not_held" },
            AnswerRefused::NotDelivered => DomainError::Conflict { what: "not_delivered" },
        }
    })?;
    // The same call `terminal.seen` makes: answering is only reachable by
    // having looked, so it ends `Done` the same way. Against the WATCHER,
    // which is the one place `Done` lives.
    watcher.mark_seen(id).await;
    Ok(id)
}

/// Who the caller is (`caller_seat`), unless its `actor` already said it
/// isn't the person (`claims_a_person`): then never a person.
async fn seat_of(svc: &Service, caller_pid: Option<i32>, actor: &str) -> Seat {
    if claims_a_person(actor) { crate::caller_seat::caller_seat(svc, caller_pid).await } else { Seat::Unreadable }
}

/// Whether a request's `actor` leaves the caller's process to say who it is:
/// empty or `user`. Any other (`manager`, `agent:<id>`, the CLI's `pane` from
/// before ov-647) is someone who said they aren't the person.
fn claims_a_person(actor: &str) -> bool {
    actor.is_empty() || actor == "user"
}

/// Refuse a hook ask's answer from anyone but a person: an agent above the
/// caller, in a pane or in none, or a caller the walk couldn't read.
fn may_answer(seat: Seat) -> Result<()> {
    if seat == Seat::Person {
        return Ok(());
    }
    tracing::warn!(?seat, "refused an answer to a held ask from a caller who isn't a person");
    Err(DomainError::Conflict { what: "not_a_person" })
}

/// The name a deny carries for the person who sent it: "Denied from iPhone".
///
/// Only a person gets this far (`may_answer`), so the name is where they
/// answered from. `local` is a caller on this runner's own socket, the Mac
/// app or the CLI in the person's own terminal, and it has no enrollment to
/// name it, so it is named for the computer the runner is (`local_name`). A
/// remote device is named by its enrollment label, which is whatever a device
/// sent when it paired, so it is cut to one short line: a newline or a
/// control character in a message claude shows the model is not something a
/// device gets to put there.
fn decider_name(local: bool, label: Option<&str>) -> String {
    /// Long enough for any device name a person chose; short enough to stay
    /// one line in the model's context.
    const LONGEST_NAME: usize = 40;
    if local {
        return local_name(cfg!(target_os = "macos")).to_string();
    }
    let clean: String = label.unwrap_or_default().chars().filter(|c| !c.is_control()).collect();
    let name: String = clean.trim().chars().take(LONGEST_NAME).collect();
    let name = name.trim_end();
    if name.is_empty() { "a paired device".to_string() } else { name.to_string() }
}

/// What the computer the runner runs on is called in a deny from its own
/// socket. "Mac" is only true on a Mac; a Linux runner's local caller is the
/// CLI on that machine.
fn local_name(macos: bool) -> &'static str {
    if macos { "Mac" } else { "this computer" }
}

#[cfg(test)]
#[path = "rpc_agent_answer_tests.rs"]
mod tests;
