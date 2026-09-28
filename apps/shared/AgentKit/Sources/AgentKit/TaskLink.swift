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
