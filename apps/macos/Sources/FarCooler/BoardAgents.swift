import AgentKit
import SwiftUI

// Moved out of TaskBoard.swift (ov-273), whole, for the file-size budget.

/// One pane that is working a task, with the worktree it is in.
///
/// Carried together because going to it needs both: `Selection.terminal`
/// names the worktree and the host as well as the terminal, and a pane found
/// on its own would have to be looked up again to learn where it lives.
struct BoardPane: Identifiable, Equatable {
    let terminal: Terminal
    let worktree: Worktree

    var id: String { terminal.id }

    /// What a menu item offering this pane says: the pane, then where it is.
    /// "claude in fix-reconnect", because two agents on one task are usually
    /// the same program, and the worktree is what tells them apart.
    ///
    /// Numbered the way the sidebar numbers it — "claude 2 in fix-reconnect"
    /// — when the worktree holds two alike, because `dispatch --again` can
    /// put the second agent in the same lane and two identical menu items
    /// would be a coin toss. See `Worktree.ordinals()`.
    var title: String {
        "\(terminal.displayName(ordinal: worktree.ordinals()[terminal.id])) in \(worktree.task)"
    }

    /// Menu titles for several panes, told apart even where their names are
    /// not: two panes the agent titled identically get their short ids.
    static func titles(_ panes: [BoardPane]) -> [String] {
        let plain = panes.map(\.title)
        let counts = Dictionary(plain.map { ($0, 1) }, uniquingKeysWith: +)
        return zip(panes, plain).map { pane, title in
            counts[title, default: 0] > 1 ? "\(title) (\(pane.terminal.short))" : title  // not a count: a twin pane told apart by its id
        }
    }

    /// Where going to `pane` should land, looked up again in the fleet as it
    /// is NOW rather than as it was when the card drew.
    ///
    /// A menu is open while the fleet moves under it, so the pane can have
    /// exited and been reaped by the time it is chosen. Then: its worktree,
    /// if that is still there, and nil — stay on the board and say so — if
    /// neither is. Either lands as `WorkspaceSelection` says a pane or a
    /// worktree does.
    static func landing(for pane: BoardPane, in fleet: Fleet) -> ContentView.Selection? {
        let host = pane.worktree.host ?? ""
        guard let worktree = WorkspaceSelection.worktree(host: host, id: pane.worktree.id, in: fleet)
        else { return nil }
        let live = worktree.terminals.contains(where: { $0.id == pane.terminal.id })
        return WorkspaceSelection.landing(in: worktree, terminal: live ? pane.terminal.id : nil, fleet: fleet)
    }
}

/// Every pane on a board's runner, and whether that runner says which pane
/// works which task.
///
/// A value handed to the board by the window, which is what holds the fleet.
/// The rule for which of these is working a card is AgentKit's
/// (`TaskAgentLink.isWorking`); this only pairs each pane with its worktree
/// so that the answer is somewhere you can go.
struct BoardAgents {
    /// The runner's worktrees, as the sidebar has them.
    var worktrees: [Worktree]
    /// Whether the runner advertises `terminal_task`. Without it no pane
    /// carries a task, and the board makes no claim either way.
    var runnerRecordsTasks: Bool

    static let none = BoardAgents(worktrees: [], runnerRecordsTasks: false)

    /// The panes a board may speak of on one runner — or none, which is
    /// "can't say": no pills, no "No Agent", and no count in the sidebar.
    ///
    /// Two gates. The runner has to record which pane works which task
    /// (`terminal_task`), and it has to be connected right now. Anything
    /// else — connecting, reconnecting, unreachable, not installed — means
    /// the worktrees are the last ones read before the link went, kept so
    /// the sidebar stays put, and the agents in them may have exited since.
    ///
    /// `.connected` and not `state.refusal == nil`: a dead runner spends most
    /// of an outage in `.reconnecting` between attempts, and that gate let the
    /// frozen pills blink back on for every one of them. `FleetStore.reading`
    /// counts the status bar's live panes by the same rule.
    static func on(
        _ worktrees: [Worktree], state: HostState, build: DaemonBuild?
    ) -> BoardAgents {
        // The rule is AgentKit's, so this board and the phone's cannot drift.
        guard TaskAgentLink.speaksOfAgents(connected: state == .connected, build: build)
        else { return .none }
        return BoardAgents(worktrees: worktrees, runnerRecordsTasks: true)
    }

    private var panes: [BoardPane] {
        worktrees.flatMap { ws in ws.terminals.map { BoardPane(terminal: $0, worktree: ws) } }
    }

    /// The panes working `row`, in sidebar order. Empty on a runner that
    /// doesn't record tasks, whatever its panes say.
    func live(for row: TaskRow) -> [BoardPane] {
        guard runnerRecordsTasks else { return [] }
        let working = Set(row.livePanes(in: worktrees.flatMap(\.terminals)).map(\.id))
        return panes.filter { working.contains($0.id) }
    }

    /// The pane `row`'s subagents live in, or nil (`TaskWorkers.orchestrator`).
    func orchestrator(for row: TaskRow) -> BoardPane? {
        runnerRecordsTasks ? TaskWorkers.orchestrator(of: row, in: worktrees) : nil
    }

    func presence(for row: TaskRow) -> TaskAgentPresence {
        row.agentPresence(livePanes: live(for: row).count, runnerRecordsTasks: runnerRecordsTasks)
    }
}
