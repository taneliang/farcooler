//! What a device that isn't the owner's may do with each method (ov-716):
//! one ruling per row of the protocol's method table, by a match with no
//! wildcard, so a method added later doesn't compile until someone rules on
//! it. `people::gate` enforces these; `reports/ov-716/matrix.md` gives each
//! one's reasons and evidence.

use farcooler_protocol::method::Method as M;

/// A member's device's say over one method, and a released device's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ruling {
    /// A read at Read scope. Members read everything (ov-637), and a
    /// released device still reads (ov-712): the one ruling it's allowed.
    Reads,
    /// The caller's own line, login, rep or connectors only, checked by the
    /// handler (`may_sign_in`, `may_start_rep`, `may_set_connectors`, the
    /// connection's own `authorized_keys` line).
    Own,
    /// A member's to call, written under their name (ov-707), within the
    /// gate's rules for panes: a rep is its member's, an orchestrator its
    /// lead's, any other pane anyone's (ov-633).
    Members,
    /// Allowed today, pending the owner's ruling (the matrix's open
    /// questions): a runner admin's power over people and devices, and the
    /// read state and attention every device shares.
    Undecided,
    /// Decided by what the payload asks (`people::gate`): a workspace
    /// setting, a pane made the orchestrator, members' logins lent, a shell
    /// line enrolled, the owner's device bound or revoked (ov-719).
    ByPayload,
    /// The workspace's lead's and the owner's (R-54, ov-637).
    Lead,
    /// The owner's alone, refused `not_yours`, for the reason given.
    Owners(Why),
}

/// Why a method is the owner's alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// It changes the plan layer, which members read and don't change (ov-637).
    Plan,
    /// It posts to GitHub with the runner's `gh` login, the owner's (ov-637).
    PostsAsTheOwner,
    /// It deletes or reshapes a workspace, or sets up the runner itself
    /// (ov-637, ov-716).
    Reshapes,
    /// It deletes the owner's files (ov-716).
    Destroys,
}

/// The ruling on `method`. See `Ruling`.
pub fn member_ruling(method: M) -> Ruling {
    use Ruling::*;
    match method {
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
        // Its `gh` fill is a member's, not a released device's (`may_fill_prs`).
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
        | M::PageList
        | M::PageGet
        | M::PageStats
        | M::WorkspaceList
        | M::ClientList
        | M::MemberList => Reads,

        M::ClientSetNodeKey
        | M::ClientRekey
        | M::ClaudeSignIn
        | M::ClaudeSignInFinish
        | M::ClaudeSignInCancel
        | M::ClaudeSignOut
        // A runner admin starts anyone's (`may_start_rep`): an open question.
        | M::RepStart
        | M::MemberConnectors => Own,

        // Reads above Read scope: paths, screens, files and diffs.
        M::RepositoryRootList
        | M::WorktreeDiscover
        | M::WorktreeFileSearch
        | M::WorktreeListDir
        | M::WorktreeReadFile
        | M::WorktreeGrep
        | M::TerminalScreen
        | M::TerminalAttach
        | M::TerminalAgentSubscribe
        | M::AgentRows
        | M::AgentRowsFollow
        | M::AgentImage
        | M::ChangesChangeSet
        | M::ChangesCommitFiles
        | M::ChangesFileDiff
        // Reads GitHub on the owner's `gh`, and writes nothing there.
        | M::PrRefresh
        | M::RepositoryLanding
        | M::RepositorySurvey
        // Lanes and panes: a member's own, under their own login.
        | M::WorktreeCreate
        | M::WorktreeHide
        | M::WorktreeUnhide
        | M::WorktreeReorder
        | M::WorktreeHydrateLfs
        | M::WorktreeAssign
        | M::TerminalCreate
        | M::TerminalResize
        | M::TerminalRename
        | M::TerminalDismissLost
        // Acting on a pane: `gate`'s rules for reps and the orchestrator.
        | M::TerminalWrite
        | M::TerminalPasteFile
        | M::TerminalStop
        | M::TerminalRestart
        | M::TerminalRemove
        | M::TerminalDraftPrompt
        | M::TerminalTell
        | M::TerminalDraftWithdraw
        | M::TerminalCompose
        | M::TerminalInterrupt
        | M::TerminalSendNow
        | M::TerminalBringDraft
        | M::TerminalAgentAnswer
        | M::MessageSend
        // Shared records of review and tiling, which change no one's work.
        | M::ChangesSetBase
        | M::StackSetParent
        | M::LayoutSplit
        | M::LayoutMove
        | M::LayoutResize
        | M::LayoutBreak
        | M::LayoutRename
        | M::LayoutViewport
        | M::LayoutPreset
        | M::LayoutCycle
        | M::LayoutFocus
        | M::LayoutZoom
        | M::LayoutSwap
        | M::LayoutGroupSelect
        // The task layer, theirs to make (ov-637).
        | M::TaskCreate
        | M::TaskUpdate
        | M::TaskSetStatus
        | M::TaskNote
        | M::TaskBlock
        | M::TaskSetWait
        | M::TaskSetLine
        | M::TaskWorker
        | M::TaskMove => Members,

        M::MemberRemove
        | M::WorkspaceCreate
        | M::WorkspaceMarkRead
        | M::ChangesMarkRead
        | M::TerminalSeen
        | M::TerminalWatching => Undecided,

        M::WorkspaceSetSettings | M::TerminalSetRole | M::MemberSetOwnLogin => ByPayload,
        // A runner admin's, but no shell line, and not the owner's devices;
        // a device enrolled is nobody's until bound (ov-719).
        M::ClientEnroll | M::ClientRevoke | M::MemberSet => ByPayload,

        M::WorkspaceStartOrchestrator => Lead,

        M::PlanSet
        | M::BoardThemeCreate
        | M::BoardThemeUpdate
        | M::BoardThemeCards
        | M::LaneCreate
        | M::LaneUpdate
        | M::LaneCards
        | M::LaneAgent
        | M::RulingAdd
        | M::RulingSet
        | M::RulingKeepAll
        | M::TrainStart
        | M::TrainSet
        | M::PageSet
        | M::PageRemove => Owners(Why::Plan),

        M::PrReady | M::PrThreadReply => Owners(Why::PostsAsTheOwner),

        M::WorkspaceDelete
        | M::WorkspaceRename
        | M::WorkspaceSetPrefix
        | M::WorkspaceWriteCharter
        | M::RepositoryRootAdd
        | M::RepositoryRootRemove
        | M::RepositoryRegister
        | M::SettingsSetBranchPrefix
        | M::SettingsSetProjector
        | M::ThemeUpsert
        | M::ThemeDelete
        | M::DaemonShutdown => Owners(Why::Reshapes),

        M::WorktreeRemove => Owners(Why::Destroys),
    }
}
