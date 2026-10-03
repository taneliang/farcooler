import Foundation

// Which task a pane is shown under.
//
// Every pane header names its task ("bil-9 Invoice PDF export"): the Mac's task
// column and a worktree layout's group bar, the iPhone's pane bar, and
// Android's top bar. The rule is stated once here and once in Kotlin
// (`TaskLink.kt`), against one set of cases, so the three agree about one pane.
// See spec §3.2 of `docs/superpowers/specs/2026-09-28-workspace-ui-design.md`.
//
// A DISPLAY rule, and only that. A person's shell in a task's worktree is shown
// under the task, because that's where it is, but it isn't the task's agent:
// `TaskAgentLink.isWorking` reads `boardTaskID` alone and never this, so the
// board's "Agent" pill, its live-agent count and dispatch's one-agent-per-task
// check never see the fallback.

/// A pane, as `TaskLink` asks about it.
///
/// A protocol because the Mac's `Terminal` isn't AgentKit's, as with
/// `TaskBoardPane`, whose `boardTaskID` this shares: a type conforming to both
/// declares it once.
public protocol TaskLinkPane {
    /// The board task this pane was opened for, or nil for a pane nobody
    /// dispatched. `Terminal.task_id`.
    var boardTaskID: String? { get }
    /// Whether this pane is its workspace's orchestrator (`role` is
    /// `orchestrator`). The Mac's `Terminal` and the phone's both have it.
    var isOrchestrator: Bool { get }
}

/// A worktree, as `TaskLink` asks about it.
public protocol TaskLinkWorktree {
    /// The ids of the tasks working in it that aren't Done or Cancelled:
    /// `Worktree.open_tasks`, as the runner sends it. Empty from a runner too
    /// old to fill it.
    var openTaskIDs: [String] { get }
    /// Whether this is its repository's own checkout rather than a lane made
    /// for some work. `Worktree.is_main_checkout`.
    var isRepositoryCheckout: Bool { get }
}

public enum TaskLink {
    /// The id of the task `pane` is shown under, or nil for none.
    ///
    /// Never for an orchestrator: it leads its workspace rather than working
    /// one task, and "an orchestrator is never a task's agent" (spec §2.2).
    /// Without this, an orchestrator in a main checkout with one open task
    /// would wear that task's chip. Android's `TaskLink.kt` makes the same
    /// exception (coordinator ruling, ov-55).
    ///
    /// The pane's own task when it was dispatched for one, whatever its
    /// worktree holds: a second agent sent to the same lane for another task
    /// is still on that task. Otherwise its worktree's task when the worktree
    /// has exactly one. Two open tasks in one worktree is none rather than a
    /// pick: a header naming the wrong task is worse than a header naming
    /// nothing.
    ///
    /// `worktree` is the one the pane runs in.
    public static func task(of pane: some TaskLinkPane, in worktree: some TaskLinkWorktree) -> String? {
        guard !pane.isOrchestrator else { return nil }
        if let own = pane.boardTaskID, !own.isEmpty { return own }
        let open = worktree.openTaskIDs
        return open.count == 1 ? open[0] : nil
    }
}

// Which task an agent's own notifications fold into (ov-94, ov-107).
//
// A different question from `TaskLink.task`, which names a header. This one
// decides whether an agent's "finished" or "needs you" banner is left to its
// task's notice, the one the runner composes (`task_link::task_of` in
// `crates/daemon/src/task_link.rs`), so it has to give the runner's answer.
// The Mac and the iPhone ask it here; Android's `TaskLink.noticeTaskId` states
// the same cases.

extension TaskLink {
    /// The id of the task `pane`'s own notifications fold into, or nil when
    /// it notifies as itself.
    ///
    /// - Never an orchestrator's: it works for its whole workspace.
    /// - The task it was opened for, when it was opened for one.
    /// - Otherwise its lane's task, when the lane has exactly one open task.
    ///   With two, a guess would file an agent's question under the wrong
    ///   task, which is worse than leaving it on its own.
    /// - Never by lane in the repository's main checkout. Ad hoc agents run
    ///   there, and one task dispatched to it (`task dispatch --worktree`)
    ///   would take in every one of them, and turn their turn ends into that
    ///   task's Done, which is off by default. The runner's `task_of` still
    ///   folds there (ov-107 report): until it doesn't, an agent there gets
    ///   both banners, never neither.
    public static func noticeTask(of pane: some TaskLinkPane, in worktree: some TaskLinkWorktree) -> String? {
        guard !pane.isOrchestrator else { return nil }
        if let own = pane.boardTaskID, !own.isEmpty { return own }
        guard !worktree.isRepositoryCheckout else { return nil }
        let open = worktree.openTaskIDs
        return open.count == 1 ? open[0] : nil
    }

    /// Whether `pane`'s own banner is left to its task's notice.
    ///
    /// `noticeReachesHere` says whether that notice reaches this device at
    /// all. On the Mac it's the runner advertising `task_notices`: the Mac
    /// posts the runner's notice itself. On a phone, which hears notices only
    /// as pushes, it's `taskNoticeReachesPhone`. Without it the agent's own
    /// banner is the only word there is, so it posts as it always has.
    public static func leavesBannerToTask(
        _ pane: some TaskLinkPane, in worktree: some TaskLinkWorktree, noticeReachesHere: Bool
    ) -> Bool {
        noticeReachesHere && noticeTask(of: pane, in: worktree) != nil
    }

    /// Whether a task notice from the runner `build` describes reaches this
    /// phone, which hears one only as a push: the runner composes them
    /// (`task_notices`), it's paired with the relay (`pushPaired`), and this
    /// phone has registered with the relay (`PushRegistration.registered`).
    /// Each missing one means no push, and the agent's own banner is all
    /// there is.
    public static func taskNoticeReachesPhone(_ build: DaemonBuild?, registered: Bool) -> Bool {
        guard let build else { return false }
        return build.can("task_notices") && build.pushPaired && registered
    }
}

/// One pane's change, as the iPhone's `Notifier.report` hears it: the pane,
/// its worktree's name for the banner's body, and whether its banner is left
/// to its task's push (ov-107).
struct AgentReport {
    var terminal: Terminal
    var worktree: String
    var leftToTask: Bool
}

extension Fleet {
    /// Every pane in this fleet as the iPhone's `Notifier.report` takes it,
    /// read from the runner `build` describes. Here rather than in the app's
    /// `Connection` so that the fold, the one decision in the loop, can be
    /// tested: the phone has no unit test target of its own.
    func agentReports(runner build: DaemonBuild?, registered: Bool) -> [AgentReport] {
        let reaches = TaskLink.taskNoticeReachesPhone(build, registered: registered)
        return worktrees.flatMap { worktree in
            worktree.terminals.map { terminal in
                AgentReport(
                    terminal: terminal, worktree: worktree.task,
                    leftToTask: TaskLink.leavesBannerToTask(terminal, in: worktree, noticeReachesHere: reaches))
            }
        }
    }
}

