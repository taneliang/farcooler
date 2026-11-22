//! Caller classifications and the request vocabulary a rep is allowed.
//!
//! The daemon still resolves the authenticated caller and enforces the
//! request, resource and workspace checks.

use farcooler_protocol::v1::{Request, request};
use uuid::Uuid;

/// The caller's identity on the runner.
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

/// Where a local call came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seat {
    /// A person: in no pane, or in their own shell pane with no agent above.
    Person,
    /// An agent, in this pane when it's one of the runner's.
    Agent(Option<Uuid>),
    /// Not known: never taken for a person.
    Unreadable,
}

/// What the gate says about one tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Refused, with the reason the model reads.
    Deny(&'static str),
}

/// The file a rep's gate reads its member's connectors from, on every call
/// (`rep-gate --connectors`): written by the runner at each launch and each
/// change (`Service::write_rep_connectors`), so one turned off is refused at
/// once.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorsFile {
    pub connectors: Vec<String>,
}

/// The most of that file the gate reads; past it, it refuses every MCP tool.
pub const CONNECTORS_FILE_MAX: u64 = 64 * 1024;

/// What a process in a rep's pane may call (R-66, ov-527): the board's
/// reads, `message.send`, a comment on a card, and, for its member, the plan
/// changes its gate's grammar reaches (`rep_plan_change`, ov-736: a rep may
/// change the plan on its member's behalf, filed under the member by
/// `stamp_rep`). An allow-list, so a method added later is refused a rep
/// until it's named here.
///
/// The reads are the ones a `read`-scoped device gets, less
/// `client.set_node_key`, which writes. Screens, files and diffs sit at
/// `control` (`terminal.screen`,
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
        M::LineOrder
        | M::BoardThemeCreate
        | M::BoardThemeUpdate
        | M::BoardThemeCards
        | M::BoardThemeMove
        | M::LineSet
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
                | M::LandingHistory
                | M::WorkspaceList
                | M::WorkspaceCheckInShow
                | M::WorkspaceCharter
                | M::WorktreeGrep
                | M::ClientList
                | M::MemberList
        ),
    }
}

/// Whether `req`, a plan write, is one a rep may make for its member: what
/// its gate's grammar reaches (`rep_gate::PLAN_VERBS`), and no more
/// (ov-736). The ranking, never emptied; a theme made, its words, state and
/// cards, and its place in the ranking (`plan theme move`, ov-755), never its
/// budget; a ticket's reason, and its stage only to waiting or dropped, never
/// its commit, checkout, branch, stack, pull request or budget; a ruling
/// recorded, or reversed with no commit. Never a ruling kept, which is its
/// person's own mark, a landing, or an agent: those start work or are
/// the orchestrator's record.
fn rep_plan_change(method: farcooler_protocol::method::Method, req: &Request) -> bool {
    use farcooler_protocol::method::Method as M;
    use farcooler_protocol::v1::{BoardRulingState, TicketStage};
    use request::Payload as P;
    match (method, req.payload.as_ref()) {
        (M::LineOrder, Some(P::LineOrder(p))) => !p.task_ids.is_empty(),
        (M::BoardThemeCreate, Some(P::BoardThemeCreate(_))) => true,
        (M::BoardThemeUpdate, Some(P::BoardThemeUpdate(p))) => {
            let farcooler_protocol::v1::BoardThemeUpdate {
                theme_id: _,
                name: _,
                outcome: _,
                story: _,
                next: _,
                owner_ask: _,
                state: _,
                ordinal,
                actor: _,
                budget_tokens,
            } = p;
            // Its place is `board_theme.move`'s, which records who moved it.
            ordinal.is_none() && budget_tokens.is_none()
        }
        (M::BoardThemeMove, Some(P::BoardThemeMove(_))) => true,
        (M::BoardThemeCards, Some(P::BoardThemeCards(_))) => true,
        (M::LineSet, Some(P::LineSet(p))) => {
            let farcooler_protocol::v1::TicketLineSet {
                task_id: _,
                stage,
                reason: _,
                branch,
                worktree_id,
                worktree_path,
                stack_parent,
                pr_number,
                landed_sha,
                budget_tokens,
                actor: _,
            } = p;
            stage.is_none_or(|s| s == TicketStage::Waiting as i32 || s == TicketStage::Dropped as i32)
                && branch.is_none()
                && worktree_id.is_none()
                && worktree_path.is_none()
                && stack_parent.is_none()
                && pr_number.is_none()
                && landed_sha.is_none()
                && budget_tokens.is_none()
        }
        (M::RulingAdd, Some(P::RulingAdd(_))) => true,
        (M::RulingSet, Some(P::RulingSet(p))) => p.state == BoardRulingState::Reversed as i32 && p.sha.is_none(),
        _ => false,
    }
}

/// The role every Maintainer holds as a decider.
pub const MAINTAINER: &str = "role:maintainer";

/// Whether `name` is one a gate may name, by its shape alone.
pub fn is_decider_name(name: &str) -> bool {
    name == "owner" || name == MAINTAINER
        || name.strip_prefix("member:").is_some_and(farcooler_core::validate::valid_handle)
}
