//! Who a caller is on a shared runner, and the rules that follow from it
//! (ov-494, phase 1 of `reports/ov-479/design.md`, with the owner's decisions
//! on ov-479 and R-49).
//!
//! **A person is worked out from the connection, never from the request.**
//! sshd's forced command names the device (`Peer::client_id`), and a runner
//! admin binds devices to members (`member.set`). The local socket, and any
//! device never bound to anybody, is the runner's owner: the ceremony still
//! enrolls only the owner's own devices (`ceremony::accept_offer`). A device a
//! member had and doesn't now is nobody's (`Person::Released`), and so is a
//! device a member's device enrolled, until it's bound (ov-719).
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
//! rep's pane gets only reads, messages, comments and, for its member,
//! changes to the plan (`rep_may`, R-66, ov-736): everything else is refused
//! `rep_messenger` too.
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

#[path = "people_rulings.rs"]
mod rulings;
pub use rulings::{Ruling, Why, member_ruling};

/// Who a caller is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Person {
    /// The runner's owner: the local socket, or a device never bound to
    /// anybody and never released, which no member's device enrolled (ov-719).
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

/// Whether `person` may start `handle`'s rep (`rep.start`), and so wake it
/// with a message: the owner starts anyone's, a member only their own,
/// runner admin or not (ov-716 rule 3). A rep runs on its member's login
/// and speaks for them, so starting it is theirs to choose.
pub fn may_start_rep(person: &Person, handle: &str) -> bool {
    *person == Person::Owner || matches!(person, Person::Member(me) if me == handle)
}

/// Whether `person` may read or set `handle`'s rep connectors
/// (`member.connectors`, ov-586): the member themself, or the owner. Not a
/// released device, and not another member, runner admin or not: the list
/// says which of the member's own accounts their rep acts in.
pub fn may_set_connectors(person: &Person, handle: &str) -> bool {
    *person == Person::Owner || matches!(person, Person::Member(me) if me == handle)
}

/// Who may sign `handle` in to Claude, finish or cancel it, or sign them out
/// (ov-584): the member themself (`not_yours` for the owner, another member,
/// a runner admin, a released device), from a caller in no pane
/// (`own_device_only`): a process in any pane, whatever device its
/// connection names, is refused, since a session preamble's device id isn't
/// a secret. The walk fails open as `pane_of` does; a guardrail.
///
/// And from a caller who is a person (`caller_seat`, ov-620): an agent above
/// it, in an adopted shell pane or a terminal outside the runner's panes,
/// or a caller whose chain can't be read, is refused `person_only`, whatever
/// device it names.
pub async fn may_sign_in(svc: &crate::service::Service, peer: &Peer, caller_pid: Option<i32>, handle: &str) -> Result<()> {
    match person_of(&svc.store, peer)? {
        Person::Member(me) if me == handle => {}
        _ => return Err(DomainError::NotYours { what: "not_yours" }),
    }
    if caller_pid.and_then(|pid| pane_of(svc, pid)).is_some() {
        tracing::info!("refused: a Claude sign-in from inside a pane");
        return Err(DomainError::Conflict { what: "own_device_only" });
    }
    if crate::caller_seat::caller_seat(svc, caller_pid).await != crate::caller_seat::Seat::Person {
        tracing::info!("refused: a Claude sign-in from a caller who isn't a person");
        return Err(DomainError::Conflict { what: "person_only" });
    }
    Ok(())
}

