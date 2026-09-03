import AppKit
import Foundation
import Testing

@testable import Far_Cooler

/// A pane you come back to is not a black rectangle.
///
/// Switching workspaces destroys every `TerminalRenderView` in the layout you
/// left — the panes are keyed by terminal, so a different layout is a different
/// set of ids — and the emulator is the only thing that holds what the terminal
/// looked like. Kept, the pane draws its last frame in the turn it mounts;
/// dropped, it draws nothing until a `farcooler terminal stream` process has
/// started, opened the database and captured four things from tmux. Measured on
/// this machine's own runner: 16ms against 176-241ms.
///
/// None of that is visible from a still — a pane that has just finished its
/// replay looks exactly like a pane that never lost it — which is why these are
/// tests and not a screenshot.
@MainActor
struct RetainedScreenTests {
    /// A view at a real pane size, with something on it.
    private func view(columns: Int = 40, rows: Int = 10) -> TerminalRenderView {
        let v = TerminalRenderView()
        v.setPaneGrid(PaneGrid(columns: columns, rows: rows))
        return v
    }

    private func text(of view: TerminalRenderView) -> String {
        view.core.withSnapshot { snapshot in
            (0..<snapshot.rows)
                .map { row in
                    String(
                        (0..<snapshot.columns).map { snapshot[row, $0].character ?? " " }
                    ).trimmingCharacters(in: .whitespaces)
                }
                .joined(separator: "\n")
                .trimmingCharacters(in: .whitespacesAndNewlines)
        } ?? ""
    }

    @Test func a_view_that_mounts_onto_a_kept_screen_shows_it_at_once() {
        let leaving = view()
        leaving.feed(Array("hello from before".utf8))
        #expect(text(of: leaving).contains("hello from before"))

        let store = TerminalScreens.shared
        store.keep("t1", core: leaving.core)

        let arriving = view()
        #expect(text(of: arriving).isEmpty, "a fresh view starts blank")
        guard let kept = store.take("t1") else {
            Issue.record("the screen was not kept")
            return
        }
        arriving.showRetained(kept)

        // Nothing has been fed, nothing has been asked of a runner, and the
        // pane is already showing what it showed before.
        #expect(text(of: arriving).contains("hello from before"))
    }

    /// The bug that makes the kept frame worth having a second core for.
    ///
    /// A replay opens with the pane's scrollback. Fed into an emulator that
    /// already holds that scrollback, the terminal ends up with two copies of
    /// the same history and scrolling up walks through it twice. So the replay
    /// fills a core of its own and the view swaps onto it on the first byte.
    @Test func a_replay_lands_on_a_fresh_core_and_not_on_the_kept_one() {
        // Twelve lines into a ten-row pane: two of them are history.
        let history = (1...12).map { "line \($0)\r\n" }.joined()

        let cold = view()
        cold.feed(Array(history.utf8))
        let coldHistory = cold.core.withSnapshot { $0.historySize } ?? 0
        #expect(coldHistory > 0, "the fixture has to produce scrollback to be about anything")

        let leaving = view()
        leaving.feed(Array(history.utf8))
        let store = TerminalScreens.shared
        store.keep("t2", core: leaving.core)

        let arriving = view()
        arriving.showRetained(store.take("t2")!)
        // What `TerminalSurface.attach` does when it opens the stream, and then
        // the replay arriving on it.
        arriving.beginLiveCore()
        arriving.feed(Array(history.utf8))

        let after = arriving.core.withSnapshot { $0.historySize } ?? 0
        #expect(
            after == coldHistory,
            "the replay was applied on top of the kept frame: \(after) lines of history where a cold pane fed the same bytes has \(coldHistory)"
        )
        #expect(text(of: arriving).contains("line 12"))
    }

    /// The other half of that rule: a pane that mounted cold has an empty core
    /// and the replay belongs in it. Building a second one and swapping would
    /// throw away bytes that had already arrived in the first.
    @Test func a_cold_view_keeps_the_core_it_mounted_with() {
        let cold = view()
        let mounted = cold.core
        cold.beginLiveCore()
        cold.feed(Array("first bytes".utf8))
        #expect(cold.core === mounted, "a cold pane swapped cores it had no reason to")
        #expect(text(of: cold).contains("first bytes"))
    }

    /// A kept frame was last drawn in whatever pane it left, which may not be
    /// the shape of the pane it is coming back to.
    @Test func a_kept_screen_is_reflowed_to_the_pane_it_returns_to() {
        let leaving = view(columns: 40, rows: 10)
        leaving.feed(Array("hello".utf8))
        let store = TerminalScreens.shared
        store.keep("t3", core: leaving.core)

        let arriving = view(columns: 80, rows: 24)
        arriving.showRetained(store.take("t3")!)
        #expect(arriving.grid == PaneGrid(columns: 80, rows: 24))
    }

    /// The same rule, for the case that has nothing to do with switching panes.
    ///
    /// A pane re-attaches when its runner's link is replaced — same terminal,
    /// same view, a second replay. That replay opens with the pane's scrollback
    /// too, and the view it lands in is the one that has been drawing the pane
    /// all along, so it has that history already.
    @Test func a_reconnect_does_not_append_the_pane_s_history_a_second_time() {
        let history = (1...12).map { "line \($0)\r\n" }.joined()

        let cold = view()
        cold.feed(Array(history.utf8))
        let once = cold.core.withSnapshot { $0.historySize } ?? 0
        #expect(once > 0)

        // What a re-attach does: no kept frame to take, a stream opened on the
        // view that is already showing this terminal, and the replay arriving.
        cold.beginLiveCore()
        cold.feed(Array(history.utf8))

        let twice = cold.core.withSnapshot { $0.historySize } ?? 0
        #expect(
            twice == once,
            "a reconnect left \(twice) lines of history where the pane has \(once)"
        )
    }

    @Test func the_store_holds_its_budget_and_drops_the_least_recently_kept() {
        let store = TerminalScreens.shared
        let ids = (0...TerminalScreens.budget).map { "budget-\($0)" }
        for id in ids { store.keep(id, core: VTCore(columns: 8, rows: 2)) }
        // The count itself, not just which ids survived: an eviction that fired
        // but dropped the wrong one would leave the right number behind.
        #expect(store.count <= TerminalScreens.budget)
        #expect(!store.holds(ids[0]), "the oldest screen was kept past the budget")
        #expect(store.holds(ids[ids.count - 1]), "the newest screen was evicted")
    }

    @Test func taking_a_screen_hands_over_ownership() {
        let store = TerminalScreens.shared
        store.keep("t4", core: VTCore(columns: 8, rows: 2))
        #expect(store.take("t4") != nil)
        #expect(store.take("t4") == nil, "a screen was handed to two views at once")
    }
}
