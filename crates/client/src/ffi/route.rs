//! The names an app passes `farcooler_client_call`, one per wire method.
//!
//! In its own file, outside `#[cfg(test)]`, so the exhaustive match below is
//! compiled into every build (ov-171). `dispatch` matches wire names as
//! strings and the compiler cannot check those; this match is the half it can.

use farcooler_protocol::method::Method;

/// The name an app passes `farcooler_client_call` to reach each wire method,
/// or `None` where no app can.
///
/// A match on `Method` with no wildcard, so a method added to the protocol's
/// table does not build here, in any profile, until somebody decides whether
/// apps reach it. It was test-only (ov-115), so `cargo build` and a release
/// build passed a method nobody had decided on; `dispatch`'s last arm now
/// calls it, which is what keeps it in every build. Deciding `None` is
/// allowed; forgetting is not. `every_route_has_an_arm` then calls each name.
pub(super) fn route(method: Method) -> Option<&'static str> {
    match method {
        // An arm under its own wire name.
        Method::HostHealth
        | Method::DaemonVersion
        | Method::RepositoryRegister
        | Method::RepositoryRootList
        | Method::RepositoryRootAdd
        | Method::RepositoryRootRemove
        | Method::WorktreeCreate
        | Method::WorktreeHide
        | Method::WorktreeHydrateLfs
        | Method::WorktreeUnhide
        | Method::WorktreeRemove
        | Method::BranchList
        | Method::WorktreeFileSearch
        | Method::TerminalCreate
        | Method::TerminalScreen
        | Method::TerminalWrite
        | Method::TerminalResize
        | Method::TerminalStop
        | Method::TerminalSeen
        | Method::TerminalRemove
        | Method::TerminalDismissLost
        | Method::TerminalRestart
        | Method::TerminalSetPaneMode
        | Method::TerminalAgentSubscribe
        | Method::TerminalAgentPrompt
        | Method::TerminalDraftPrompt
        // Withdraw a draft held behind a dialog (ov-385).
        | Method::TerminalDraftWithdraw
        | Method::TerminalAgentAnswer
        | Method::TerminalAgentSetMode
        | Method::TerminalAgentSetModel
        | Method::TerminalAgentSetConfig
        | Method::TerminalAgentEditQueued
        | Method::TerminalAgentCancelQueued
        | Method::TerminalAgentCancel
        | Method::TerminalAgentSteerQueued
        | Method::ChangesChangeSet
        | Method::ChangesCommitFiles
        | Method::ChangesFileDiff
        | Method::ChangesSetBase
        | Method::ChangesMarkRead
        | Method::ChangesInbox
        | Method::StackGet
        | Method::PrRefresh
        | Method::AdapterUpsert
        | Method::AdapterDelete
        | Method::AdapterTest
        | Method::ThemeUpsert
        | Method::ThemeDelete
        | Method::SettingsSetBranchPrefix
        | Method::ClientList
        | Method::ClientEnroll
        | Method::ClientRevoke
        | Method::WorktreeReorder
        | Method::TaskList
        | Method::TaskGet
        | Method::TaskNote
        // Read state on the runner (ov-113): a phone opens a ticket, and
        // the Unread section is built from what comes back.
        | Method::WorkspaceMarkRead
        | Method::WorkspaceStartOrchestrator
        | Method::TerminalWatching
        // A worktree's files and the runner's read-only folders, read-only
        // (ov-259).
        | Method::WorktreeListDir
        | Method::WorktreeReadFile
        | Method::UsageTask
        // The plan layer's two reads (ov-274), behind `board_plan`.
        | Method::PlanGet
        | Method::PlanEvents
        // Orchestrator pages (ov-269) are read-only on a phone: the list and
        // one page. The orchestrator is their one writer.
        | Method::PageList
        | Method::PageGet
        // A terminal's agent rows (ov-366), for the native views (ov-372).
        | Method::AgentRows
        | Method::AgentRowsFollow
        // A prompt's images on its turn row (ov-454).
        | Method::AgentImage
        // The native view's composer (ov-372), and its Stop and Send Now (ov-368).
        | Method::TerminalCompose
        | Method::TerminalInterrupt
        | Method::TerminalSendNow
        // Bring Here (ov-369).
        | Method::TerminalBringDraft
        // Its setting, the phones' settings row (ov-373): `host_admin` on
        // the runner, so a phone enrolled at `control` is refused it.
        | Method::SettingsSetProjector => Some(method.name()),
        // The owner's two marks on a ruling (ov-333): Keep, and Keep All.
        // `ruling.set` is a phone's only as `ruling.keep`: it can't reverse or
        // settle any other way. Reversing is a request to the orchestrator,
        // which goes as a prompt (`terminal.agent_prompt`).
        Method::RulingSet => Some("ruling.keep"),
        Method::RulingKeepAll => Some(method.name()),
        Method::AdapterList => Some("adapters"),
        Method::ThemeList => Some("themes"),
        Method::RepositoryList => Some("repositories"),
        Method::NeedsYouList => Some("needs_you"),
        Method::WorktreeList | Method::TerminalList => Some("fleet"),
        // `host` is `host.health` with this client's build beside it.
        Method::HostGet => None,
        // The Mac app owns the local daemon's lifecycle, through the CLI.
        Method::DaemonShutdown => None,
        // Discovery and a node key are the Mac's and the ceremony's: paths
        // sit behind `host_admin`, and the tunnel is joined by the CLI.
        Method::WorktreeDiscover | Method::ClientSetNodeKey => None,
        // The title bar's message to a terminal orchestrator (ov-214): a phone
        // has no title bar. Ask the Orchestrator's paste (ov-184,
        // `terminal.draft_prompt`) is routed above, for the phones' task screen
        // (ov-241).
        Method::TerminalTell => None,
        // Agents and their orchestrator, through the CLI (ov-455): the apps
        // read messages as the card's notes.
        Method::MessageSend => None,
        // Their own C entry points, `farcooler_client_paste_file` and
        // `farcooler_client_stream_start`, because neither is one reply.
        Method::TerminalPasteFile | Method::TerminalAttach => None,
        // Tiling is tmux's, and a phone shows one pane at a time.
        Method::LayoutList
        | Method::LayoutSplit
        | Method::LayoutMove
        | Method::LayoutResize
        | Method::LayoutBreak
        | Method::LayoutRename
        | Method::LayoutViewport
        | Method::LayoutPreset
        | Method::LayoutCycle
        | Method::LayoutFocus
        | Method::LayoutZoom
        | Method::LayoutSwap
        | Method::LayoutGroupSelect => None,
        // The orchestrator owns the task list (ov-184): a phone never
        // creates, edits, moves or blocks a task. `PHONES_NEVER_WRITE_A_TASK`.
        Method::TaskCreate
        | Method::TaskUpdate
        | Method::TaskSetStatus
        | Method::TaskBlock
        | Method::TaskMove => None,
        // The workspaces' writes and the stack's, and two board reads: the
        // Mac and the CLI make them, and no phone screen offers them yet.
        // `workspace.list` is read inside `worktree.create`.
        Method::StackSetParent
        | Method::TaskGetByKey
        | Method::TaskSearch
        | Method::WorkspaceList
        | Method::WorkspaceCreate
        | Method::WorkspaceRename
        | Method::WorkspaceSetPrefix
        | Method::WorkspaceSetSettings
        | Method::WorkspaceDelete
        | Method::WorktreeAssign
        | Method::TerminalSetRole
        // Naming a terminal (ov-234): the phones show the name and do not set it.
        | Method::TerminalRename => None,
        // The plan layer's writes (ov-268): the CLI makes them, the
        // orchestrator being the layer's one writer. A phone reads `plan.get`
        // and `plan.events`, routed above, and never routes a write.
        Method::PlanSet
        | Method::BoardThemeCreate
        | Method::BoardThemeUpdate
        | Method::BoardThemeCards
        | Method::LaneCreate
        | Method::LaneUpdate
        | Method::LaneCards
        | Method::LaneAgent
        // A phone neither writes a page nor reads how often one is published.
        | Method::PageSet
        | Method::PageRemove
        | Method::PageStats
        // Rulings (ov-304) are read in `plan.get`'s answer. Only the
        // orchestrator records one: the owner asks it, citing the short id.
        | Method::RulingAdd
        // Trains (ov-309) are read in `plan.get`'s answer too, and only the
        // orchestrator writes them.
        | Method::TrainStart
        | Method::TrainSet
        // Landing detection (ov-313) is the CLI's, run with the owner's `gh`.
        | Method::RepositoryLanding => None,
        // The CLI reads it today; the Summary page (ov-188 phase 3) will
        // route it here.
        Method::ReportGet => None,
        // Spend (ov-194): the CLI's `farcooler report` reads the whole
        // runner's through `Session`. A task's own is the task screen's
        // Usage section (ov-195), routed above.
        Method::UsageReport => None,
        // When a task starts and who works it (ov-212, ov-213): the CLI
        // writes them, from the orchestrator's session. The phones only read
        // the board (ov-184), and the board's rows carry both.
        Method::TaskSetWait | Method::TaskSetLine | Method::TaskWorker => None,
    }
}
