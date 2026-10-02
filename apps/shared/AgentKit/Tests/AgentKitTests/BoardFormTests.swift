import Foundation
import Testing

@testable import AgentKit

// How a board's list is drawn: its sections, which open and close, and what
// a quiet board says. Every board is this list (ov-83 removed the kanban).

/// The list form's sections: an empty one is a collapsed header that can't
/// open; Done and Canceled start collapsed; the rest start open.
@Test("An empty section is collapsed and can't open; Done and Canceled start collapsed")
func anEmptySectionIsCollapsedAndCantOpen() {
    let full = TaskBoardColumn(
        status: .todo,
        rows: [TaskRow(id: "1", key: "-1", title: "t", status: .todo, statusSince: .now)])
    let empty = TaskBoardColumn(status: .todo, rows: [])
    #expect(BoardForm.canExpand(full))
    #expect(!BoardForm.canExpand(empty))
    #expect(BoardForm.isExpanded(full, collapsed: []))
    #expect(!BoardForm.isExpanded(full, collapsed: [.todo]))
    #expect(!BoardForm.isExpanded(empty, collapsed: []))
    #expect(BoardForm.collapsedByDefault == [.done, .cancelled])
}

/// Which sections a board has collapsed, kept per device and per board like
/// the form: Done and Canceled until the person says otherwise, and what
/// they said after that, an empty set included.
@Test("Collapsed sections are kept per board, Done and Canceled by default")
func collapsedSectionsAreKeptPerBoard() throws {
    let suite = "BoardFormTests.\(UUID().uuidString)"
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }

    #expect(BoardForm.collapsedKey(host: "mini", workspace: "w-1") == "board.collapsed.mini.w-1")
    #expect(BoardForm.collapsed(host: "mini", workspace: "w-1", from: defaults) == [.done, .cancelled])
    BoardForm.setCollapsed([.backlog], host: "mini", workspace: "w-1", in: defaults)
    #expect(BoardForm.collapsed(host: "mini", workspace: "w-1", from: defaults) == [.backlog])
    #expect(BoardForm.collapsed(host: "mini", workspace: "w-2", from: defaults) == [.done, .cancelled])
    // Everything opened is remembered as everything opened, not as unset.
    BoardForm.setCollapsed([], host: "mini", workspace: "w-1", in: defaults)
    #expect(BoardForm.collapsed(host: "mini", workspace: "w-1", from: defaults).isEmpty)
}

/// A board with no task anywhere is an empty state, not seven zero headers.
@Test("A board with no task in any status is blank")
func aBoardWithNothingOnItIsBlank() {
    #expect(BoardForm.isBlank(TaskBoardModel.board(from: [])))
}

@Test("A board with one task, or one unplaceable row, is not blank")
func aBoardWithAnyRowIsNotBlank() {
    let task = TaskBoardColumn(
        status: .todo,
        rows: [TaskRow(id: "1", key: "-1", title: "t", status: .todo, statusSince: .now)])
    #expect(!BoardForm.isBlank(TaskBoardModel(columns: [task], unreadable: [])))
    let odd = UnreadableTaskRow(id: "2", key: "-2", title: "t", status: "someday")
    #expect(!BoardForm.isBlank(TaskBoardModel(columns: [], unreadable: [odd])))
}

/// Empty statuses are said once, not drawn as a header each.
@Test("The empty statuses are named in one sentence")
func emptyStatusesAreNamedOnce() {
    let todo = TaskBoardColumn(
        status: .todo,
        rows: [TaskRow(id: "1", key: "-1", title: "t", status: .todo, statusSince: .now)])
    let one = TaskBoardModel(columns: [todo, TaskBoardColumn(status: .backlog, rows: [])])
    #expect(BoardForm.emptyNote(one) == "Backlog is empty.")
    let three = TaskBoardModel(columns: [
        todo, TaskBoardColumn(status: .backlog, rows: []),
        TaskBoardColumn(status: .inReview, rows: []), TaskBoardColumn(status: .cancelled, rows: []),
    ])
    #expect(BoardForm.emptyNote(three) == "Backlog, In Review and Canceled are empty.")
    #expect(BoardForm.emptyNote(TaskBoardModel(columns: [todo])) == nil)
}
