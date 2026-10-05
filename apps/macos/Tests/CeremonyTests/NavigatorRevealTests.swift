import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The selected row stays in sight in its pane (ov-295): found by integ-12,
/// where the selected task sat scrolled out of view under In Progress once
/// ov-292 sized the panes to the window. Rows are looked for by their probes
/// in a window drawn offscreen; no input is sent.
@MainActor
struct NavigatorRevealTests {
    /// The tasks as the list draws them, and the key of one.
    @MainActor private struct Board {
        let store: TaskBoardStore
        let defaults = UserDefaults(suiteName: "reveal-\(UUID().uuidString)")!
        var tasks: [String] = []

        init() async {
            store = await NavigatorSplitTests.store(tasks: 120)
            tasks = BoardKeys.rows(
                store.board,
                collapsed: BoardForm.collapsed(host: store.hostKey, workspace: store.workspace.id, from: defaults),
                reads: store.reads, keeping: nil, showingMore: [], filtering: false, now: Date())
        }

        func view(_ current: NavigatorItem?, keyed: Bool? = nil) -> TaskBoardView {
            NavigatorSplitTests.view(store, terminals: 30, defaults: defaults, current: current, keyed: keyed)
        }

        func key(_ id: String) -> String { store.board.rows.first { $0.id == id }?.key ?? "" }
    }

    /// Whether the row probed `row` lies inside the pane probed `pane`.
    private func shows(_ drawn: NavigatorSplitTests.Drawn, _ row: String, in pane: String) throws -> Bool {
        let viewport = try #require(drawn.probe("navigator-pane-\(pane)"), "no \(pane) pane")
        let frame = try #require(drawn.box.probes.first { $0.0 == row }?.1, "no row \(row)")
        // Half a point of slack for the pane's edge rounding, far under a row's height.
        return viewport.insetBy(dx: 0, dy: -0.5).contains(CGPoint(x: viewport.midX, y: frame.midY))
    }

    @Test("A task selected at launch, however far down, is scrolled into its pane")
    func launchShowsTheSelection() async throws {
        let board = await Board()
        let last = try #require(board.tasks.last)
        let drawn = NavigatorSplitTests.Drawn(board.view(.task(last), keyed: false), height: 800)
        await drawn.settle()
        #expect(try shows(drawn, "board-row-\(board.key(last))", in: "tasks"))
    }

    @Test("A task selected from elsewhere, with no keyboard on the list, is scrolled into view")
    func selectionShowsWithoutKeyboard() async throws {
        let board = await Board()
        let drawn = NavigatorSplitTests.Drawn(board.view(.task(board.tasks[0]), keyed: false), height: 800)
        await drawn.settle()
        let far = try #require(board.tasks.dropLast(2).last)
        drawn.show(board.view(.task(far), keyed: false))
        await drawn.settle()
        #expect(try shows(drawn, "board-row-\(board.key(far))", in: "tasks"))
    }

    @Test("A window made shorter keeps the selected row in its pane")
    func resizeKeepsTheSelection() async throws {
        let board = await Board()
        let last = try #require(board.tasks.last)
        let drawn = NavigatorSplitTests.Drawn(board.view(.task(last), keyed: false), height: 900)
        await drawn.settle()
        for height: CGFloat in [600, 450] {
            drawn.window.setContentSize(NSSize(width: drawn.window.frame.width, height: height))
            drawn.show(board.view(.task(last), keyed: false))
            await drawn.settle()
            #expect(try shows(drawn, "board-row-\(board.key(last))", in: "tasks"), "gone at \(height) pt")
        }
    }

    @Test("A terminal selected in a short window with Worktrees closed is in its pane")
    func closedSectionShowsTheSelection() async throws {
        let store = await NavigatorSplitTests.store(tasks: 120)
        let view = NavigatorSplitTests.view(
            store, terminals: 30, closed: ["worktrees"], current: .terminal("term29"), keyed: false)
        let drawn = NavigatorSplitTests.Drawn(view, height: 500)
        await drawn.settle()
        #expect(try shows(drawn, "navigator-terminal-term29", in: "terminals"))
    }

    @Test("A row already in view never moves the pane")
    func visibleRowStaysPut() async throws {
        let board = await Board()
        let drawn = NavigatorSplitTests.Drawn(board.view(.task(board.tasks[1]), keyed: false), height: 800)
        await drawn.settle()
        let scroll = try #require(drawn.scrollViews().first)
        #expect(scroll.contentView.bounds.origin.y == 0)
        drawn.window.setContentSize(NSSize(width: drawn.window.frame.width, height: 900))
        drawn.show(board.view(.task(board.tasks[2]), keyed: false))
        await drawn.settle()
        #expect(try #require(drawn.scrollViews().first).contentView.bounds.origin.y == 0, "the pane jumped")
    }
}
