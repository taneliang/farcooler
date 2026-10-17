import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The task view's own least width (ov-601): `WorkspaceColumns.taskViewMinimum`
/// is what the navigator tier keeps for it, and this hosts the real header and
/// tab bar at the narrowest width SwiftUI will give them, so a wider button or
/// label moves the number instead of clipping the view at 1,600 pt.
@MainActor
struct TaskViewFitTests {
    /// The width `view` takes when it is offered almost none: the sum of what
    /// can't shrink (`fixedSize`) and what's left of what can.
    static func least<V: View>(_ view: V) -> CGFloat {
        NSHostingController(rootView: view).sizeThatFits(in: CGSize(width: 1, height: 200)).width
    }

    static let worktree = Worktree(
        id: "w", short: "w", task: "a-lane-with-a-long-real-name", branch: "b", repository: "r", host: "", path: "/tmp/x",
        state: "active",
        terminals: [
            Terminal(id: "a", short: "a", title: "a", preset: "claude", state: "running", epoch: 0),
            Terminal(id: "b", short: "b", title: "b", preset: "zsh", state: "running", epoch: 0),
        ], repositoryID: "r", workspace: "ws")

    /// What a task with several agents draws, at the longest labels it has:
    /// the bar in its compact form, which is what it takes at a width this
    /// small (ov-601).
    static func parts() -> (header: CGFloat, tabs: CGFloat) {
        let row = TaskRow(
            id: "t", key: "bil-128", title: "Total each invoice, and a title long enough to need its ellipsis",
            status: .inProgress, statusSince: Date())
        let pane = BoardPane(terminal: worktree.terminals[0], worktree: worktree)
        return (
            least(TaskViewHeader(row: row, agent: pane)),
            least(TaskTabBar(
                tab: .agent, onChoose: { _ in }, worktree: worktree, agents: [pane, pane], chosen: pane, onNewTerminal: {}))
        )
    }

    @Test("The task view's minimum is its widest part, measured, and not more than a few points over")
    func minimumIsMeasured() {
        let measured = max(Self.parts().header, Self.parts().tabs)
        #expect(
            WorkspaceColumns.taskViewMinimum >= measured,
            "the task view needs \(measured) pt and taskViewMinimum is \(WorkspaceColumns.taskViewMinimum)")
        #expect(
            WorkspaceColumns.taskViewMinimum <= measured + 8,
            "taskViewMinimum \(WorkspaceColumns.taskViewMinimum) is well over the \(measured) pt measured")
    }

    @Test("No width the navigator sits beside the canvas at leaves the task view less than it needs")
    func tierAdmitsOnlyWhatFits() {
        let measured = max(Self.parts().header, Self.parts().tabs)
        for width in stride(from: CGFloat(1180), through: 2600, by: 1) {
            let sizing = WorkspaceColumns.Canvas()
            guard sizing.hasCanvas(width), sizing.navigatorBeside(width) else { continue }
            let frames = WorkspaceColumns.frames(
                width: width,
                arrangement: WorkspaceColumns.canvasLayout(opened: true, hasBoard: true, focused: false, floats: false),
                navigator: sizing.navigator, cell: sizing.cell, chatColumns: sizing.chatColumns)
            #expect(frames.main >= measured, "the task view is \(frames.main) pt at \(width), and needs \(measured)")
        }
    }
}
