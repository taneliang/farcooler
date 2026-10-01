import AppKit
import CoreGraphics
import SwiftUI
import Testing

@testable import Far_Cooler

/// A workspace's levels at the widths 2A measured (spec §4.3). The detail is
/// the window less a 248 pt sidebar: 1222 pt at a full-screen 1470, 1192 at
/// 1440, and 1032 in a 1280 pt window.
struct WorkspaceColumnsTests {
    private typealias Columns = WorkspaceColumns

    /// The conversation's minimum is the narrowest width holding its
    /// columns: one point less loses one. That's 48 at the default font.
    @Test("The conversation's minimum holds its measured columns and no more")
    func theMinimumHoldsItsMeasuredColumns() {
        func columns(_ width: CGFloat) -> Int { Int((width - Columns.chrome) / Columns.defaultCell) }
        #expect(columns(Columns.conversationMinimum()) == 48)
        #expect(columns(Columns.conversationMinimum() - 1) == 47)
        // At 13 pt it's 426.
        let thirteen: CGFloat = 8.0361328125
        #expect(Columns.conversationMinimum(cell: thirteen) == 426)
        let two = Columns.conversationMinimum(cell: thirteen) + Columns.boardMinimum + 1
        #expect(Columns.layout(width: two, drilled: false, cell: thirteen) == .two)
        #expect(Columns.layout(width: two - 1, drilled: false, cell: thirteen) == .one)
    }

    /// Never four columns (ov-79): a task or a worktree opened takes the
    /// detail at every width, from the measured full screen down to the
    /// window's 600 pt minimum, where the detail is 352 pt, with the
    /// conversation a rail beside it and the board out of sight.
    @Test("Opening a task drills in at every width, with the conversation as a rail")
    func openingATaskDrillsInAtEveryWidth() {
        for width: CGFloat in [1600, 1222, 1192, 1032, 807, 600, 352] {
            let drilled = Columns.layout(width: width, drilled: true)
            #expect(drilled == .drilledIn, "at \(width)")
            #expect(!drilled.board && drilled.drilled && drilled.conversation == .rail)
        }
        #expect(Columns.layout(width: 1032, drilled: true, peek: true) == .peeked)
        // Focus is what's opened alone, without the rail, popped open or not.
        #expect(Columns.layout(width: 1222, drilled: true, focused: true) == .drilledAlone)
        #expect(Columns.layout(width: 1222, drilled: true, focused: true, peek: true) == .drilledAlone)
        // The popped-open conversation is its minimum, or what the rail leaves.
        #expect(Columns.peekWidth(in: 1032) == Columns.conversationMinimum())
        #expect(Columns.peekWidth(in: 300) == 300 - Columns.rail - Columns.divider)
    }

    /// Down to the window's 600 pt minimum, where the detail is 352 pt.
    @Test("Below the two-column minimum it's one column with Orchestrator | Board")
    func belowTheTwoColumnMinimumItsOneColumn() {
        let two = Columns.conversationMinimum() + Columns.boardMinimum + 1
        #expect(Columns.layout(width: 1032, drilled: false) == .two)
        #expect(Columns.layout(width: two, drilled: false) == .two)
        #expect(Columns.layout(width: two - 1, drilled: false) == .one)
        #expect(Columns.layout(width: 352, drilled: false).switcher)
        // A workspace with no conversation, on a runner without workstreams:
        // the board, or what's opened, alone.
        #expect(Columns.layout(width: 352, drilled: false, hasConversation: false) == .boardAlone)
        #expect(Columns.layout(width: 1000, drilled: true, hasConversation: false) == .drilledAlone)
        #expect(Columns.layout(width: 1000, drilled: true, hasConversation: false, peek: true) == .drilledAlone)
    }

