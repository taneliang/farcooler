//! Who a caller is on a shared runner, and the rules that follow from it
//! (ov-494, phase 1 of `reports/ov-479/design.md`, with the owner's decisions
//! on ov-479 and R-49).
//!
//! **A person is worked out from the connection, never from the request.**
//! sshd's forced command names the device (`Peer::client_id`), and a runner
//! admin binds devices to members (`member.set`). The local socket, and any
//! device bound to nobody, is the runner's owner: the ceremony still enrolls
//! only the owner's own devices (`ceremony::accept_offer`).
//!
//! **Only a workspace's lead talks to its orchestrator** (ov-479 D1). Typing
//! into the orchestrator's pane, in any of the ways a client can (`gate`), and
//! a person's `message.send`, are refused as `not_lead` for anyone else.
//! Everyone else goes through their rep. The lead is the owner until a runner
//! admin hands it on (R-49).
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
    /// The runner's owner: the local socket, or a device bound to nobody.
    Owner,
    /// A member, by handle.
    Member(String),
}

/// Who `peer` is. A store that can't be read refuses the call rather than
/// guessing the owner.
pub fn person_of(store: &Store, peer: &Peer) -> Result<Person> {
    match peer.client_id.as_deref() {
        None => Ok(Person::Owner),
        Some(device) => Ok(store.member_of_device(device)?.map_or(Person::Owner, Person::Member)),
    }
}

/// Whether `person` leads `workspace`: the member it was handed to, or with
/// none, the owner. Exactly one person does.
pub fn leads(store: &Store, person: &Person, workspace: Uuid) -> Result<bool> {
    Ok(match (person, store.workspace_lead(workspace)?) {
        (Person::Owner, None) => true,
        (Person::Member(handle), Some(lead)) => *handle == lead,
        _ => false,
    })
}

/// The pane a request types into, or presses a key or answers a prompt in:
/// every way a client puts words in front of an agent.
fn typed_into(req: &Request) -> Option<Uuid> {
    let named = |id: &[u8]| crate::wire::parse_id(id);
    match (req.method.as_str(), req.payload.as_ref()) {
        ("terminal.write" | "terminal.paste_file", _) => req.target_resource_id.as_deref().and_then(named),
        (_, Some(request::Payload::AgentPrompt(p))) => named(&p.terminal_id),
        (_, Some(request::Payload::AgentCancel(p))) => named(&p.terminal_id),
        (_, Some(request::Payload::BringDraft(p))) => named(&p.terminal_id),
        (_, Some(request::Payload::DraftWithdraw(p))) => named(&p.terminal_id),
        (_, Some(request::Payload::AgentAnswer(p))) => named(&p.terminal_id),
        _ => None,
    }
}

/// Refuse `req` as `not_lead` when it types into a workspace's orchestrator
/// and `peer` isn't that workspace's lead. Anything else passes, to be
/// answered by its own handler: a pane that isn't there is `NotFound` there.
pub fn gate(store: &Store, peer: &Peer, req: &Request) -> Result<()> {
    let Some(pane) = typed_into(req) else { return Ok(()) };
    let Ok(terminal) = store.get_terminal(pane) else { return Ok(()) };
    let Some(workspace) = terminal.workspace_id.filter(|_| terminal.role == TerminalRole::Orchestrator) else {
        return Ok(());
    };
    if leads(store, &person_of(store, peer)?, workspace)? {
        return Ok(());
    }
    tracing::info!(device = peer.client_id.as_deref().unwrap_or("-"), method = %req.method, "refused: only the lead talks to the orchestrator");
    Err(DomainError::Conflict { what: "not_lead" })
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
