//! What a device that isn't the owner's may do with each method (ov-716):
//! one ruling per row of the protocol's method table, by a match with no
//! wildcard, so a method added later doesn't compile until someone rules on
//! it. `people::gate` enforces these; `reports/ov-716/matrix.md` gives each
//! one's reasons and evidence.

use farcooler_protocol::method::Method as M;

/// A member's device's say over one method, and a released device's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ruling {
    /// A read at Read scope. Members read everything (ov-637). A released
    /// device reads nothing (the owner, 25 Oct, ov-758): every call from
    /// one is refused, this ruling's included.
    Reads,
    /// The caller's own line, login, rep or connectors only, checked by the
    /// handler (`may_sign_in`, `may_start_rep`, `may_set_connectors`, the
    /// connection's own `authorized_keys` line).
    Own,
    /// A member's to call, written under their name (ov-707), within the
    /// gate's rules for panes: a rep is its member's, an orchestrator its
    /// lead's, and another person's lane only answered and stopped (ov-716
    /// rule 5, `refuse_someone_elses_lane`), while a pane of their own is
    /// theirs (ov-633).
    Members,
    /// A person's own read state and attention: what's read, reviewed, seen
    /// and watched. Every member's to call, and kept per person (ov-716 rule
    /// 1, ov-740, `people::reader_of`): a member's mark is theirs and leaves
    /// the owner's alone, and the owner's leaves theirs: the owner's look at
    /// a member's failed turn leaves it on their Needs You, and the owner's
    /// screen holds back only the owner's push (ov-744).
    ///
    /// Whose marks a call reads and writes is its caller's person
    /// (`caller_person`, ov-747): a device's own, and on the local socket
    /// its pane's. A pane on a member's login reads and writes that
    /// member's; anything else on the socket, the owner's. A rep's pane
    /// reads its member's and writes none, these four being no messenger's
    /// (`rep_may`).
    PerPerson,
    /// The plan layer: the ranking, a theme, a lane, a ruling, a train or an
    /// orchestrator's page. Every member's to change, written under their
    /// name (ov-707), and a rep's on its member's behalf, written under the
    /// member's (`stamp_rep`). The owner, 24 Oct (ov-736), reversing ov-637:
    /// a team works on one plan through one orchestrator.
    Plan,
    /// Decided by what the payload asks (`people::gate`): a workspace
    /// setting (the owner's, or a Maintainer's for the four that don't act
    /// as the owner, ov-784), a pane made the orchestrator, members' logins
    /// lent, a shell line enrolled, the owner's device bound or revoked
    /// (ov-719).
    ByPayload,
    /// The workspace's lead's and the owner's (R-54, ov-637).
    Lead,
    /// A workspace's charter, edited (ov-757): the owner's, the workspace's
    /// lead's and a Maintainer's (`people::may_edit_charter`). A rep never
    /// edits it (`rep_may`); it asks the orchestrator.
    Charter,
    /// A workspace's name and key prefix (ov-784): the owner's and a
    /// Maintainer's (`people::maintains`), never a lead's for being one. A
    /// rep never changes them (`rep_may`).
    Maintains,
    /// The owner's alone, refused `not_yours`, for the reason given.
    Owners(Why),
}

/// Why a method is the owner's alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// It posts to GitHub with the runner's `gh` login, the owner's (ov-637).
    PostsAsTheOwner,
    /// It deletes or reshapes a workspace, or sets up the runner itself
    /// (ov-637, ov-716).
    Reshapes,
    /// It deletes the owner's files (ov-716).
    Destroys,
    /// It gives a member a role (ov-757): a member admin could otherwise
    /// make themselves a Maintainer.
    Grants,
    /// It reports the owner's own device's crash (ov-797): a member's phone
    /// sends none, and the runner files none from one.
    OwnersDevice,
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
        | M::MemberList
        // Who changed what about the people here (ov-757).
        | M::MemberEvents
        // The charter (ov-755): the owner, a member and a member's rep
        // (`rep_may`) read it.
        | M::WorkspaceCharter => Reads,

        // A device's own step-up (ov-856): a nonce, its key, and an approval
        // that `step_up_rpc` holds to the same person.
        M::AuthChallenge
        | M::ClientSetStepUpKey
        | M::ClientApproveStepUp
        | M::ClientSetNodeKey
        | M::ClientRekey
        | M::ClaudeSignIn
        | M::ClaudeSignInFinish
        | M::ClaudeSignInCancel
        | M::ClaudeSignOut
        // Their own rep; only the owner starts anyone's (ov-716 rule 3).
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
        | M::TaskMove
        // A workspace made only adds (ov-716 rule 2). Removing a member is
        // a runner admin's, member or owner (rule 4), by scope.
        | M::WorkspaceCreate
        | M::MemberRemove => Members,

        M::WorkspaceMarkRead
        | M::ChangesMarkRead
        | M::TerminalSeen
        | M::TerminalWatching => PerPerson,

        M::WorkspaceSetSettings | M::TerminalSetRole | M::MemberSetOwnLogin => ByPayload,
        // A runner admin's, but no shell line, and not the owner's devices;
        // a device enrolled is nobody's until bound (ov-719).
        M::ClientEnroll | M::ClientRevoke | M::MemberSet => ByPayload,

        M::WorkspaceStartOrchestrator => Lead,

        M::WorkspaceSetCharter => Charter,

        M::WorkspaceRename | M::WorkspaceSetPrefix => Maintains,

        M::PlanSet
        | M::BoardThemeCreate
        | M::BoardThemeUpdate
        | M::BoardThemeCards
        | M::BoardThemeMove
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
        | M::PageRemove => Plan,

        M::PrReady | M::PrThreadReply => Owners(Why::PostsAsTheOwner),

        // An access or a cap is the owner's alone too, and the runner's own
        // socket's: `person_access::socket_person_only` (ov-856).
        M::MemberSetRole | M::MemberSetAccess | M::ClientSetCap => Owners(Why::Grants),

        M::CrashReport => Owners(Why::OwnersDevice),

        M::WorkspaceDelete
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
