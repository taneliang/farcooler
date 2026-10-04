import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The history a long press on Back or Forward lists (ov-248): the whole
/// session in one list, Firefox's way, and a row chosen in one move.
struct NavigationHistoryMenuTests {
    fileprivate typealias Selection = ContentView.Selection
    fileprivate typealias Stop = NavigationHistory.Stop

    fileprivate static func task(_ id: String) -> Selection { .workspace(host: "", workspace: "billing", focus: .task(id)) }
    private static func always(_: Selection) -> Bool { true }

    fileprivate static func walked(_ path: [Selection]) -> NavigationHistory {
        var history = NavigationHistory()
        for (old, new) in zip(path, path.dropFirst()) { history.record(from: old, to: new) }
        return history
    }

    @Test("Jumping three back leaves the two skipped places and this one on Forward, in order")
    func jumpBack() {
        // a b c d e, now at f.
        var history = Self.walked(["a", "b", "c", "d", "e", "f"].map(Self.task))
        let landed = history.go(toBack: 2, from: Self.task("f"), trail: nil)
        #expect(landed == Stop(Self.task("c")))
        #expect(history.back.map(\.place) == ["a", "b"].map(Self.task))
        // Forward's next is d, then e, then f.
        #expect(history.forward.map(\.place) == ["f", "e", "d"].map(Self.task))
        // The landing isn't a new step.
        history.record(from: Self.task("f"), to: Self.task("c"))
        #expect(history.forward.count == 3)
    }

    @Test("Jumping forward is the same move the other way")
    func jumpForward() {
        var history = Self.walked(["a", "b", "c", "d", "e", "f"].map(Self.task))
        _ = history.go(toBack: 3, from: Self.task("f"), trail: nil)
        history.record(from: Self.task("f"), to: Self.task("b"))
        // At b, with c d e f ahead: go to e.
        let landed = history.go(toForward: 2, from: Self.task("b"), trail: nil)
        #expect(landed == Stop(Self.task("e")))
        #expect(history.back.map(\.place) == ["a", "b", "c", "d"].map(Self.task))
        #expect(history.forward.map(\.place) == [Self.task("f")])
    }

    @Test("A jump past the end, or from nowhere, does nothing")
    func jumpOutOfRange() {
        var history = Self.walked(["a", "b"].map(Self.task))
        let before = history
        #expect(history.go(toBack: 5, from: Self.task("b"), trail: nil) == nil)
        #expect(history.go(toForward: 0, from: Self.task("b"), trail: nil) == nil)
        #expect(history == before)
    }

    @Test("The list is Forward farthest first, this place, then Back nearest first")
    func rowOrder() {
        var history = Self.walked(["a", "b", "c", "d", "e"].map(Self.task))
        _ = history.go(toBack: 1, from: Self.task("e"), trail: nil)
        history.record(from: Self.task("e"), to: Self.task("c"))
        // Back: a b, now c, Forward: d e (d next).
        let rows = history.rows(current: Self.task("c"), trail: nil, resolves: Self.always)
        #expect(rows.map(\.stop.place) == ["e", "d", "c", "b", "a"].map(Self.task))
        #expect(rows.map(\.spot) == [.forward(1), .forward(0), .current, .back(0), .back(1)])
    }

    @Test("Fifteen a side, and a known-gone place is left out without moving the others' distances")
    func rowsCappedAndFiltered() {
        let path = (0...40).map { Self.task("t\($0)") }
        let history = Self.walked(path)
        let rows = history.rows(current: path.last, trail: nil, resolves: Self.always)
        #expect(rows.filter { if case .back = $0.spot { true } else { false } }.count == 15)
        #expect(rows.count == 16)

        let gone = Self.walked(["a", "gone", "c"].map(Self.task))
        let kept = gone.rows(current: Self.task("c"), trail: nil) { $0 != Self.task("gone") }
        #expect(kept.map(\.stop.place) == [Self.task("c"), Self.task("a")])
        // a is the second back stop, whatever's left out before it.
        #expect(kept.last?.spot == .back(1))
    }

    @Test("A row's place is a step of its own, not the place it left")
    func trailsKept() {
        var history = NavigationHistory()
        history.record(from: Self.task("a"), to: Self.task("b"), trail: Self.task("z"))
        let rows = history.rows(current: Self.task("b"), trail: Self.task("y"), resolves: Self.always)
        #expect(rows.first?.stop == Stop(Self.task("b"), trail: Self.task("y")))
        #expect(rows.last?.stop == Stop(Self.task("a"), trail: Self.task("z")))
    }
}

extension NavigationHistoryMenuTests {
    @Test("A row on either side is a jump to it; the place it's at is none")
    func jumpBySpot() {
        var history = Self.walked(["a", "b", "c", "d"].map(Self.task))
        #expect(history.go(to: .current, from: Self.task("d"), trail: nil) == nil)
        #expect(history.go(to: .back(1), from: Self.task("d"), trail: nil) == Stop(Self.task("b")))
        history.record(from: Self.task("d"), to: Self.task("b"))
        #expect(history.go(to: .forward(1), from: Self.task("b"), trail: nil) == Stop(Self.task("d")))
    }

    @Test("A task its board has read and lacks is passed over; one on a board not read yet isn't")
    func goneTaskResolves() {
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [], branchPrefix: nil)
        fleet.runnerWorkspaces[""] = [
            WorkspaceSummary(id: "billing", name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: "r")
        ]
        let row = TaskRow(id: "here", key: "bil-1", title: "Kept", status: .todo, statusSince: Date(timeIntervalSince1970: 0))
        let read = TaskBoardModel(columns: [TaskBoardColumn(status: .todo, rows: [row])])
        let none: (String) -> [String] = { _ in [] }
        func resolves(_ id: String, _ board: TaskBoardModel?) -> Bool {
            NavigationHistory.resolves(Self.task(id), in: fleet, repositories: none, board: { _, _ in board })
        }
        #expect(resolves("here", read))
        #expect(!resolves("gone", read))
        #expect(resolves("gone", .empty), "a board with no rows isn't read yet")
        #expect(resolves("gone", nil), "no board in the window says nothing")
    }

    @Test("Workspace ▸ History is there for a window with somewhere to go, and not over an overlay")
    func menuBarItem() {
        let rows = [
            PlaceRow(spot: .current, title: "A", subtitle: nil, symbol: "tray"),
            PlaceRow(spot: .back(0), title: "B", subtitle: nil, symbol: "tray"),
        ]
        #expect(MainWindowFocus.goes(\.goesHistory, MainWindowFocus(overlayOpen: false, history: rows)))
        #expect(!MainWindowFocus.goes(\.goesHistory, MainWindowFocus(overlayOpen: false, history: [rows[0]])))
        #expect(!MainWindowFocus.goes(\.goesHistory, MainWindowFocus(overlayOpen: true, history: rows)))
        #expect(!MainWindowFocus.goes(\.goesHistory, nil))
    }
}
