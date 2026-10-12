//! Who a caller is on a shared runner, and the rules that follow from it
//! (ov-494, phase 1 of `reports/ov-479/design.md`, with the owner's decisions
//! on ov-479 and R-49).
//!
//! **A person is worked out from the connection, never from the request.**
//! sshd's forced command names the device (`Peer::client_id`), and a runner
//! admin binds devices to members (`member.set`). The local socket, and any
//! device never bound to anybody, is the runner's owner: the ceremony still
//! enrolls only the owner's own devices (`ceremony::accept_offer`). A device a
//! member had and doesn't now is nobody's (`Person::Released`).
//!
//! **Only a workspace's lead talks to its orchestrator** (ov-479 D1), and the
//! runner's owner always may (R-54). Acting on the orchestrator's pane, in any
//! of the ways a client can (`gate`), and a person's `message.send`, are
//! refused as `not_lead` for anyone else, under its own code
//! (`DomainError::NotYours`). Everyone else goes through their rep. The lead
//! is the owner until a runner admin hands it on (R-49).
//!
//! **A rep is its person's.** Only that member, and the owner, type into,
//! stop, restart or remove it (`gate`, `not_yours`), so what it carries is
//! theirs. And it speaks only as itself: a caller running in a rep's pane,
//! told by its pid, can't write as the owner or the orchestrator.
//!
//! **A rep is only a messenger** (ov-479 D3). It relays its person's asks and
//! answers to the orchestrator, and the orchestrator's questions back. The
//! runner refuses it an answer or a decision note (`refuse_rep_answer`) and
//! answering a prompt: an answer it relays is the person's words in a
//! message, which the orchestrator records as theirs.
//!
//! **A guardrail, not a boundary.** A pane's actor is an environment variable
//! anything in the pane can change, and anyone with Control can already type
//! into any pane on the runner, the orchestrator's included, through a shell
//! (`terminal.write` into one, or Key B). These rules keep the shape the owner
//! chose, and stop mistakes and casual prompt injection, not someone
//! determined: the hard version needs the directory's per-person grants and a
//! rep with no shell (design §7).

use std::path::Path;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{Request, request};
use farcooler_store::Store;
use farcooler_store::models::{Actor, NoteKind, TerminalRole};
use farcooler_transport::Peer;
use uuid::Uuid;

/// Who a caller is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Person {
    /// The runner's owner: the local socket, or a device never bound to anybody.
    Owner,
    /// A member, by handle.
    Member(String),
    /// A device that was a member's and is nobody's now (`Store::device_released`):
    /// never the owner, so removing someone gives their phone no say.
    Released,
}

impl Person {
    /// Whether this is someone other than the owner: a member, or a released device.
    pub fn is_someone_else(&self) -> bool {
        !matches!(self, Person::Owner)
    }
}

/// Who `peer` is. A store that can't be read refuses the call rather than
/// guessing the owner.
pub fn person_of(store: &Store, peer: &Peer) -> Result<Person> {
    let Some(device) = peer.client_id.as_deref() else { return Ok(Person::Owner) };
    if let Some(handle) = store.member_of_device(device)? {
        return Ok(Person::Member(handle));
    }
    Ok(if store.device_released(device)? { Person::Released } else { Person::Owner })
}

/// Whether `person` leads `workspace`: the member it was handed to, or with
/// none, the owner. Exactly one person does; it's who a person's message is
/// tagged as coming from with authority.
pub fn leads(store: &Store, person: &Person, workspace: Uuid) -> Result<bool> {
    Ok(match (person, store.workspace_lead(workspace)?) {
        (Person::Owner, None) => true,
        (Person::Member(handle), Some(lead)) => *handle == lead,
        _ => false,
    })
}

/// Whether `person` may talk to `workspace`'s orchestrator directly: its
/// lead, and the runner's owner always (R-54). The owner runs the runner and
/// pays for its account, and can take the lead back at any moment, so
/// refusing them would stop no one and take away their Stop.
pub fn talks_to_orchestrator(store: &Store, person: &Person, workspace: Uuid) -> Result<bool> {
    Ok(*person == Person::Owner || leads(store, person, workspace)?)
}

/// The pane a request acts on as a person would: types into, presses a key
/// or answers a prompt in, or stops, restarts, removes or recasts. Every
/// way a client can speak for, or silence, the agent in it.
fn acted_on(req: &Request) -> Option<Uuid> {
    let named = |id: &[u8]| crate::wire::parse_id(id);
    match (req.method.as_str(), req.payload.as_ref()) {
        (
            "terminal.write" | "terminal.paste_file" | "terminal.stop" | "terminal.restart" | "terminal.remove"
            | "terminal.set_role",
            _,
        ) => req.target_resource_id.as_deref().and_then(named),
        (_, Some(request::Payload::AgentPrompt(p))) => named(&p.terminal_id),
        (_, Some(request::Payload::AgentCancel(p))) => named(&p.terminal_id),
        (_, Some(request::Payload::BringDraft(p))) => named(&p.terminal_id),
        (_, Some(request::Payload::DraftWithdraw(p))) => named(&p.terminal_id),
        (_, Some(request::Payload::AgentAnswer(p))) => named(&p.terminal_id),
        _ => None,
    }
}

/// The actor a request says it writes as, for the writes a rep could use to
/// speak for its person: a note, an answer to a prompt, a message.
fn claimed_actor(req: &Request) -> Option<&str> {
    match req.payload.as_ref()? {
        request::Payload::TaskNoteAppend(p) => Some(&p.actor),
        request::Payload::AgentAnswer(p) => Some(&p.actor),
        request::Payload::MessageSend(p) => Some(&p.actor),
        _ => None,
    }
}

