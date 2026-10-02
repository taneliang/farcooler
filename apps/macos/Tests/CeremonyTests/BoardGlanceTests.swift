import AppKit
import Testing

@testable import Far_Cooler

/// Glancing at tasks from the board beside them (ov-85): a click opens one
/// beside the board and a second click on it closes it, ↑ and ↓ step
/// through the list without closing, and closing goes back to the board,
/// which keeps the keyboard throughout.
struct BoardGlanceTests {
    private typealias Selection = ContentView.Selection
    private let top = Selection.workspace(host: "h", workspace: "ws", focus: nil)
    private let bil3 = Selection.workspace(host: "h", workspace: "ws", focus: .task("bil-3"))
    private let bil7 = Selection.workspace(host: "h", workspace: "ws", focus: .task("bil-7"))

    @Test("A click opens a task, switches to another, and closes the one open")
    func aClickTogglesTheTaskOpen() {
        func click(_ id: String, from: Selection?) -> Selection {
            WorkspaceNavigation.choosing(task: id, host: "h", workspace: "ws", from: from, toggles: true)
        }
        #expect(click("bil-3", from: top) == bil3)
        #expect(click("bil-7", from: bil3) == bil7)
        #expect(click("bil-7", from: bil7) == top)
        // From a worktree open beside the board, or another workspace.
        let lane = Selection.workspace(host: "h", workspace: "ws", focus: .worktree("w-3", terminal: nil))
        #expect(click("bil-3", from: lane) == bil3)
        #expect(click("bil-3", from: .needsYou) == bil3)
        // A glance never closes.
        #expect(WorkspaceNavigation.choosing(task: "bil-7", host: "h", workspace: "ws", from: bil7, toggles: false) == bil7)
    }

    @Test("↑ and ↓ step through the rows shown, held at either end")
    func theArrowsStepThroughTheRows() {
        let ids = ["bil-1", "bil-3", "bil-7"]
        #expect(BoardKeys.step(from: nil, by: 1, in: ids) == "bil-1")
        #expect(BoardKeys.step(from: nil, by: -1, in: ids) == "bil-7")
        #expect(BoardKeys.step(from: "bil-3", by: 1, in: ids) == "bil-7")
        #expect(BoardKeys.step(from: "bil-3", by: -1, in: ids) == "bil-1")
        #expect(BoardKeys.step(from: "bil-7", by: 1, in: ids) == "bil-7")
        #expect(BoardKeys.step(from: "bil-1", by: -1, in: ids) == "bil-1")
        // One no longer shown (its section folded) starts over.
        #expect(BoardKeys.step(from: "bil-9", by: 1, in: ids) == "bil-1")
        #expect(BoardKeys.step(from: "bil-3", by: 1, in: []) == nil)
    }

    @Test("Closing goes back to the board, a loose worktree's included")
    func closingGoesToTheBoard() {
        #expect(WorkspaceNavigation.closing(bil3, board: "ws") == top)
        let lane = Selection.workspace(host: "h", workspace: "ws", focus: .worktree("w-3", terminal: "t"))
        #expect(WorkspaceNavigation.closing(lane, board: "ws") == top)
        let loose = Selection.looseWorktree(host: "h", worktree: "w-9", terminal: nil)
        #expect(WorkspaceNavigation.closing(loose, board: "main") == .workspace(host: "h", workspace: "main", focus: nil))
        #expect(WorkspaceNavigation.closing(loose, board: nil) == nil)
        #expect(WorkspaceNavigation.closing(top, board: "ws") == nil)
        #expect(WorkspaceNavigation.closing(.needsYou, board: "ws") == nil)
    }

    @Test("The board draws the task open beside it as selected")
    func theOpenTaskIsSelected() {
        #expect(WorkspaceNavigation.selectedTask(bil3, trail: nil, board: "ws") == "bil-3")
        #expect(WorkspaceNavigation.selectedTask(bil3, trail: nil, board: "other") == nil)
        #expect(WorkspaceNavigation.selectedTask(top, trail: nil, board: "ws") == nil)
        // A worktree opened from its task: the task stays selected.
        let lane = Selection.workspace(host: "h", workspace: "ws", focus: .worktree("w-3", terminal: nil))
        #expect(WorkspaceNavigation.selectedTask(lane, trail: bil3, board: "ws") == "bil-3")
        #expect(WorkspaceNavigation.selectedTask(lane, trail: nil, board: "ws") == nil)
    }

    @Test("The board keeps the keyboard across its own opens, switches and closes")
    func theBoardKeepsTheKeyboard() {
        func keeps(_ from: Selection?, _ to: Selection?, pending: Bool = true) -> Bool {
            WorkspaceNavigation.boardKeepsKeyboard(pending: pending, from: from, to: to)
        }
        #expect(keeps(top, bil3))
        #expect(keeps(bil3, bil7))
        #expect(keeps(bil7, top))
        #expect(keeps(.looseWorktree(host: "h", worktree: "w-9", terminal: nil), top))
        #expect(!keeps(top, bil3, pending: false))
        #expect(!keeps(bil3, .workspace(host: "h", workspace: "billing", focus: nil)))
        #expect(!keeps(.needsYou, bil3))
        #expect(!keeps(top, top))
    }

    @MainActor
    @Test("Esc closes a loose worktree too, when no terminal wants it")
    func escClosesALooseWorktree() {
        let loose = Selection.looseWorktree(host: "h", worktree: "w-9", terminal: nil)
        #expect(EscapeBack.goesBack(responder: NSView(), selection: loose, focusColumn: false))
        #expect(!EscapeBack.goesBack(responder: TerminalRenderView(), selection: loose, focusColumn: false))
        // At the workspace level, with the orchestrator popped open over
        // the board: Esc puts it away.
        #expect(EscapeBack.goesBack(responder: NSView(), selection: top, focusColumn: true))
        #expect(!EscapeBack.goesBack(responder: NSView(), selection: top, focusColumn: false))
    }
}
