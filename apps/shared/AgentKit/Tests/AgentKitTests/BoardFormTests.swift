import Foundation
import Testing

@testable import AgentKit

// Which form a board draws in, list or kanban, from its own width and the
// person's toggle (owner decision 3, spec §5). The view measures and draws;
// every rule about what the measurement means is here.

@Test("Below 800 it's a list, at 824 and up a kanban")
func belowEightHundredItsAListAtEightTwentyFourAndUpAKanban() {
    for previous: BoardForm? in [nil, .list, .kanban] {
        #expect(BoardForm.resolve(width: 600, previous: previous, forced: .auto) == .list)
        #expect(BoardForm.resolve(width: 799.5, previous: previous, forced: .auto) == .list)
        #expect(BoardForm.resolve(width: 824, previous: previous, forced: .auto) == .kanban)
        #expect(BoardForm.resolve(width: 1600, previous: previous, forced: .auto) == .kanban)
    }
}

/// The hysteresis: a divider dragged back and forth across one line would
/// flicker between the forms, so going up it switches at 824 and going down
/// at 800, and in between it stays what it was. A board with no form yet
/// starts as a list there, because below 824 three columns don't fit.
@Test("Between 800 and 824 it keeps the form it had")
func betweenEightHundredAndEightTwentyFourItKeepsTheFormItHad() {
    for width in [800.0, 812, 823.5] {
        #expect(BoardForm.resolve(width: width, previous: .list, forced: .auto) == .list)
        #expect(BoardForm.resolve(width: width, previous: .kanban, forced: .auto) == .kanban)
        #expect(BoardForm.resolve(width: width, previous: nil, forced: .auto) == .list)
    }
}

@Test("A forced form ignores width")
func aForcedFormIgnoresWidth() {
    for width in [300.0, 812, 2000] {
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