/// The gate every request passes before dispatch (`Rpc::handle`).
///
/// - **A rep's pane speaks as itself** (ov-479 D3). A local caller whose
///   process runs in a rep's pane (`rep_pane_of`, from its pid, never from
///   what it claims) may write a note, answer or message only as that pane:
///   any other actor, `user` and `manager` included, is refused `actor`. Its
///   own actor then meets `refuse_rep_answer`.
/// - **A rep is its person's.** Acting on a rep's pane is refused
///   `not_yours` for anyone but its member and the owner.
/// - **Only the lead, and the owner (R-54), talk to the orchestrator.**
///   Acting on its pane is refused `not_lead` for anyone else.
///
/// Anything else passes, to be answered by its own handler: a pane that
/// isn't there is `NotFound` there.
pub fn gate(svc: &crate::service::Service, peer: &Peer, caller_pid: Option<i32>, req: &Request) -> Result<()> {
    let store = &svc.store;
    if let Some(claimed) = claimed_actor(req)
        && let Some(rep) = caller_pid.filter(|_| store.any_reps().unwrap_or(true)).and_then(|pid| rep_pane_of(svc, pid))
        && claimed != format!("agent:{rep}")
    {
        return Err(DomainError::InvalidArgument { what: "actor" });
    }
    let Some(pane) = acted_on(req) else { return Ok(()) };
    let Ok(terminal) = store.get_terminal(pane) else { return Ok(()) };
    if let Some(rep) = store.rep_of_terminal(pane)? {
        return match person_of(store, peer)? {
            Person::Member(handle) if handle == rep.handle => Ok(()),
            Person::Owner => Ok(()),
            _ => Err(DomainError::NotYours { what: "not_yours" }),
        };
    }
    let Some(workspace) = terminal.workspace_id.filter(|_| terminal.role == TerminalRole::Orchestrator) else {
        return Ok(());
    };
    if talks_to_orchestrator(store, &person_of(store, peer)?, workspace)? {
        return Ok(());
    }
    tracing::info!(device = peer.client_id.as_deref().unwrap_or("-"), method = %req.method, "refused: only the lead talks to the orchestrator");
    Err(DomainError::NotYours { what: "not_lead" })
}

/// The rep pane process `pid` runs in, if any: `pid` or one of its ancestors
/// is the pane's own process (`ScreenStamp::pid`). Read off the process
/// table, so a rep can't name itself out of it.
pub fn rep_pane_of(svc: &crate::service::Service, pid: i32) -> Option<Uuid> {
    let panes: std::collections::HashMap<i32, Uuid> = svc
        .inventory_snapshot()
        .panes
        .iter()
        .filter(|p| svc.store.rep_of_terminal(p.terminal_id).ok().flatten().is_some())
        .map(|p| (p.stamp.pid as i32, p.terminal_id))
        .collect();
    if panes.is_empty() {
        return None;
    }
    let table = crate::proc_table::snapshot()?;
    let parents: std::collections::HashMap<i32, i32> = table
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?))
        })
        .collect();
    pane_among(pid, &parents, &panes)
}

/// The pane in `panes` (by its process's pid) that `pid` descends from,
/// walking `parents` up at most 64 generations.
pub fn pane_among(
    pid: i32,
    parents: &std::collections::HashMap<i32, i32>,
    panes: &std::collections::HashMap<i32, Uuid>,
) -> Option<Uuid> {
    let mut at = pid;
    for _ in 0..64 {
        if let Some(pane) = panes.get(&at) {
            return Some(*pane);
        }
        match parents.get(&at) {
            Some(&up) if up > 1 && up != at => at = up,
            _ => return None,
        }
    }
    None
}

/// Refuse a rep's answer or decision note, or its answer to a prompt, as
/// `rep_messenger`: it relays its person's answers to the orchestrator in a
/// message, and never gives one itself.
pub fn refuse_rep_answer(store: &Store, actor: Actor, kind: Option<NoteKind>) -> Result<()> {
    let Actor::Agent { terminal } = actor else { return Ok(()) };
    let answering = kind.is_none_or(|k| matches!(k, NoteKind::Answer | NoteKind::Decision));
    if answering && store.rep_of_terminal(terminal)?.is_some() {
        return Err(DomainError::Conflict { what: "rep_messenger" });
    }
    Ok(())
}

/// The Claude account this runner's user is signed in to, which every
/// Claude Code pane on it uses (ov-479 D2): the email in Claude Code's own
/// `.claude.json`, under `$CLAUDE_CONFIG_DIR` when that's set, else in the
/// home directory. Read only. `None` when it can't be read.
pub fn claude_account(home: Option<&Path>, config_dir: Option<&Path>) -> Option<String> {
    let file = match config_dir {
        Some(dir) => dir.join(".claude.json"),
        None => home?.join(".claude.json"),
    };
    let text = std::fs::read_to_string(file).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let email = json.pointer("/oauthAccount/emailAddress")?.as_str()?.trim();
    (!email.is_empty() && email.len() <= 254 && !email.chars().any(char::is_control)).then(|| email.to_string())
}

/// `claude_account` for this daemon's own user.
pub fn runner_claude_account() -> Option<String> {
    let home = directories::UserDirs::new().map(|d| d.home_dir().to_path_buf());
    let config = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    claude_account(home.as_deref(), config.as_deref())
}

#[cfg(test)]
#[path = "people_tests.rs"]
mod tests;
