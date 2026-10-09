import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Switching workspaces goes back to where each was left (ov-442): the
/// window's side of it, as values (`WorkspacePlaces`, `PlaceSwitch`).
struct WorkspacePlacesTests {
    typealias Selection = ContentView.Selection

    private static func defaults() -> UserDefaults {
        UserDefaults(suiteName: "places-\(UUID().uuidString)")!
    }

    private static func board(_ id: String) -> Selection { .workspace(host: "", workspace: id, focus: nil) }
    private static func task(_ ws: String, _ id: String) -> Selection {
        .workspace(host: "", workspace: ws, focus: .task(id))
    }

    /// What the window does for one change, as `ContentView` does it: a
    /// substitute is selected and goes through the same step, and the
    /// history records what carries on.
    private static func drive(
        _ places: inout PlaceSwitch, _ history: inout NavigationHistory, selection: inout Selection?,
        to next: Selection, in fleet: Fleet = .empty, defaults: UserDefaults
    ) {
        var was = selection
        selection = next
        var change = next
        while true {
            switch places.changed(from: was, to: change, in: fleet, defaults: defaults) {
            case .open(let place):
                was = change
                selection = place
                change = place
            case .carryOn(let from):
                history.record(from: from, to: change)
                return
            }
        }
    }

    @Test("Switching away from a workspace and back opens what was open in it")
    func switchBackRestoresTheTask() {
        let defaults = Self.defaults()
        var places = PlaceSwitch(), history = NavigationHistory()
        var selection: Selection?
        Self.drive(&places, &history, selection: &selection, to: Self.board("a"), defaults: defaults)
        Self.drive(&places, &history, selection: &selection, to: Self.task("a", "t1"), defaults: defaults)
        Self.drive(&places, &history, selection: &selection, to: Self.board("b"), defaults: defaults)
        Self.drive(&places, &history, selection: &selection, to: Self.task("b", "t2"), defaults: defaults)
        // The sidebar, ⌘1 and the switcher all select a workspace's board.
        Self.drive(&places, &history, selection: &selection, to: Self.board("a"), defaults: defaults)
        #expect(selection == Self.task("a", "t1"))
        Self.drive(&places, &history, selection: &selection, to: Self.board("b"), defaults: defaults)
        #expect(selection == Self.task("b", "t2"))
    }

    @Test("A plan page and a worktree come back too, and the board of the same workspace is left as asked")
    func otherPlacesAndTheSameWorkspace() {
        let defaults = Self.defaults()
        let fleet = Fleet(
            runtimeHealthy: true, livePanes: 0,
            worktrees: [
                Worktree(
                    id: "wt", short: "wt", task: "t", branch: "b", repository: "r", host: "", path: "/tmp/wt",
                    state: "active", terminals: [])
            ],
            branchPrefix: nil)
        var places = PlaceSwitch(), history = NavigationHistory()
        var selection: Selection?
        let plan = Selection.workspace(host: "", workspace: "a", focus: .plan(.theme("th")))
        let tree = Selection.workspace(host: "", workspace: "b", focus: .worktree("wt", terminal: nil))
        Self.drive(&places, &history, selection: &selection, to: plan, in: fleet, defaults: defaults)
        Self.drive(&places, &history, selection: &selection, to: tree, in: fleet, defaults: defaults)
        Self.drive(&places, &history, selection: &selection, to: Self.board("a"), in: fleet, defaults: defaults)
        #expect(selection == plan)
        // Stepping up to the board from inside the workspace is not a switch.
        Self.drive(&places, &history, selection: &selection, to: Self.board("a"), in: fleet, defaults: defaults)
        #expect(selection == Self.board("a"))
        Self.drive(&places, &history, selection: &selection, to: Self.board("b"), in: fleet, defaults: defaults)
        #expect(selection == tree)
        Self.drive(&places, &history, selection: &selection, to: Self.board("b"), in: fleet, defaults: defaults)
        #expect(selection == Self.board("b"))
        // So the board is what b keeps now, and what a switch back finds.
        Self.drive(&places, &history, selection: &selection, to: plan, in: fleet, defaults: defaults)
        Self.drive(&places, &history, selection: &selection, to: Self.board("b"), in: fleet, defaults: defaults)
        #expect(selection == Self.board("b"))
        Self.drive(&places, &history, selection: &selection, to: tree, in: fleet, defaults: defaults)
        // A worktree removed since is not opened.
        Self.drive(&places, &history, selection: &selection, to: Self.board("a"), in: .empty, defaults: defaults)
        Self.drive(&places, &history, selection: &selection, to: Self.board("b"), in: .empty, defaults: defaults)
        #expect(selection == Self.board("b"))
    }

    @Test("It's kept across a relaunch: the next window reads it from the same defaults")
    func keptAcrossRelaunch() {
        let defaults = Self.defaults()
        WorkspacePlaces.remember(Self.task("a", "t1"), in: defaults)
        WorkspacePlaces.remember(Self.board("b"), in: defaults)
        #expect(
            WorkspacePlaces.restoring(from: Self.board("b"), to: Self.board("a"), in: .empty, defaults: defaults)
                == Self.task("a", "t1"))
        // Needs You and a loose worktree are no workspace's place.
        WorkspacePlaces.remember(.needsYou, in: defaults)
        WorkspacePlaces.remember(.looseWorktree(host: "", worktree: "w", terminal: nil), in: defaults)
        #expect(
            WorkspacePlaces.restoring(from: .needsYou, to: Self.board("a"), in: .empty, defaults: defaults)
                == Self.task("a", "t1"))
    }

    @Test("History records the switch as one step from where it began, to the place opened")
    func historyHasOneStep() {
        let defaults = Self.defaults()
        var places = PlaceSwitch(), history = NavigationHistory()
        var selection: Selection?
        Self.drive(&places, &history, selection: &selection, to: Self.task("a", "t1"), defaults: defaults)
        Self.drive(&places, &history, selection: &selection, to: Self.task("b", "t2"), defaults: defaults)
        Self.drive(&places, &history, selection: &selection, to: Self.board("a"), defaults: defaults)
        #expect(selection == Self.task("a", "t1"))
        let back = history.goBack(from: selection, resolves: { _ in true })
        #expect(back?.place == Self.task("b", "t2"))
    }
}
