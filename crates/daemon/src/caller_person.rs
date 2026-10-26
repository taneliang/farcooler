//! Whose a caller is, for what is kept per person (ov-747): the one rule
//! for Needs You (`needs_you.list` and the notices a connection is sent),
//! the read marks a call reads (`task.list`, `changes.inbox`,
//! `terminal.list`, a pane's row and a board's read state in events) and
//! writes (`terminal.seen`, `terminal.watching`, `workspace.mark_read`,
//! `changes.mark_read`), and who a held ask's answer was
//! (`terminal.agent_answer`).
//!
//! **The rule.**
//! - A remote device is its person (`people::person_of`): a member's phone
//!   is that member, a released one nobody's, any other the owner's.
//! - On the local socket, a caller is whoever the pane it runs in is:
//!   - a rep's pane is the rep's member (it relays to them what's theirs; it
//!     runs on their login, `login_for_new_pane`);
//!   - any other pane on a member's login (`claude_logins`: their shell, their
//!     agent, an orchestrator they lead) is that member;
//!   - a pane on a login whose member was removed is nobody's (`Released`);
//!   - anything else, a pane on the runner's own login or a caller in no pane
//!     (the Mac app, the owner's own terminal), is the owner, as the local
//!     socket always was.
//!
//! So Bob typing `farcooler terminal agent-answer` in his own shell pane is
//! Bob, as his rep reads Needs You and his marks as Bob. A pane is whose
//! login it runs on, not whose keyboard: the owner typing in Bob's pane
//! from the Mac app is Bob there too.
//!
//! **Cost.** The walk from a pid to its pane reads the whole process table
//! (`people::pane_of`). It runs only when some pane could be someone else's,
//! a rep or a pane on a member's login (two `EXISTS` reads), so a runner
//! with neither walks nothing and looks no one up. And at most once per
//! connection: `pane` is the connection's own (`RpcFactory`), since its
//! caller's pid is fixed for its life. Only a pane found is kept: a walk
//! that found none (in no pane, or an inventory not yet read) is made again
//! on the next call, so a caller is never kept as the owner for having
//! asked too early. Whose that pane is is read afresh on each call, so a
//! rep unbound or a member removed counts at once. A process that leaves
//! its pane after the first call keeps that pane's person on that
//! connection.
//!
//! What this decides is whose marks and whose answer, never what is
//! permitted: whether a caller may do a thing is `people::gate` and
//! `caller_seat`. A released person's writes are refused or do nothing
//! (`people::reader_of`, `Watcher::mark_seen_by`).

use std::sync::OnceLock;

use farcooler_core::{DomainError, Result};
use farcooler_transport::Peer;
use uuid::Uuid;

use crate::claude_logins::Login;
use crate::people::{Person, person_of};
use crate::service::Service;

/// Whose `peer`, a local caller at `caller_pid`, is (the module's rule).
/// `pane` holds the pane the connection's walk found, once it found one.
pub fn caller_person(svc: &Service, peer: &Peer, caller_pid: Option<i32>, pane: &OnceLock<Uuid>) -> Result<Person> {
    caller_person_with(svc, peer, caller_pid, pane, |pid| crate::people::pane_of(svc, pid))
}

/// Who `peer`, a local caller at `caller_pid`, is, as the people log writes
/// it (ov-757): `member:<handle>` for a member's device or a pane on their
/// login, `released` for a pane on a removed member's, and `user` for the
/// owner. Walked afresh: the log is written rarely.
pub fn log_actor(svc: &Service, peer: &Peer, caller_pid: Option<i32>) -> Result<String> {
    Ok(match caller_person(svc, peer, caller_pid, &OnceLock::new())? {
        Person::Member(handle) => format!("member:{handle}"),
        Person::Released => "released".to_string(),
        Person::Owner => "user".to_string(),
    })
}

/// `caller_person`, with the walk from a pid to its pane given.
pub(crate) fn caller_person_with(
    svc: &Service,
    peer: &Peer,
    caller_pid: Option<i32>,
    pane: &OnceLock<Uuid>,
    walk: impl FnOnce(i32) -> Option<Uuid>,
) -> Result<Person> {
    if peer.client_id.is_some() {
        return person_of(&svc.store, peer);
    }
    let Some(pid) = caller_pid else { return Ok(Person::Owner) };
    if pane.get().is_none() && !svc.store.any_reps()? && !svc.store.any_terminal_login()? {
        return Ok(Person::Owner);
    }
    // Only a pane found is kept: none found is asked again next time.
    let found = match pane.get() {
        Some(kept) => Some(*kept),
        None => walk(pid).inspect(|found| {
            let _ = pane.set(*found);
        }),
    };
    match found {
        Some(found) => pane_person(svc, found),
        None => Ok(Person::Owner),
    }
}

/// Whose `pane` is: whose login it runs on (`recorded_login`), which for a
/// rep is its member's, recorded at launch or, for a rep from before logins
/// were recorded, read off the rep. A pane gone since the walk is no one's
/// pane, so the owner's, as a caller in none.
fn pane_person(svc: &Service, pane: Uuid) -> Result<Person> {
    let term = match svc.store.get_terminal(pane) {
        Err(DomainError::NotFound) => return Ok(Person::Owner),
        other => other?,
    };
    let login = svc.recorded_login(&term)?;
    let Login::Member { handle, .. } = &login else { return Ok(Person::Owner) };
    match svc.refuse_a_gone_generation(&login) {
        Ok(()) => Ok(Person::Member(handle.clone())),
        Err(DomainError::Conflict { what: "login_gone" }) => Ok(Person::Released),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
#[path = "caller_person_tests.rs"]
mod tests;
