import AgentKit
import SwiftUI

/// Where the Plan view's pages open, and what they need from the window
/// (ov-273).
extension ContentView {
    /// A theme's or lane's page, in the main area where a task opens. The
    /// navigator keeps the keyboard, as it does for the History page.
    func openPlan(_ page: PlanPage, host: String, workspace: String) {
        let next = Selection.workspace(host: host, workspace: workspace, focus: .plan(page))
        guard next != selection else { return }
        trail = nil
        focusColumn = false
        selection = next
    }

    /// The page open in `workspace`'s main area, if it's a plan page.
    func planPage(host: String, workspace: String) -> PlanPage? {
        if case .workspace(host, workspace, .plan(let page)?)? = selection { return page }
        return nil
    }

    /// What a plan page reads from the board: its rows, and where a task,
    /// a lane or a theme opens.
    func planContext(_ board: TaskBoardStore, host: String, workspace: String) -> PlanPageContext {
        PlanPageContext(
            rows: Dictionary(board.board.rows.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a }),
            onTask: { row in openTask(row.id, host: host, workspace: workspace) },
            onOpen: { page in openPlan(page, host: host, workspace: workspace) })
    }
}
