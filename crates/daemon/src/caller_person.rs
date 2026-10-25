//! Whose a caller is, for what is kept per person (ov-747): the one rule
//! for Needs You (`needs_you.list` and the notices a connection is sent),
//! the read marks a read carries (`task.list`, `changes.inbox`,
//! `terminal.list`), and who a held ask's answer was (`terminal.agent_answer`).
//!
//! **The rule.**
//! - A remote device is its person (`people::person_of`): a member's phone
//!   is that member, a released one nobody's, any other the owner's.
//! - On the local socket, a caller is whoever the pane it runs in is:
//!   - a rep's pane is the rep's member, whatever login it runs on (it relays
//!     to them what's theirs);
//!   - any other pane on a member's login (`claude_logins`: their shell, their
//!     agent, an orchestrator they lead) is that member;
//!   - a pane on a login whose member was removed is nobody's (`Released`);
//!   - anything else, a pane on the runner's own login or a caller in no pane
//!     (the Mac app, the owner's own terminal), is the owner, as the local
//!     socket always was.
//!
//! So Bob typing `farcooler terminal agent-answer` in his own shell pane is
//! Bob, and his rep reads Needs You as Bob even on the runner's login. A pane
//! is whose login it runs on, not whose keyboard: the owner typing in Bob's
//! pane from the Mac app is Bob there too.
//!
//! **Cost.** The walk from a pid to its pane reads the whole process table
//! (`people::pane_of`). It runs only when some pane could be someone else's,
//! a rep or a pane on a member's login (two `EXISTS` reads), so a runner
//! with neither walks nothing and looks no one up. And at most once per
//! connection: `pane` is the connection's own (`RpcFactory`), since its
//! caller's pid is fixed for its life. Whose that pane is is read afresh on
//! each call, so a rep unbound or a member removed counts at once. A process
//! that leaves its pane after the first call keeps that pane's person on
//! that connection.
//!
//! What this decides is read and attributed, never permitted: whether a
//! caller may do a thing is `people::gate` and `caller_seat`, and a mark is
//! written by the device's person (`people::reader_of`).

use std::sync::OnceLock;

use farcooler_core::{DomainError, Result};
use farcooler_transport::Peer;
use uuid::Uuid;

use crate::claude_logins::Login;
use crate::people::{Person, person_of};
use crate::service::Service;

/// Whose `peer`, a local caller at `caller_pid`, is (the module's rule).
/// `pane` holds the connection's walk once made.
pub fn caller_person(svc: &Service, peer: &Peer, caller_pid: Option<i32>, pane: &OnceLock<Option<Uuid>>) -> Result<Person> {
    caller_person_with(svc, peer, caller_pid, pane, |pid| crate::people::pane_of(svc, pid))
}

/// `caller_person`, with the walk from a pid to its pane given.
pub(crate) fn caller_person_with(
    svc: &Service,
    peer: &Peer,
    caller_pid: Option<i32>,
    pane: &OnceLock<Option<Uuid>>,
    walk: impl FnOnce(i32) -> Option<Uuid>,
) -> Result<Person> {
    if peer.client_id.is_some() {
        return person_of(&svc.store, peer);
    }
    let Some(pid) = caller_pid else { return Ok(Person::Owner) };
    if pane.get().is_none() && !svc.store.any_reps()? && !svc.store.any_terminal_login()? {
        return Ok(Person::Owner);
    }
    match *pane.get_or_init(|| walk(pid)) {
        Some(pane) => pane_person(svc, pane),
        None => Ok(Person::Owner),
    }
}

/// Whose `pane` is: its rep's member, else whose login it runs on. A pane
/// gone since the walk is no one's pane, so the owner's, as a caller in none.
fn pane_person(svc: &Service, pane: Uuid) -> Result<Person> {
    if let Some(rep) = svc.store.rep_of_terminal(pane)? {
        return Ok(Person::Member(rep.handle));
    }
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
