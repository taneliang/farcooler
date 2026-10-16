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
//! message, which the orchestrator records as theirs. And a process in a
//! rep's pane gets only reads, messages and comments (`rep_may`, R-66):
//! everything else is refused `rep_messenger` too.
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

/// Whether `person`, on a connection holding `scope`, may start `handle`'s
/// rep (`rep.start`), and so wake it with a message: a runner admin or the
/// owner starts anyone's, a member their own.
pub fn may_start_rep(person: &Person, scope: farcooler_protocol::v1::Scope, handle: &str) -> bool {
    scope == farcooler_protocol::v1::Scope::HostAdmin
        || *person == Person::Owner
        || matches!(person, Person::Member(me) if me == handle)
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

/// Whether `req` switches the nudge on, which posts to GitHub as the owner.
fn turns_nudging_on(req: &Request) -> bool {
    matches!(req.payload.as_ref(), Some(request::Payload::WorkspaceSetSettings(p)) if p.pr_nudge == Some(true))
}

/// The gate every request passes before dispatch (`Rpc::handle`).
///
/// - **A rep's pane only relays** (R-66). A local caller whose process runs
///   in a rep's pane gets the methods `rep_may` lists, and every other is
///   refused `rep_messenger`: no plan, theme, lane, ruling or train write, no
///   dispatch, no task made or changed, no worktree or pane made or acted on,
///   no member or workspace setting.
/// - **A rep's pane speaks as itself** (ov-479 D3). A local caller whose
///   process runs in a rep's pane (`rep_pane_of`, from its pid, never from
///   what it claims) may write a note, answer or message only as that pane:
///   any other actor, `user` and `manager` included, is refused `actor`. Its
///   own actor then meets `refuse_rep_answer`.
/// - **A rep is its person's.** Acting on a rep's pane is refused
///   `not_yours` for anyone but its member and the owner.
/// - **Only a person turns the nudge on** (ov-580). A local caller whose
///   process runs in an agent pane is refused `person_only`.
/// - **Only the lead, and the owner (R-54), talk to the orchestrator.**
///   Acting on its pane is refused `not_lead` for anyone else.
///
/// Anything else passes, to be answered by its own handler: a pane that
/// isn't there is `NotFound` there.
pub fn gate(svc: &crate::service::Service, peer: &Peer, caller_pid: Option<i32>, req: &Request) -> Result<()> {
    let store = &svc.store;
    let speaks_for = claimed_actor(req).is_some() || acted_on(req).is_some();
    let caller_rep = caller_pid
        .filter(|_| (speaks_for || !rep_may(req)) && store.any_reps().unwrap_or(true))
        .and_then(|pid| rep_pane_of(svc, pid));
    if caller_rep.is_some() && !rep_may(req) {
        tracing::info!(method = %req.method, "refused: a rep only relays");
        return Err(DomainError::Conflict { what: "rep_messenger" });
    }
    if let (Some(claimed), Some(rep)) = (claimed_actor(req), caller_rep)
        && claimed != format!("agent:{rep}")
    {
        return Err(DomainError::InvalidArgument { what: "actor" });
    }
    // Only a person opts in to a post under the owner's name (ov-580): an
    // agent pane, whoever it speaks as, is refused. Turning it off is anyone's.
    if turns_nudging_on(req) && caller_pid.and_then(|pid| agent_pane_of(svc, pid)).is_some() {
        tracing::info!("refused: an agent can't turn nudging on");
        return Err(DomainError::Conflict { what: "person_only" });
    }
    let Some(pane) = acted_on(req) else { return Ok(()) };
    // A rep acts on its own pane only: it never types into, answers in, or
    // stops another pane, the orchestrator's included, as the owner it would
    // otherwise pass for.
    if caller_rep.is_some_and(|rep| rep != pane) {
        return Err(DomainError::NotYours { what: "not_yours" });
    }
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

/// What a process in a rep's pane may call (R-66, ov-527): the board's
/// reads, `message.send`, and a comment on a card. An allow-list, so a method
/// added later is refused a rep until it's named here.
///
/// The reads are the ones a `read`-scoped device gets, less
/// `client.set_node_key`, which writes, and the pages, which the board's
/// layers never name (ov-269) and a rep's skill never reads. Not the
/// screens, files and diffs that sit at `control` (`terminal.screen`,
/// `worktree.read_file`, `changes.file_diff` and the like): a rep carries
/// words, and reads code with Claude Code's Read and Grep, inside its
/// checkout, which its gate allows (`rep_gate`). A note is a comment only; `refuse_rep_answer`
/// still turns away an answer or a decision written any other way.
pub fn rep_may(req: &Request) -> bool {
    use farcooler_protocol::method::Method as M;
    let Some(method) = M::parse(&req.method) else { return false };
    match method {
        M::TaskNote => matches!(
            req.payload.as_ref(),
            Some(request::Payload::TaskNoteAppend(p)) if p.kind == farcooler_protocol::v1::TaskNoteKind::Comment as i32
        ),
        M::MessageSend => true,
        _ => matches!(
            method,
            M::HostGet
                | M::HostHealth
                | M::DaemonVersion
                | M::ThemeList
                | M::RepositoryList
                | M::WorktreeList
                | M::TerminalList
                | M::BranchList
                | M::LayoutList
                | M::ChangesInbox
                | M::StackGet
                | M::NeedsYouList
                | M::ReportGet
                | M::UsageReport
                | M::UsageTask
                | M::TaskList
                | M::TaskGet
                | M::TaskGetByKey
                | M::TaskSearch
                | M::PlanGet
                | M::PlanEvents
                | M::WorkspaceList
                | M::WorktreeGrep
                | M::ClientList
                | M::MemberList
        ),
    }
}

/// The rep pane process `pid` runs in, if any: `pid` or one of its ancestors
/// is the pane's own process (`ScreenStamp::pid`). Read off the process
/// table, so a rep can't name itself out of it.
///
/// It fails open, which is right for a guardrail and says what it isn't: a
/// process that double-forks or `setsid`s away from the pane, one started by
/// `tmux new-window` or `run-shell` from inside it (a child of the tmux
/// server, not of the pane, and without `FARCOOLER_REP`), a pane younger than
/// the inventory's last refresh, and a system whose process table this
/// process can't read (no `ps` fallback here) all read as nobody's pane.
pub fn rep_pane_of(svc: &crate::service::Service, pid: i32) -> Option<Uuid> {
    pane_running(svc, pid, |svc, terminal| svc.store.rep_of_terminal(terminal).ok().flatten().is_some())
}

/// The agent or orchestrator pane `pid` runs in, if any (ov-580): the same
/// walk as `rep_pane_of`, over every pane that runs an agent rather than a
/// shell, with the same failing open. A person's own shell pane is not one.
pub fn agent_pane_of(svc: &crate::service::Service, pid: i32) -> Option<Uuid> {
    pane_running(svc, pid, |svc, terminal| {
        svc.store.get_terminal(terminal).is_ok_and(|t| t.role != TerminalRole::Shell)
    })
}

fn pane_running(svc: &crate::service::Service, pid: i32, wanted: impl Fn(&crate::service::Service, Uuid) -> bool) -> Option<Uuid> {
    let panes: std::collections::HashMap<i32, Uuid> = svc
        .inventory_snapshot()
        .panes
        .iter()
        .filter(|p| wanted(svc, p.terminal_id))
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

/// Refuse a rep's comment that reaches past its own (ov-527), as
/// `rep_messenger`: one on a card outside its workspace, one carrying a
/// `message` in its extra (which reads as a queued message's record), or
/// one superseding a note it didn't write. Anyone else's note passes.
pub fn refuse_rep_reach(
    store: &Store,
    actor: Actor,
    task: Uuid,
    extra: &serde_json::Value,
    supersedes: Option<Uuid>,
) -> Result<()> {
    let Actor::Agent { terminal } = actor else { return Ok(()) };
    let Some(rep) = store.rep_of_terminal(terminal)? else { return Ok(()) };
    let refused = DomainError::Conflict { what: "rep_messenger" };
    if store.get_task(task)?.workspace_id != rep.workspace || extra.get("message").is_some() {
        return Err(refused);
    }
    if let Some(note) = supersedes
        && !store.notes_for(task, None)?.iter().any(|n| n.id == note && n.actor == actor)
    {
        return Err(refused);
    }
    Ok(())
}

/// The Claude account this runner's user is signed in to, which every
/// Claude Code pane on it uses (ov-479 D2): the email in Claude Code's own
/// `.claude.json`, under `$CLAUDE_CONFIG_DIR` when that's set, else in the
/// home directory. Read only. `None` when it can't be read.
pub fn claude_account(home: Option<&Path>, config_dir: Option<&Path>) -> Option<String> {
    account_in(&claude_config(home, config_dir)?)
}

/// Where Claude Code keeps `.claude.json` for these directories.
fn claude_config(home: Option<&Path>, config_dir: Option<&Path>) -> Option<std::path::PathBuf> {
    match config_dir {
        Some(dir) => Some(dir.join(".claude.json")),
        None => home.map(|h| h.join(".claude.json")),
    }
}

/// The account's email in one `.claude.json`, or `None`.
pub fn account_in(file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let email = json.pointer("/oauthAccount/emailAddress")?.as_str()?.trim();
    (!email.is_empty() && email.len() <= 254 && !email.chars().any(char::is_control)).then(|| email.to_string())
}

/// This daemon's own user's `.claude.json`, which `Service::open` reads the
/// account from (a test's service reads one under its own root, ov-508).
/// An empty path, which reads as no account, when there's no home.
pub fn runner_claude_config() -> std::path::PathBuf {
    let home = directories::UserDirs::new().map(|d| d.home_dir().to_path_buf());
    let config = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    claude_config(home.as_deref(), config.as_deref()).unwrap_or_default()
}

#[cfg(test)]
#[path = "people_tests.rs"]
mod tests;
