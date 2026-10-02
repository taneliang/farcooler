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
    /// What's opened keeps 58 the same way, and both grow with the font.
    @Test("The minimums hold their measured columns and no more")
    func theMinimumsHoldTheirMeasuredColumns() {
        func columns(_ width: CGFloat) -> Int { Int((width - Columns.chrome) / Columns.defaultCell) }
        #expect(columns(Columns.conversationMinimum()) == 48)
        #expect(columns(Columns.conversationMinimum() - 1) == 47)
        #expect(columns(Columns.openedMinimum()) == 58)
        #expect(columns(Columns.openedMinimum() - 1) == 57)
        #expect(Columns.openedMinimum() == 489)
        // At 13 pt the conversation is 426, and what's opened needs a wider
        // detail to keep the board beside it.
        let thirteen: CGFloat = 8.0361328125
        #expect(Columns.conversationMinimum(cell: thirteen) == 426)
        let beside = Columns.rail + 1 + Columns.boardMinimum + 1 + Columns.openedMinimum(cell: thirteen)
        #expect(Columns.layout(width: beside, opened: true, cell: thirteen).board)
        #expect(!Columns.layout(width: beside - 1, opened: true, cell: thirteen).board)
    }

    /// The chain at every width the spec measured, from a full-screen 1470
    /// pt window's 1222 pt detail down to the 600 pt window's 352: the rail,
    /// then the board filling the rest with nothing open.
    @Test("With nothing open, the board fills the content beside the rail", arguments: [1600, 1222, 1192, 1032, 778, 600, 352] as [CGFloat])
    func theBoardFillsTheContent(width: CGFloat) {
        let arrangement = Columns.layout(width: width, opened: false)
        #expect(arrangement == .workspace)
        let frames = Columns.frames(width: width, arrangement: arrangement, list: 300)
        #expect(frames.content == 29)
        #expect(frames.board == width - 29)
        // Popped open, the conversation is over the board, which stays put.
        let peeked = Columns.layout(width: width, opened: false, peek: true)
        #expect(peeked == .peeked)
        #expect(Columns.frames(width: width, arrangement: peeked, list: 300) == frames)
        // No conversation: no rail, and the board from the edge.
        let alone = Columns.layout(width: width, opened: false, hasConversation: false)
        #expect(alone == .boardAlone)
        #expect(Columns.frames(width: width, arrangement: alone, list: 300).board == width)
    }

    /// Opening a task narrows the board to the list and puts the task
    /// beside it, with the widest pane the work's: at a 13-inch laptop's
    /// widths, a 300 pt list and 700–900 pt for the task. Where both don't
    /// fit at their minimums, the task covers the board instead.
    @Test("A task open narrows the board to a list beside it, or covers it")
    func aTaskOpenNarrowsTheBoard() {
        func at(_ width: CGFloat, list: CGFloat = 300, focused: Bool = false, conversation: Bool = true)
            -> (Columns.Arrangement, Columns.Frames)
        {
            let arrangement = Columns.layout(width: width, opened: true, hasConversation: conversation, focused: focused)
            return (arrangement, Columns.frames(width: width, arrangement: arrangement, list: list))
        }
        // Full screen on an M2 Air, an M1, and a 1280 pt window.
        for (width, opened) in [(1222, 892), (1192, 862), (1032, 702)] as [(CGFloat, CGFloat)] {
            let (arrangement, frames) = at(width)
            #expect(arrangement == .beside, "at \(width)")
            #expect(frames.board == 300, "at \(width)")
            #expect(frames.openedX == 330, "at \(width)")
            #expect(frames.opened == opened, "at \(width)")
            #expect(frames.openedX + frames.opened == width)
        }
        // The narrowest that keeps both: the list gives way to its minimum
        // first, so what's opened keeps its 58 columns.
        let edge = 29 + Columns.boardMinimum + 1 + Columns.openedMinimum()
        #expect(edge == 779)
        let (kept, keptFrames) = at(edge)
        #expect(kept == .beside)
        #expect(keptFrames.board == Columns.boardMinimum && keptFrames.opened == Columns.openedMinimum())
        // A point narrower, what's opened covers the board, which keeps
        // its width underneath rather than reflowing.
        for width in [edge - 1, 600, 352] {
            let (covering, frames) = at(width)
            #expect(covering == .covering, "at \(width)")
            #expect(frames.openedX == 29 && frames.opened == width - 29, "at \(width)")
            #expect(frames.board == Columns.boardMinimum, "at \(width)")
        }
        // A list dragged wide stops where what's opened would lose a column;
        // dragged narrow, at its minimum.
        #expect(at(1222, list: 900).1.board == 1222 - 29 - 1 - Columns.openedMinimum())
        #expect(at(1222, list: 100).1.board == Columns.boardMinimum)
        #expect(at(1222, list: 420).1.board == 420)
        // Focus: what's opened alone, from the edge, popped open or not.
        let (focus, focusFrames) = at(1222, focused: true)
        #expect(focus == .alone)
        #expect(focusFrames.openedX == 0 && focusFrames.opened == 1222)
        #expect(Columns.layout(width: 1222, opened: true, focused: true, peek: true) == .alone)
        // Focus means nothing with nothing open.
        #expect(Columns.layout(width: 1222, opened: false, focused: true) == .workspace)
        // No conversation: no rail, and the list from the edge.
        let (bare, bareFrames) = at(1222, conversation: false)
        #expect(bare.conversation == .none && bare.board)
        #expect(bareFrames.content == 0 && bareFrames.openedX == 301 && bareFrames.opened == 921)
        // The popped-open conversation is its minimum, or what the rail leaves.
        #expect(Columns.peekWidth(in: 1032) == Columns.conversationMinimum())
        #expect(Columns.peekWidth(in: 300) == 300 - Columns.rail - Columns.divider)
    }

    /// The motion's state, apart from the window's (ov-85): opening,
    /// switching and closing, in any order and at any speed, end where the
    /// window's state says, and a close that settles late never takes away
    /// what opened after it.
    @Test("Open, switch and close end in the state last asked for")
    func theStageEndsInTheStateLastAskedFor() {
        var stage = WorkspaceStage<String>(open: nil)
        #expect(stage.open == nil && stage.drawn == nil)
        stage.show("bil-3")
        #expect(stage.open == "bil-3" && stage.drawn == "bil-3")
        // Glancing: the next one, in place.
        stage.show("bil-7")
        #expect(stage.open == "bil-7" && stage.drawn == "bil-7")
        // Closed: drawn until its motion settles, then let go.
        stage.show(nil)
        let closing = stage.generation
        #expect(stage.open == nil && stage.drawn == "bil-7")
        stage.settle(closing)
        #expect(stage.drawn == nil)
        // Opened again mid-close: the late settle takes nothing away.
        stage.show("bil-3")
        stage.show(nil)
        let late = stage.generation
        stage.show("bil-9")
        stage.settle(late)
        #expect(stage.open == "bil-9" && stage.drawn == "bil-9")
        // Twenty toggles inside a frame: the last one decides.
        for index in 0..<20 { stage.show(index.isMultiple(of: 2) ? "bil-\(index)" : nil) }
        #expect(stage.open == nil && stage.drawn == "bil-18")
        stage.settle(stage.generation - 1)
        #expect(stage.drawn == "bil-18", "a superseded settle let go")
        stage.settle(stage.generation)
        #expect(stage.drawn == nil)
        // Reopened on launch: drawn from the first frame, no motion.
        let launched = WorkspaceStage<String>(open: "bil-3", focused: true)
        #expect(launched.drawn == "bil-3" && launched.focused)
    }

    /// The drawn view, hosted in a detail 600 pt wide inside a 1600 pt
    /// window, lays out by its own width: a task covers the board there,
    /// where a view that read the window would put them side by side and
    /// clip. At 1032 they're side by side.
    @MainActor
    @Test("The workspace lays out by its own width, not the window's")
    func theWorkspaceLaysOutByItsOwnWidth() async {
        final class Seen { var arrangement: WorkspaceColumns.Arrangement? }
        let seen = Seen()
        struct Hosted: View {
            let seen: Seen
            let width: CGFloat
            var body: some View {
                WorkspaceView(
                    opened: "t" as String?, hasConversation: true, cell: WorkspaceColumns.defaultCell,
                    focused: false, peek: false, listWidth: .constant(300),
                    conversation: { Color.clear }, rail: { Color.clear }, board: { Color.clear },
                    breadcrumb: { _ in Color.clear }, detail: { _ in Color.clear })
                .frame(width: width, height: 400)
                .frame(width: 1600, height: 400, alignment: .leading)
                .onPreferenceChange(WorkspaceArrangementPreference.self) { value in
                    MainActor.assumeIsolated { seen.arrangement = value }
                }
            }
        }
        for (width, expected) in [(600, WorkspaceColumns.Arrangement.covering), (1032, .beside)] as [(CGFloat, WorkspaceColumns.Arrangement)] {
            seen.arrangement = nil
            let host = NSHostingView(rootView: Hosted(seen: seen, width: width))
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
            #expect(seen.arrangement == expected, "at \(width)")
        }
    }

    /// On screen means drawn: a conversation on its rail, or left out in
    /// Focus, isn't, so it isn't marked seen or watched and the keyboard
    /// doesn't act on it. Popped open, over the board or a task, it is.
    /// What's opened is on screen while it's open. Nothing is, before the
    /// detail has been measured.
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
        func columns(_ arrangement: WorkspaceColumns.Arrangement?) -> [ShownLayout.Column] {
            WorkspaceScreen.visible(shown, arrangement: arrangement).map(\.column)
        }
        #expect(columns(.besidePeeked) == [.conversation, .task])
        #expect(columns(.beside) == [.task])
        #expect(columns(.covering) == [.task])
        #expect(columns(.alone) == [.task])
        #expect(columns(.workspace) == [])
        #expect(columns(.peeked) == [.conversation])
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
