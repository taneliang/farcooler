import Foundation
import Testing

@testable import AgentKit

// Which form a board draws in, list or kanban, from its own width and the
// person's toggle (owner decision 3, spec §5). The view measures and draws;
// every rule about what the measurement means is here.

/// The thresholds come from the kanban's own metrics: three whole 280 pt
/// columns, two 12 pt gaps and 14 pt each side is 892 (the 2A measurement,
/// which replaced spec §5's 824), and the band is 24 pt under it.
@Test("Below 868 it's a list, at 892 and up a kanban")
func belowEightSixtyEightItsAListAtEightNinetyTwoAndUpAKanban() {
    #expect(BoardForm.kanbanFrom == 892)
    #expect(BoardForm.listBelow == 868)
    for previous: BoardForm? in [nil, .list, .kanban] {
        #expect(BoardForm.resolve(width: 600, previous: previous, forced: .auto) == .list)
        #expect(BoardForm.resolve(width: 867.5, previous: previous, forced: .auto) == .list)
        #expect(BoardForm.resolve(width: 892, previous: previous, forced: .auto) == .kanban)
        #expect(BoardForm.resolve(width: 1600, previous: previous, forced: .auto) == .kanban)
    }
}

/// The hysteresis: a divider dragged back and forth across one line would
/// flicker between the forms, so going up it switches at 892 and going down
/// at 868, and in between it stays what it was. A board with no form yet
/// starts as a list there, because below 892 three columns don't fit.
@Test("Between 868 and 892 it keeps the form it had")
func betweenEightSixtyEightAndEightNinetyTwoItKeepsTheFormItHad() {
    for width in [868.0, 880, 891.5] {
        #expect(BoardForm.resolve(width: width, previous: .list, forced: .auto) == .list)
        #expect(BoardForm.resolve(width: width, previous: .kanban, forced: .auto) == .kanban)
        #expect(BoardForm.resolve(width: width, previous: nil, forced: .auto) == .list)
    }
}

@Test("A forced form ignores width")
func aForcedFormIgnoresWidth() {
    for width in [300.0, 880, 2000] {
        for previous: BoardForm? in [nil, .list, .kanban] {
            #expect(BoardForm.resolve(width: width, previous: previous, forced: .list) == .list)
            #expect(BoardForm.resolve(width: width, previous: previous, forced: .kanban) == .kanban)
        }
    }
}

/// The toggle is a segmented control, `≡` and `▦`: choosing one forces it,
/// and choosing the one already forced goes back to Automatic. Under
/// Automatic neither is forced, so either tap forces its form.
@Test("Choosing the forced form again returns to Automatic")
func choosingTheForcedFormAgainReturnsToAutomatic() {
    #expect(BoardForm.Choice.auto.choosing(.list) == .list)
    #expect(BoardForm.Choice.auto.choosing(.kanban) == .kanban)
    #expect(BoardForm.Choice.list.choosing(.list) == .auto)
    #expect(BoardForm.Choice.kanban.choosing(.kanban) == .auto)
    #expect(BoardForm.Choice.list.choosing(.kanban) == .kanban)
    #expect(BoardForm.Choice.kanban.choosing(.list) == .list)
}

/// Kept per device and per board: `board.form.<host>.<workspace>`, default
/// Automatic, and a value this build can't read is Automatic too.
@Test("The choice is kept per host and workspace, Automatic by default")
func theChoiceIsKeptPerHostAndWorkspace() throws {
    let suite = "BoardFormTests.\(UUID().uuidString)"
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }

    #expect(BoardForm.key(host: "mini", workspace: "w-1") == "board.form.mini.w-1")
    #expect(BoardForm.Choice.read(host: "mini", workspace: "w-1", from: defaults) == .auto)
    BoardForm.Choice.kanban.write(host: "mini", workspace: "w-1", in: defaults)
    #expect(BoardForm.Choice.read(host: "mini", workspace: "w-1", from: defaults) == .kanban)
    #expect(BoardForm.Choice.read(host: "mini", workspace: "w-2", from: defaults) == .auto)
    #expect(BoardForm.Choice.read(host: "studio", workspace: "w-1", from: defaults) == .auto)
    defaults.set("sideways", forKey: BoardForm.key(host: "mini", workspace: "w-1"))
    #expect(BoardForm.Choice.read(host: "mini", workspace: "w-1", from: defaults) == .auto)
    // Automatic is the default, so choosing it clears the slot.
    BoardForm.Choice.auto.write(host: "mini", workspace: "w-1", in: defaults)
    #expect(defaults.object(forKey: BoardForm.key(host: "mini", workspace: "w-1")) == nil)
}

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
