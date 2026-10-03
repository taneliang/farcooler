import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Back and forward through where the window has been (ov-192): places, not
/// panes, each with the task it was opened from; a step's own landing isn't
/// a new step; a place that's gone is passed over.
struct NavigationHistoryTests {
    private typealias Selection = ContentView.Selection
    private typealias Stop = NavigationHistory.Stop

    private static let ws = "billing"
    private static var orchestrator: Selection { .workspace(host: "", workspace: ws, focus: nil) }
    private static func task(_ id: String) -> Selection { .workspace(host: "", workspace: ws, focus: .task(id)) }
    private static func worktree(_ id: String, _ terminal: String? = nil) -> Selection {
        .workspace(host: "", workspace: ws, focus: .worktree(id, terminal: terminal))
    }

    /// Records each change in `path`, as the window's `onChange` does.
    private static func walked(_ path: [Selection]) -> NavigationHistory {
        var history = NavigationHistory()
        for (old, new) in zip(path, path.dropFirst()) { history.record(from: old, to: new) }
        return history
    }

    private static func always(_: Selection) -> Bool { true }

    @Test("Back goes where you were, then Forward returns, each step's landing unrecorded")
    func backAndForward() {
        var history = Self.walked([Self.orchestrator, Self.task("a"), Self.task("b"), Self.worktree("pdf")])
        #expect(history.back.map(\.place) == [Self.orchestrator, Self.task("a"), Self.task("b")])
        let back = history.goBack(from: Self.worktree("pdf"), resolves: Self.always)?.place
        #expect(back == Self.task("b"))
        history.record(from: Self.worktree("pdf"), to: back)
        let again = history.goBack(from: back, resolves: Self.always)?.place
        #expect(again == Self.task("a"))
        history.record(from: back, to: again)
        #expect(history.forward.map(\.place) == [Self.worktree("pdf"), Self.task("b")])
        let forward = history.goForward(from: again, resolves: Self.always)?.place
        #expect(forward == Self.task("b"))
        history.record(from: again, to: forward)
        #expect(history.back.map(\.place) == [Self.orchestrator, Self.task("a")])
        #expect(history.forward.map(\.place) == [Self.worktree("pdf")])
        #expect(history.canGoForward)
        _ = history.goForward(from: forward, resolves: Self.always)
        #expect(!history.canGoForward)
    }

    @Test("Another pane in the same worktree is no step, and it's kept as the place")
    func panesAreNoStep() {
        let history = Self.walked([
            Self.worktree("pdf", "t1"), Self.worktree("pdf", "t2"), Self.worktree("pdf"), Self.task("a"),
        ])
        #expect(history.back == [Stop(Self.worktree("pdf"))])
        // Back into the worktree, then another pane in it: Forward stays.
        var back = history
        let landed = back.goBack(from: Self.task("a"), resolves: Self.always)?.place
        back.record(from: Self.task("a"), to: landed)
        back.record(from: landed, to: Self.worktree("pdf", "t3"))
        #expect(back.forward == [Stop(Self.task("a"))])
    }

    @Test("A worktree opened from its task comes back with the task, both ways")
    func keepsTheTask() {
        var history = NavigationHistory()
        // Task → Open Worktree → another task.
        history.record(from: Self.task("a"), to: Self.worktree("pdf"))
        history.record(from: Self.worktree("pdf"), to: Self.task("b"), trail: Self.task("a"))
        let back = history.goBack(from: Self.task("b"), resolves: Self.always)
        #expect(back == Stop(Self.worktree("pdf"), trail: Self.task("a")))
        history.record(from: Self.task("b"), to: back?.place)
        // Back again to the task, from the worktree still opened from it.
        _ = history.goBack(from: back?.place, trail: Self.task("a"), resolves: Self.always)
        let forward = history.goForward(from: Self.task("a"), resolves: Self.always)
        #expect(forward == Stop(Self.worktree("pdf"), trail: Self.task("a")))
    }

    @Test("⌃⌘←: Focus is left first; else where you were; with nowhere, up a level")
    func backRoute() {
        var history = Self.walked([Self.task("a"), Self.task("b")])
        #expect(history.back(focus: true, from: Self.task("b"), trail: nil, resolves: Self.always) == .upALevel)
        #expect(history.canGoBack)
        #expect(history.back(focus: false, from: Self.task("b"), trail: nil, resolves: Self.always) == .history(Stop(Self.task("a"))))
        var empty = NavigationHistory()
        #expect(empty.back(focus: false, from: Self.task("b"), trail: nil, resolves: Self.always) == .upALevel)
    }

    @Test("Going somewhere new clears Forward")
    func newClearsForward() {
        var history = Self.walked([Self.orchestrator, Self.task("a"), Self.task("b")])
        let back = history.goBack(from: Self.task("b"), resolves: Self.always)?.place
        history.record(from: Self.task("b"), to: back)
        #expect(history.canGoForward)
        history.record(from: back, to: Self.task("c"))
        #expect(!history.canGoForward)
        #expect(history.back.map(\.place) == [Self.orchestrator, Self.task("a")])
    }

    @Test("A place that's gone is passed over; with none left, nothing")
    func goneIsPassedOver() {
        var history = Self.walked([Self.task("a"), Self.worktree("gone"), Self.task("b")])
        let back = history.goBack(from: Self.task("b"), resolves: { $0 != Self.worktree("gone") })
        #expect(back?.place == Self.task("a"))
        var empty = NavigationHistory()
        #expect(empty.goBack(from: Self.task("a"), resolves: Self.always) == nil)
        #expect(empty.goForward(from: Self.task("a"), resolves: Self.always) == nil)
    }

    @Test("It keeps the last fifty")
    func capped() {
        let path = (0...60).map { Self.task("t\($0)") }
        let history = Self.walked(path)
        #expect(history.back.count == NavigationHistory.limit)
        #expect(history.back.first?.place == Self.task("t10"))
    }

    @Test("A place resolves while its workspace and worktree are there")
    func resolves() {
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [], branchPrefix: nil)
        fleet.runnerWorkspaces[""] = [
            WorkspaceSummary(id: Self.ws, name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: "r")
        ]
        let none: (String) -> [String] = { _ in [] }
        #expect(NavigationHistory.resolves(Self.task("a"), in: fleet, repositories: none))
        #expect(!NavigationHistory.resolves(Self.worktree("pdf"), in: fleet, repositories: none))
        #expect(!NavigationHistory.resolves(.workspace(host: "", workspace: "gone", focus: nil), in: fleet, repositories: none))
        #expect(NavigationHistory.resolves(.needsYou, in: fleet, repositories: none))
    }

    @Test("Forward greys out with nothing ahead, Go to Jump Bar with no bar, and both with something over the window")
    func menuEnablement() {
        func goes(_ can: KeyPath<MainWindowFocus, Bool>, _ focus: MainWindowFocus?) -> Bool {
            MainWindowFocus.goes(can, focus)
        }
        #expect(!goes(\.goesForward, MainWindowFocus(overlayOpen: false)))
        #expect(goes(\.goesForward, MainWindowFocus(overlayOpen: false, goesForward: true)))
        #expect(!goes(\.goesForward, MainWindowFocus(overlayOpen: true, goesForward: true)))
        #expect(!goes(\.hasJumpBar, MainWindowFocus(overlayOpen: false)))
        #expect(goes(\.hasJumpBar, MainWindowFocus(overlayOpen: false, hasJumpBar: true)))
        #expect(!goes(\.hasJumpBar, nil))
    }
}