    /// The drawn view, hosted in a detail 600 pt wide inside a 1600 pt
    /// window: it lays out by its own width, so the workspace level is one
    /// column with Orchestrator | Board. A view that read the window would
    /// draw both and clip. Drilled in, it's a rail and what's opened.
    @MainActor
    @Test("The workspace lays out by its own width, not the window's")
    func theWorkspaceLaysOutByItsOwnWidth() async {
        final class Seen { var arrangement: WorkspaceColumns.Arrangement? }
        let seen = Seen()
        struct Hosted: View {
            let seen: Seen
            let drilled: Bool
            var body: some View {
                WorkspaceView(
                    drilled: drilled, hasConversation: true, cell: WorkspaceColumns.defaultCell, focused: false,
                    peek: false, pick: .constant(.orchestrator),
                    conversation: { Color.clear }, rail: { Color.clear }, board: { Color.clear },
                    opened: { Color.clear })
                .frame(width: 600, height: 400)
                .frame(width: 1600, height: 400, alignment: .leading)
                .onPreferenceChange(WorkspaceArrangementPreference.self) { value in
                    MainActor.assumeIsolated { seen.arrangement = value }
                }
            }
        }
        for (drilled, expected) in [(true, WorkspaceColumns.Arrangement.drilledIn), (false, .one)] {
            seen.arrangement = nil
            let host = NSHostingView(rootView: Hosted(seen: seen, drilled: drilled))
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 1600, height: 400), styleMask: [.borderless],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = host
            for _ in 0..<5 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
            window.close()
            #expect(seen.arrangement == expected)
        }
    }

    /// On screen means drawn: a conversation shrunk to its rail, left out
    /// in Focus, or behind Board in the one-column form isn't, so it isn't
    /// marked seen or watched and the keyboard doesn't act on it. Popped
    /// open over a task, it is. Nothing is, before the detail has been
    /// measured, and no task is at the workspace's own level.
    @Test("A conversation on its rail or hidden isn't on screen")
    func aRailedOrHiddenConversationIsntOnScreen() {
        let worktree = Worktree(
            id: "w", short: "w", task: "w", branch: "b", repository: nil, host: "", path: "/tmp/w",
            state: "active", terminals: [])
        func layout(_ column: ShownLayout.Column, _ id: String) -> ShownLayout {
            let group = PaneGroup(id: id, name: "", active: true, columns: 80, rows: 24, layout: id, panes: [])
            return ShownLayout(column: column, worktree: worktree, group: group, groups: [group])
        }
        let shown = [layout(.conversation, "@1"), layout(.task, "@2")]
        func columns(_ arrangement: WorkspaceColumns.Arrangement?, _ pick: WorkspacePick = .orchestrator) -> [ShownLayout.Column] {
            WorkspaceScreen.visible(shown, arrangement: arrangement, pick: pick).map(\.column)
        }
        #expect(columns(.peeked) == [.conversation, .task])
        #expect(columns(.drilledIn) == [.task])
        #expect(columns(.drilledAlone) == [.task])
        #expect(columns(.two) == [.conversation])
        #expect(columns(.one, .orchestrator) == [.conversation])
        #expect(columns(.one, .board) == [])
        #expect(columns(nil) == [])
    }

    /// ⌃H ⌃J ⌃K ⌃L traverse only while the layout the keyboard is in has
    /// somewhere to go. With a one-pane conversation beside a three-pane
    /// task, the count follows the key pane's column, not whichever terminal
    /// view appeared last.
    @Test("Pane keys follow the column the keyboard is in")
    func paneKeysFollowTheColumnTheKeyboardIsIn() {
        let worktree = Worktree(
            id: "w", short: "w", task: "w", branch: "b", repository: nil, host: "", path: "/tmp/w",
            state: "active", terminals: [])
        func layout(_ column: ShownLayout.Column, _ id: String, _ panes: [String]) -> ShownLayout {
            let rects = panes.map {
                PaneRect(id: $0, short: $0, title: nil, left: 0, top: 0, columns: 80, rows: 24, focused: false, zoomed: false)
            }
            let group = PaneGroup(id: id, name: "", active: true, columns: 80, rows: 24, layout: id, panes: rects)
            return ShownLayout(column: column, worktree: worktree, group: group, groups: [group])
        }
        let shown = [layout(.conversation, "@1", ["conductor"]), layout(.task, "@2", ["a", "b", "c"])]
        func key(_ terminal: String) -> PaneRef { PaneRef(host: "", worktree: "w", terminal: terminal) }
        #expect(WorkspaceScreen.tiledPanes(key("conductor"), in: shown) == 1)
        #expect(WorkspaceScreen.tiledPanes(key("b"), in: shown) == 3)
        #expect(WorkspaceScreen.tiledPanes(key("gone"), in: shown) == 0)
        #expect(WorkspaceScreen.tiledPanes(nil, in: shown) == 0)
    }
}