/// Whether `req` changes a member's rep connectors, which only a person does.
fn sets_connectors(req: &Request) -> bool {
    matches!(req.payload.as_ref(), Some(request::Payload::MemberConnectors(p)) if p.set)
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

/// The `actor` field of a payload that has one, borrowed as `$m` says: one
/// list for `wire_actor` and `wire_actor_mut`, so they can't disagree.
macro_rules! actor_field {
    ($payload:expr, $($m:tt)*) => {{
        use request::Payload as P;
        Some(match $payload {
            P::AgentAnswer(p) => &$($m)* p.actor,
            P::TaskCreate(p) => &$($m)* p.actor,
            P::TaskUpdate(p) => &$($m)* p.actor,
            P::TaskSetStatus(p) => &$($m)* p.actor,
            P::TaskNoteAppend(p) => &$($m)* p.actor,
            P::TaskBlockSet(p) => &$($m)* p.actor,
            P::TaskSetWait(p) => &$($m)* p.actor,
            P::TaskSetLine(p) => &$($m)* p.actor,
            P::TaskWorkerSet(p) => &$($m)* p.actor,
            P::TaskMove(p) => &$($m)* p.actor,
            P::BoardThemeCreate(p) => &$($m)* p.actor,
            P::BoardThemeUpdate(p) => &$($m)* p.actor,
            P::BoardThemeCards(p) => &$($m)* p.actor,
            P::LaneCreate(p) => &$($m)* p.actor,
            P::LaneUpdate(p) => &$($m)* p.actor,
            P::LaneCards(p) => &$($m)* p.actor,
            P::LaneAgentSet(p) => &$($m)* p.actor,
            P::PlanSet(p) => &$($m)* p.actor,
            P::RulingAdd(p) => &$($m)* p.actor,
            P::RulingSet(p) => &$($m)* p.actor,
            P::RulingKeepAll(p) => &$($m)* p.actor,
            P::MessageSend(p) => &$($m)* p.actor,
            P::TrainStart(p) => &$($m)* p.actor,
            P::TrainSet(p) => &$($m)* p.actor,
            P::PageSet(p) => &$($m)* p.actor,
            P::PageRemove(p) => &$($m)* p.actor,
            _ => return None,
        })
    }};
}

/// The actor any write says it writes as (ov-706): every request whose
/// payload carries `actor`, which a handler records and some trust for
/// authority (`ruling.set` keeps only as `user`). Kept beside the proto by
/// `people_actor_tests`, which fails on a write added with an actor and not
/// named here.
fn wire_actor(req: &Request) -> Option<&str> {
    actor_field!(req.payload.as_ref()?,).map(String::as_str)
}

/// `wire_actor`, to be written: what `stamp_member` rewrites.
fn wire_actor_mut(req: &mut Request) -> Option<&mut String> {
    actor_field!(req.payload.as_mut()?, mut)
}

/// Whether `req` switches the nudge on, which posts to GitHub as the owner.
fn turns_nudging_on(req: &Request) -> bool {
    matches!(req.payload.as_ref(), Some(request::Payload::WorkspaceSetSettings(p)) if p.pr_nudge == Some(true))
}

/// The gate every request passes before dispatch (`Rpc::handle`).
///
/// - **A rep's pane relays, and changes the plan for its member** (R-66,
///   ov-736). A local caller whose process runs in a rep's pane gets the
///   methods `rep_may` lists, and every other is refused `rep_messenger`: no
///   dispatch, no task made or changed, no worktree or pane made or acted
///   on, no member or workspace setting. A plan write from it names the rep
///   itself, never `user`, nobody, `manager` or another pane (`actor`), and
///   `stamp_rep` files it under the rep's member.
/// - **A rep's pane speaks as itself** (ov-479 D3). A local caller whose
///   process runs in a rep's pane (`rep_pane_of`, from its pid, never from
///   what it claims) may write a note, answer or message only as that pane:
///   any other actor, `user` and `manager` included, is refused `actor`. Its
///   own actor then meets `refuse_rep_answer`.
/// - **A member's device writes as its person** (ov-706). On any write
///   that names an actor (`wire_actor`), it may claim only `user`, which is
///   them: `manager` or a pane is refused `actor`. `stamp_member` then files it under
///   them. Nobody claims `member:<handle>` on the wire (ov-707). So honest and modified apps alike
///   record the right person. It doesn't sandbox a member: one with
///   Control can open a shell pane and act as the runner's Unix user, the
///   documented trust boundary (docs/runners.md).
/// - **Every method has a member ruling** (`member_ruling`, ov-716), and
///   what's the owner's alone (`Ruling::Owners`) is refused a member's
///   device and a released one, `not_yours`, whatever it names as its
///   actor: a post to GitHub with the runner's `gh` login (ov-637), the
///   runner's and a workspace's shape (ov-637: deleting, renaming or
///   re-prefixing a workspace, its charter, where repositories are found
///   and, ov-716, which are registered, the branch prefix, the projector,
///   the terminal themes, and stopping the daemon), and removing a
///   worktree, the owner's files (ov-716).
/// - **Some by the payload**: any change to a workspace's settings but
///   turning the nudge off (`changes_settings`, ov-637), letting
///   members run on the owner's logins (`lends_the_owners_login`, ov-716),
///   and enrolling a shell line (`enrolls_a_shell`, ov-719), are refused
///   the same way.
/// - **The owner's devices are the owner's** (ov-719): binding one to a
///   member, or revoking one, from a member's device is refused
///   `not_yours` (`touches_the_owners_device`). A device a member's device
///   enrolls is released until bound (`client.enroll`), so it never is one.
/// - **A released device only reads** (ov-712). A device whose member
///   was removed is nobody's: every call but a `Ruling::Reads` is refused
///   `not_yours`. Removing a member also revokes their devices
///   (`member.remove`); this holds if one is still connected or comes back.
/// - **The plan is everyone's** (`Ruling::Plan`, the owner, 24 Oct,
///   ov-736, reversing ov-637): a member's device changes the ranking, a
///   theme, a lane, a ruling, a train's name or a page, under their own
///   name. Not integration or its record (`records_the_orchestrators`,
///   `moves_a_landing_lane`): a lane landing or landed, or moved once it
///   is, its commit, train or agent, a train started or changed but for its
///   name, a reversal marked done.
/// - **Another person's lane is answered and stopped, nothing more**
///   (ov-716 rule 5, `refuse_someone_elses_lane`): a member typing into,
///   restarting, removing or recasting a lane pane on someone else's login
///   is refused `not_yours`.
/// - **Only the lead, and the owner, start the orchestrator** (ov-637):
///   `workspace.start_orchestrator`, or `terminal.set_role` making a pane
///   one (ov-716), from anyone else's device is `not_lead`.
/// - **A rep is its person's.** Acting on a rep's pane is refused
///   `not_yours` for anyone but its member and the owner.
/// - **A caller no walk can place** (`unplaced`, ov-649), when the runner has
///   a rep, gets only what a rep may, less the plan, which a rep writes only
///   as its member, and is refused `in_pane` otherwise.
/// - **Only a person turns the nudge on** (ov-580), **or changes a member's
///   rep connectors** (ov-586). A local caller whose process runs in an
///   agent pane, or that no walk can place, is refused `person_only`: a prompt a lane or rep reads must
///   not widen what a rep can reach.
/// - **Only the lead, and the owner (R-54), talk to the orchestrator.**
///   Acting on its pane is refused `not_lead` for anyone else.
///
/// Anything else passes, to be answered by its own handler: a pane that
/// isn't there is `NotFound` there.
pub fn gate(svc: &crate::service::Service, peer: &Peer, caller_pid: Option<i32>, req: &Request) -> Result<()> {
    gate_rep(svc, peer, caller_pid, req).map(drop)
}

/// `gate`, answering the rep's pane it found the caller in, if it looked
/// and found one, for `stamp_rep`: one walk decides both whether a rep may
/// and whose it is (ov-736, review M3).
pub fn gate_rep(svc: &crate::service::Service, peer: &Peer, caller_pid: Option<i32>, req: &Request) -> Result<Option<Uuid>> {
    let mut found = None;
    gate_seen(svc, peer, caller_pid, req, &mut found)?;
    Ok(found)
}

fn gate_seen(svc: &crate::service::Service, peer: &Peer, caller_pid: Option<i32>, req: &Request, found: &mut Option<Uuid>) -> Result<()> {
    let store = &svc.store;
    let speaks_for = claimed_actor(req).is_some() || acted_on(req).is_some() || writes_the_plan(req);
    let caller_rep = caller_pid
        .filter(|_| (speaks_for || !rep_may(req)) && store.any_reps().unwrap_or(true))
        .and_then(|pid| rep_pane_of(svc, pid));
    *found = caller_rep;
    if caller_rep.is_some() && !rep_may(req) {
        tracing::info!(method = %req.method, "refused: a rep only relays");
        return Err(DomainError::Conflict { what: "rep_messenger" });
    }
    // A caller no walk can place (a command left behind by a rep's pane,
    // ov-649) might be a rep's: it gets no more than a rep does.
    // Nor a plan write, which a rep makes only as its member (`stamp_rep`),
    // and nobody can say whose this would be.
    if caller_rep.is_none()
        && (!rep_may(req) || writes_the_plan(req))
        && store.any_reps().unwrap_or(true)
        && unplaced(svc, caller_pid)
    {
        tracing::info!(method = %req.method, "refused: a caller in no known pane and not shown to be a person");
        return Err(DomainError::Conflict { what: "in_pane" });
    }
    if let (Some(claimed), Some(rep)) = (claimed_actor(req), caller_rep)
        && claimed != format!("agent:{rep}")
    {
        return Err(DomainError::InvalidArgument { what: "actor" });
    }
    // A rep's plan write says it's from the rep, as its CLI does, and is
    // filed under its member (`stamp_rep`). Never nobody or `user`, which a
    // write the stamp missed would file as the owner's, nor the orchestrator.
    if let (Some(claimed), Some(rep), true) = (wire_actor(req), caller_rep, writes_the_plan(req))
        && claimed != format!("agent:{rep}")
    {
        tracing::info!(method = %req.method, "refused: a rep's plan write as someone else");
        return Err(DomainError::InvalidArgument { what: "actor" });
    }
    if let Some(claimed) = wire_actor(req) {
        // Only the runner writes a member's word (`stamp_member`, ov-707).
        if claimed.starts_with("member:") {
            tracing::info!(method = %req.method, "refused: a caller naming a member");
            return Err(DomainError::InvalidArgument { what: "actor" });
        }
        refuse_someone_elses_claim(store, peer, req, claimed)?;
    }
    // A removed member's device is nobody's, the owner included (ov-712):
    // it reads, and writes nothing. Before, its writes said `user`, which
    // reads as the owner's, and told nobody.
    if peer.client_id.is_some() && !reads_only(req) && person_of(store, peer)? == Person::Released {
        tracing::info!(method = %req.method, "refused: a released device writing");
        return Err(DomainError::NotYours { what: "not_yours" });
    }
    let owners = matches!(ruling_of(req), Some(Ruling::Owners(_)))
        || changes_settings(req)
        || lends_the_owners_login(req)
        || enrolls_a_shell(req);
    if owners && peer.client_id.is_some() && person_of(store, peer)?.is_someone_else() {
        tracing::info!(method = %req.method, "refused: a member's device changing what's the owner's");
        return Err(DomainError::NotYours { what: "not_yours" });
    }
    if peer.client_id.is_some()
        && (records_the_orchestrators(req) || moves_a_landing_lane(store, req)?)
        && person_of(store, peer)?.is_someone_else()
    {
        tracing::info!(method = %req.method, "refused: a member's device writing the orchestrator's record");
        return Err(DomainError::NotYours { what: "not_yours" });
    }
    // A rep's member is refused that from their own device, so the rep is
    // too: its `--state dropped` would take a landing lane off its train
    // (rereview N1). `rep_plan_change` holds the rest of the record.
    if caller_rep.is_some() && moves_a_landing_lane(store, req)? {
        tracing::info!(method = %req.method, "refused: a rep moving a lane that reached landing");
        return Err(DomainError::NotYours { what: "not_yours" });
    }
    if peer.client_id.is_some() && touches_the_owners_device(store, req)? && person_of(store, peer)?.is_someone_else() {
        tracing::info!(method = %req.method, "refused: a member's device binding or revoking the owner's");
        return Err(DomainError::NotYours { what: "not_yours" });
    }
    // Starting a workspace's orchestrator, or replacing it with one that
    // reads a handoff card first, or making a pane it, is its lead's and the
    // owner's, as talking to it is (R-54). A request naming no workspace is
    // its handler's to refuse.
    if peer.client_id.is_some()
        && let Some(workspace) = seats_an_orchestrator(store, req)
        && !talks_to_orchestrator(store, &person_of(store, peer)?, workspace)?
    {
        tracing::info!(device = peer.client_id.as_deref().unwrap_or("-"), "refused: only the lead starts the orchestrator");
        return Err(DomainError::NotYours { what: "not_lead" });
    }
    // Only a person opts in to a post under the owner's name (ov-580): an
    // agent pane, whoever it speaks as, is refused. Turning it off is anyone's.
    if turns_nudging_on(req) && (caller_pid.and_then(|pid| agent_pane_of(svc, pid)).is_some() || unplaced(svc, caller_pid)) {
        tracing::info!("refused: an agent can't turn nudging on");
        return Err(DomainError::Conflict { what: "person_only" });
    }
    if sets_connectors(req) && (caller_pid.and_then(|pid| agent_pane_of(svc, pid)).is_some() || unplaced(svc, caller_pid)) {
        tracing::info!("refused: an agent can't change a rep's connectors");
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
    let rep = store.rep_of_terminal(pane)?.map(|r| r.handle);
    let workspace = terminal.workspace_id.filter(|_| terminal.role == TerminalRole::Orchestrator);
    let lead = match workspace {
        Some(ws) if rep.is_none() => Some(store.workspace_lead(ws)?),
        _ => None,
    };
    let person = person_of(store, peer)?;
    if may_act_on(&person, rep.as_deref(), lead.as_ref().map(|l| l.as_deref())) {
        if rep.is_none() && workspace.is_none() {
            return refuse_someone_elses_lane(store, &person, pane, req);
        }
        return Ok(());
    }
    if rep.is_some() {
        return Err(DomainError::NotYours { what: "not_yours" });
    }
    tracing::info!(device = peer.client_id.as_deref().unwrap_or("-"), method = %req.method, "refused: only the lead talks to the orchestrator");
    Err(DomainError::NotYours { what: "not_lead" })
}

/// The ruling on `req`'s method, or `None` for a name this build doesn't
/// know (refused before the gate, by scope).
fn ruling_of(req: &Request) -> Option<Ruling> {
    farcooler_protocol::method::Method::parse(&req.method).map(member_ruling)
}

/// Whether `req` only reads (ov-712): `Ruling::Reads`, a method at Read
/// scope that writes nothing, not even the caller's own line or Claude login
/// (a node key, a re-key, a sign-in, which are `Ruling::Own`). What a
/// released device may still call.
fn reads_only(req: &Request) -> bool {
    ruling_of(req) == Some(Ruling::Reads)
}

/// Whether `req` changes the plan layer (`Ruling::Plan`): the ranking, a
/// theme, a lane, a ruling, a train or an orchestrator's page. Every
/// member's to change, under their own name, as the task layer is (the
/// owner, 24 Oct, ov-736, reversing ov-637), and a rep's for its member
/// (`stamp_rep`).
fn writes_the_plan(req: &Request) -> bool {
    ruling_of(req) == Some(Ruling::Plan)
}

/// The workspace whose orchestrator `req` starts or replaces, which only its
/// lead and the owner do (ov-637): `workspace.start_orchestrator`'s, or with
/// `terminal.set_role` to orchestrator, the pane's own (ov-716). A pane that
/// isn't there, or has no workspace, is its handler's to refuse.
fn seats_an_orchestrator(store: &Store, req: &Request) -> Option<Uuid> {
    let target = req.target_resource_id.as_deref().and_then(crate::wire::parse_id)?;
    match (req.method.as_str(), req.payload.as_ref()) {
        ("workspace.start_orchestrator", _) => Some(target),
        ("terminal.set_role", Some(request::Payload::TerminalSetRole(p)))
            if p.role == farcooler_protocol::v1::TerminalRole::Orchestrator as i32 =>
        {
            store.get_terminal(target).ok()?.workspace_id
        }
        _ => None,
    }
}

/// Whether `req` writes integration or its record, not the team's plan for
/// the work (ov-736, the owner's rulings on review M2 and fix round 2): a
/// lane moved to landing or landed, which puts it on a train and reads as
/// finished; a lane's landed commit or train; a lane's agent, made, set or
/// recorded, which files a worker on its card; a train started, or any
/// change to one but its title (its lanes, base, state, commit, agent or
/// card), which CI and the landing read; and a ruling's reversal marked done
/// with its commit. The owner's and the orchestrator's. Everything else in
/// the plan is every member's: themes, the ranking, cards, titles, reasons,
/// rulings, a train's name, and a lane queued, dropped, building, in review
/// or fixing until it reaches landing (`moves_a_landing_lane`).
fn records_the_orchestrators(req: &Request) -> bool {
    use farcooler_protocol::v1::{BoardTrainState, LaneState};
    use request::Payload as P;
    match req.payload.as_ref() {
        Some(P::LaneUpdate(p)) => {
            p.state.is_some_and(|s| s == LaneState::Landing as i32 || s == LaneState::Landed as i32)
                || p.landed_sha.is_some()
                || p.agent.is_some()
                || p.train.is_some()
        }
        Some(P::LaneCreate(p)) => p.agent.is_some(),
        Some(P::LaneAgentSet(_)) => true,
        Some(P::TrainStart(_)) => true,
        Some(P::TrainSet(p)) => {
            let farcooler_protocol::v1::TrainSet {
                train_id: _,
                state,
                base,
                sha,
                add_lane_ids,
                remove_lane_ids,
                actor: _,
                title: _,
                agent,
                card_id,
            } = p;
            *state != BoardTrainState::Unspecified as i32
                || base.is_some()
                || sha.is_some()
                || !add_lane_ids.is_empty()
                || !remove_lane_ids.is_empty()
                || agent.is_some()
                || card_id.is_some()
        }
        Some(P::RulingSet(p)) => p.sha.is_some(),
        _ => false,
    }
}

/// Whether `req` moves a lane that has reached landing or landed to another
/// state, dropping included (ov-736, fix round 2): a landing lane is on a
/// train, and taking it off can empty that train, which sends the one
/// behind it off and tells the orchestrator. Integration's, so the owner's
/// and the orchestrator's. A lane that isn't there is its handler's.
fn moves_a_landing_lane(store: &Store, req: &Request) -> Result<bool> {
    use farcooler_protocol::v1::LaneState as Wire;
    use farcooler_store::plan::LaneState;
    let Some(request::Payload::LaneUpdate(p)) = req.payload.as_ref() else { return Ok(false) };
    let (Some(to), Some(lane)) = (p.state, crate::wire::parse_id(&p.lane_id)) else { return Ok(false) };
    let now = match store.lane(lane) {
        Ok(lane) => lane.state,
        Err(DomainError::NotFound) => return Ok(false),
        Err(e) => return Err(e),
    };
    Ok(match now {
        LaneState::Landing => to != Wire::Landing as i32,
        LaneState::Landed => to != Wire::Landed as i32,
        _ => false,
    })
}

/// Whether `req` lets members run on the runner's own Claude or Codex
/// login (`member.set_own_login` off, ov-652): the owner's account, lent,
/// which a member admin can't do (ov-716). Requiring members' own is
/// anyone's with the scope, as turning the nudge off is.
fn lends_the_owners_login(req: &Request) -> bool {
    matches!(req.payload.as_ref(), Some(request::Payload::MemberOwnLoginSet(p)) if !p.on)
}

/// Whether `req` enrolls a plain line (`client.enroll` with `shell_access`,
/// a Mac's Key B): a shell on the runner's account with no forced command,
/// so nothing the gate can hold, and one that outlives a member's removal.
/// The owner's alone (ov-719); a teammate is never given one.
fn enrolls_a_shell(req: &Request) -> bool {
    matches!(req.payload.as_ref(), Some(request::Payload::ClientEnroll(p)) if p.shell_access)
}

/// Whether `req` binds (`member.set`) or revokes (`client.revoke`) a device
/// that is the owner's (ov-719): one `person_of` reads as `Person::Owner`,
/// bound to no member and not released. A device a member's device enrolled
/// is released until bound (`Store::record_member_enrolled`), so it's never
/// one of these. Bound, the owner's phone would be a member's; revoked, the
/// owner is locked out of their own runner.
fn touches_the_owners_device(store: &Store, req: &Request) -> Result<bool> {
    let ids: Vec<&str> = match req.payload.as_ref() {
        Some(request::Payload::MemberSet(p)) => p.client_ids.iter().map(|d| d.trim()).collect(),
        Some(request::Payload::ClientRevoke(p)) => vec![p.client_id.as_str()],
        _ => return Ok(false),
    };
    for id in ids.into_iter().filter(|d| !d.is_empty()) {
        if store.device_is_owners(id)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether `peer`'s `stack.get` may start the `gh` fill that read does when
/// the cache is empty (`review_ops::fill_prs_in_background`, ov-716). The
/// owner's and a member's may: members read everything, and the fill only
/// reads, once a minute a repository at most. A released device reads
/// what's cached and spends nothing of the owner's. A store that can't be
/// read starts none.
pub fn may_fill_prs(store: &Store, peer: &Peer) -> bool {
    peer.client_id.is_none() || person_of(store, peer).is_ok_and(|p| p != Person::Released)
}

/// Whether `req` changes a workspace's settings (ov-637): how it lands
/// (`DIRECT` has the owner's orchestrator push to the base with the owner's
/// credentials), onto what, its pull requests' size and cost line, the nudge
/// and how soon it posts, waking on an answer, or its lead. All the owner's.
/// Turning the nudge off alone isn't: anyone may stop a post in the owner's
/// name (ov-580). Every field is named, with no `..`, so a setting added to
/// the message doesn't compile here until it has a ruling (ov-716).
fn changes_settings(req: &Request) -> bool {
    let Some(request::Payload::WorkspaceSetSettings(p)) = req.payload.as_ref() else { return false };
    let farcooler_protocol::v1::WorkspaceSetSettings {
        wake_on_answer,
        expected_version: _,
        landing,
        base,
        pr_max_lines,
        pr_cost_line,
        lead,
        pr_nudge,
        pr_nudge_hours,
    } = p;
    wake_on_answer.is_some()
        || landing.is_some()
        || base.is_some()
        || pr_max_lines.is_some()
        || pr_cost_line.is_some()
        || lead.is_some()
        || *pr_nudge == Some(true)
        || pr_nudge_hours.is_some()
}

/// Refuse a write from a member's device, or a released one, that names
/// anyone but the person holding it (ov-706). `user`, or no actor, is that
/// person, as `message.send` reads it; `manager` or `agent:<id>` would file
/// the write under the orchestrator or a pane, and is refused `actor`. The
/// local socket and a device bound to nobody are the owner, and are not
/// asked.
fn refuse_someone_elses_claim(store: &Store, peer: &Peer, req: &Request, claimed: &str) -> Result<()> {
    if peer.client_id.is_none() || !person_of(store, peer)?.is_someone_else() {
        return Ok(());
    }
    if !(claimed.is_empty() || claimed == "user") {
        tracing::info!(method = %req.method, "refused: a member's device writing as someone else");
        return Err(DomainError::InvalidArgument { what: "actor" });
    }
    Ok(())
}

/// File a member's write under them (ov-707): on a member's device, a
/// write that names an actor (`wire_actor`) and says `user` or nothing is
/// rewritten to `member:<handle>` before its handler reads it, so the board
/// says who did it. Run after `gate`, which has refused any other claim
/// from them. The owner, the local socket and a released device are left as
/// they are, and so is `terminal.agent_answer`, which records nobody.
pub fn stamp_member(store: &Store, peer: &Peer, mut req: Request) -> Result<Request> {
    if peer.client_id.is_none() || matches!(req.payload, Some(request::Payload::AgentAnswer(_))) {
        return Ok(req);
    }
    let Person::Member(handle) = person_of(store, peer)? else { return Ok(req) };
    if let Some(actor) = wire_actor_mut(&mut req)
        && (actor.is_empty() || actor == "user")
    {
        *actor = format!("member:{handle}");
    }
    Ok(req)
}

/// File a rep's plan write under its member (ov-736): a plan write
/// (`writes_the_plan`) from `rep`, the rep's pane `gate_rep` found the caller
/// in, is rewritten to `member:<handle>` before its handler reads it, so the
/// plan says whose word it was. The gate has refused it any actor but the
/// rep's own, so a rep unbound since is left as itself, never the owner.
/// Every other request, and a caller in no rep's pane, is left as it is.
pub fn stamp_rep(store: &Store, rep: Option<Uuid>, mut req: Request) -> Result<Request> {
    let Some(pane) = rep.filter(|_| writes_the_plan(&req)) else { return Ok(req) };
    let Some(rep) = store.rep_of_terminal(pane)? else { return Ok(req) };
    if let Some(actor) = wire_actor_mut(&mut req) {
        *actor = format!("member:{}", rep.handle);
    }
    Ok(req)
}

/// `gate`'s rule for acting on one pane, from its facts: a rep (`rep`, its
/// member's handle) is its member's and the owner's; an orchestrator
/// (`orchestrator_lead`, its lead, `None` while the owner leads) is its
/// lead's and the owner's (R-54); any other pane is anyone's. Needs You asks
/// it too, so a row never offers an answer the gate would refuse (ov-633).
pub fn may_act_on(person: &Person, rep: Option<&str>, orchestrator_lead: Option<Option<&str>>) -> bool {
    if *person == Person::Owner {
        return true;
    }
    let me = match person {
        Person::Member(handle) => Some(handle.as_str()),
        _ => None,
    };
    if let Some(rep) = rep {
        return me == Some(rep);
    }
    match orchestrator_lead {
        Some(lead) => lead.is_some() && me == lead,
        None => true,
    }
}

/// Refuse a member acting on another person's lane pane, other than to
/// answer its prompt or stop it (ov-716 rule 5): typing into it, pasting,
/// drafting, sending what's queued, taking or withdrawing its draft,
/// restarting, removing or recasting it, `not_yours`. The lane is whose
/// login it runs on (`Store::terminal_login`): a member's handle, or with
/// none, the owner's. A member's own panes are theirs, and the owner acts
/// on any. Run on a pane that's neither a rep nor an orchestrator, which
/// have their own rules.
fn refuse_someone_elses_lane(store: &Store, person: &Person, pane: Uuid, req: &Request) -> Result<()> {
    let Person::Member(me) = person else { return Ok(()) };
    if matches!(req.method.as_str(), "terminal.agent_answer" | "terminal.interrupt" | "terminal.stop") {
        return Ok(());
    }
    if store.terminal_login(pane)?.is_some_and(|login| login.handle == *me) {
        return Ok(());
    }
    tracing::info!(method = %req.method, "refused: a member acting on someone else's lane");
    Err(DomainError::NotYours { what: "not_yours" })
}

/// What a process in a rep's pane may call (R-66, ov-527): the board's
/// reads, `message.send`, a comment on a card, and, for its member, the plan
/// changes its gate's grammar reaches (`rep_plan_change`, ov-736: a rep may
/// change the plan on its member's behalf, filed under the member by
/// `stamp_rep`). An allow-list, so a method added later is refused a rep
/// until it's named here.
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
        M::PlanSet
        | M::BoardThemeCreate
        | M::BoardThemeUpdate
        | M::BoardThemeCards
        | M::LaneUpdate
        | M::LaneCards
        | M::RulingAdd
        | M::RulingSet => rep_plan_change(method, req),
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

/// Whether `req`, a plan write, is one a rep may make for its member: what
/// its gate's grammar reaches (`rep_gate::PLAN_VERBS`), and no more
/// (ov-736). The ranking, never emptied; a theme made, its words, state and cards, never
/// its budget or place; a lane's title, reason and cards, and its state only
/// to queued or dropped, never its train, commit, checkout, branch, agent or
/// budget; a ruling recorded, or reversed with no commit. Never a ruling
/// kept, which is its person's own mark, a train, a page, a lane made, or a
/// lane's agent: those start work or are the orchestrator's record.
fn rep_plan_change(method: farcooler_protocol::method::Method, req: &Request) -> bool {
    use farcooler_protocol::method::Method as M;
    use farcooler_protocol::v1::{BoardRulingState, LaneState};
    use request::Payload as P;
    match (method, req.payload.as_ref()) {
        (M::PlanSet, Some(P::PlanSet(p))) => !p.lane_ids.is_empty(),
        (M::BoardThemeCreate, Some(P::BoardThemeCreate(_))) => true,
        (M::BoardThemeUpdate, Some(P::BoardThemeUpdate(p))) => p.budget_tokens.is_none() && p.ordinal.is_none(),
        (M::BoardThemeCards, Some(P::BoardThemeCards(_))) => true,
        (M::LaneUpdate, Some(P::LaneUpdate(p))) => {
            let farcooler_protocol::v1::LaneUpdate {
                lane_id: _,
                state,
                reason: _,
                train,
                landed_sha,
                worktree_path,
                branch,
                worktree_id,
                agent,
                actor: _,
                budget_tokens,
                title: _,
            } = p;
            state.is_none_or(|s| s == LaneState::Queued as i32 || s == LaneState::Dropped as i32)
                && train.is_none()
                && landed_sha.is_none()
                && worktree_path.is_none()
                && branch.is_none()
                && worktree_id.is_none()
                && agent.is_none()
                && budget_tokens.is_none()
        }
        (M::LaneCards, Some(P::LaneCards(_))) => true,
        (M::RulingAdd, Some(P::RulingAdd(_))) => true,
        (M::RulingSet, Some(P::RulingSet(p))) => p.state == BoardRulingState::Reversed as i32 && p.sha.is_none(),
        _ => false,
    }
}

/// Whether a local caller is in no pane the walks can name and nothing shows
/// a person behind it (`caller_seat::walk_seat`, ov-649): a command an agent
/// left behind, orphaned to pid 1 with no terminal, or a socket caller whose
/// pid wasn't said. The pane walks below read it as in no pane, so a check
/// that trusts "no pane" asks this too. `None`, the in-process stdio
/// session, is placed.
pub fn unplaced(svc: &crate::service::Service, caller_pid: Option<i32>) -> bool {
    caller_pid.is_some_and(|pid| crate::caller_seat::walk_seat(svc, pid) == crate::caller_seat::Seat::Unreadable)
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

/// The pane `pid` runs in, whatever it runs (ov-583): the same walk, failing
/// open the same way, for whose Claude login a pane it opens runs on
/// (`claude_logins`).
pub fn pane_of(svc: &crate::service::Service, pid: i32) -> Option<Uuid> {
    pane_running(svc, pid, |_, _| true)
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

#[cfg(test)]
#[path = "people_actor_tests.rs"]
mod actor_tests;

#[cfg(test)]
#[path = "people_plan_tests.rs"]
mod plan_tests;

#[cfg(test)]
#[path = "people_settings_tests.rs"]
mod settings_tests;

#[cfg(test)]
#[path = "people_released_tests.rs"]
mod released_tests;

#[cfg(test)]
#[path = "people_rulings_tests.rs"]
mod rulings_tests;

#[cfg(test)]
#[path = "people_enrolled_tests.rs"]
mod enrolled_tests;

#[cfg(test)]
#[path = "people_team_tests.rs"]
mod team_tests;

#[cfg(test)]
#[path = "people_rep_plan_tests.rs"]
mod rep_plan_tests;
