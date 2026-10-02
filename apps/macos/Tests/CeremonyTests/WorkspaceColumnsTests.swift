import AppKit
import CoreGraphics
import SwiftUI
import Testing

@testable import Far_Cooler

/// A workspace's layout at the widths 2A measured (spec §4.3, ov-89). The
/// detail is the window less a 248 pt sidebar: 1222 pt at a full-screen 1470,
/// 1192 at 1440, and 1032 in a 1280 pt window.
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
        // At 13 pt the conversation is 426, and the board needs a wider
        // detail to stay a sidebar.
        let thirteen: CGFloat = 8.0361328125
        #expect(Columns.conversationMinimum(cell: thirteen) == 426)
        let side = Columns.rail + 1 + Columns.openedMinimum(cell: thirteen) + 1 + Columns.boardMinimum
        #expect(Columns.layout(width: side, opened: true, cell: thirteen).board == .side)
        #expect(Columns.layout(width: side - 1, opened: true, cell: thirteen).board == .strip)
    }

    /// With nothing open, the orchestrator fills the main area and the board
    /// is a sidebar on the right, from a full-screen 1470 pt window's 1222
    /// pt detail down to the narrowest that keeps it (ov-89).
    @Test("With nothing open, the orchestrator fills the main area beside the board", arguments: [1600, 1222, 1192, 1032, 779] as [CGFloat])
    func theOrchestratorFillsTheMainArea(width: CGFloat) {
        let arrangement = Columns.layout(width: width, opened: false)
        #expect(arrangement == .workspace)
        let frames = Columns.frames(width: width, arrangement: arrangement, board: 300)
        // 300 pt, or what leaves the rail and a task their own.
        let board = min(300, width - 29 - 1 - Columns.openedMinimum())
        #expect(frames.boardX == width - board && frames.board == board)
        #expect(frames.main == width - board - 1)
        // Filling it from the leading edge, as wide as it is past the rail.
        #expect(frames.conversationX == 0 && frames.conversation == frames.main - 29)
        // A peek means nothing with the orchestrator out already.
        #expect(Columns.layout(width: width, opened: false, peek: true) == .workspace)
        // No conversation: nothing fills it, and the board is where it was.
        let bare = Columns.layout(width: width, opened: false, hasConversation: false)
        #expect(bare.conversation == .none && bare.board == .side)
        #expect(Columns.frames(width: width, arrangement: bare, board: 300).boardX == width - board)
    }

    /// Opening a task springs the orchestrator to its rail and puts the task
    /// in the main area: at a 13-inch laptop's widths, a 300 pt board and
    /// 700–890 pt for the task, the same as ov-85 gave it.
    @Test("A task open takes the main area, past the rail")
    func aTaskOpenTakesTheMainArea() {
        for (width, opened) in [(1222, 892), (1192, 862), (1032, 702)] as [(CGFloat, CGFloat)] {
            let arrangement = Columns.layout(width: width, opened: true)
            #expect(arrangement == .beside, "at \(width)")
            let frames = Columns.frames(width: width, arrangement: arrangement, board: 300)
            #expect(frames.openedX == 29 && frames.opened == opened, "at \(width)")
            #expect(frames.openedX + frames.opened + 1 == frames.boardX, "at \(width)")
            // On its rail, the conversation keeps its width.
            #expect(frames.conversation == opened)
            // Popped open over the task: covering it, past the rail.
            let peeked = Columns.layout(width: width, opened: true, peek: true)
            #expect(peeked == .besidePeeked)
            let over = Columns.frames(width: width, arrangement: peeked, board: 300)
            #expect(over.conversationX == 29 && over.conversation == opened)
            #expect(over.openedX == frames.openedX && over.opened == frames.opened, "the peek resized the task")
        }
        // Focus: what's opened alone, from edge to edge.
        let focus = Columns.layout(width: 1222, opened: true, focused: true, peek: true, boardOver: true)
        #expect(focus == .alone)
        let focusFrames = Columns.frames(width: 1222, arrangement: focus, board: 300)
        #expect(focusFrames.openedX == 0 && focusFrames.opened == 1222)
        // Focus means nothing with nothing open.
        #expect(Columns.layout(width: 1222, opened: false, focused: true) == .workspace)
        // No conversation: no rail, and the task from the edge.
        let bare = Columns.layout(width: 1222, opened: true, hasConversation: false)
        #expect(bare.conversation == .none)
        #expect(Columns.frames(width: 1222, arrangement: bare, board: 300).openedX == 0)
        // No board: the main area runs to the trailing edge.
        let boardless = Columns.layout(width: 1222, opened: true, hasBoard: false)
        #expect(boardless.board == .none)
        let boardlessFrames = Columns.frames(width: 1222, arrangement: boardless, board: 300)
        #expect(boardlessFrames.main == 1222 && boardlessFrames.opened == 1222 - 29, "\(boardlessFrames)")
    }

    /// The board never moves between states (ov-89): its leading edge and
    /// width are the same with nothing open, a task open, the orchestrator
    /// popped open over it, and Focus left again, at every width that keeps
    /// it and every width it was dragged to.
    @Test("The board's width and place are the same in every state", arguments: [1600, 1222, 1032, 900, 779] as [CGFloat])
    func theBoardStaysPut(width: CGFloat) {
        for list in [100, 260, 300, 420, 900] as [CGFloat] {
            let states = [
                Columns.layout(width: width, opened: false),
                Columns.layout(width: width, opened: true),
                Columns.layout(width: width, opened: true, peek: true),
                Columns.layout(width: width, opened: false, peek: true),
            ]
            let boards = states.map { arrangement in
                let frames = Columns.frames(width: width, arrangement: arrangement, board: list)
                return [frames.boardX, frames.board, frames.main]
            }
            #expect(Set(boards).count == 1, "at \(width), dragged to \(list): \(boards)")
            let board = boards[0][1]
            #expect(board >= Columns.boardMinimum)
            #expect(board == min(max(list, Columns.boardMinimum), width - 29 - 1 - Columns.openedMinimum()))
            // What's opened keeps its 58 columns beside it, at any drag.
            let opened = Columns.frames(width: width, arrangement: states[1], board: list).opened
            #expect(opened >= Columns.openedMinimum(), "at \(width), dragged to \(list)")
        }
    }

    /// The orchestrator is one width in the main area, on its rail and
    /// popped open (ov-89 review), so moving between them never resizes its
    /// terminal or tmux window, and popped open → filling the main area is
    /// one slide. Only the detail's width, or the board collapsing, changes
    /// it. In Focus it isn't drawn, and keeps what it had.
    @Test("The orchestrator is one width in every state", arguments: [1600, 1222, 1032, 779, 778, 600] as [CGFloat])
    func theOrchestratorIsOneWidth(width: CGFloat) {
        let states = [
            Columns.layout(width: width, opened: false),
            Columns.layout(width: width, opened: true),
            Columns.layout(width: width, opened: true, peek: true),
            Columns.layout(width: width, opened: false, boardOver: true),
            Columns.layout(width: width, opened: true, peek: true, boardOver: true),
        ]
        let widths = states.map { Columns.frames(width: width, arrangement: $0, board: 300).conversation }
        #expect(Set(widths).count == 1, "at \(width): \(widths)")
        #expect(widths[0] == Columns.conversationWidth(in: Columns.frames(width: width, arrangement: states[0], board: 300).main))
        #expect(Columns.frames(width: width, arrangement: .alone, board: 300).conversation == nil)
    }

    /// Below the width where the rail, what's opened at its 58 columns and
    /// the board at its 260 pt all fit, the board collapses to a strip at
    /// the trailing edge, in every state, so the main area never drops below
    /// a usable terminal; the strip pops it open over the main area.
    @Test("The board collapses to its strip below 779 pt, and pops open over the main area")
    func theBoardCollapses() {
        #expect(Columns.sidebarMinimum() == 779)
        #expect(!Columns.collapses(width: 779))
        #expect(Columns.collapses(width: 778))
        for width in [778, 700, 600, 352] as [CGFloat] {
            let closed = Columns.layout(width: width, opened: false)
            #expect(closed == .narrow, "at \(width)")
            let open = Columns.layout(width: width, opened: true)
            #expect(open == .narrowOpened, "at \(width)")
            let frames = Columns.frames(width: width, arrangement: open, board: 300)
            #expect(frames.main == width - 29, "at \(width)")
            #expect(frames.openedX == 29 && frames.opened == width - 58, "at \(width)")
            // Tucked under the strip, out of sight.
            #expect(frames.boardX >= width - 28, "at \(width)")
            #expect(Columns.frames(width: width, arrangement: closed, board: 300).conversation == width - 58)
            // Popped open: over the main area, against the strip.
            let over = Columns.layout(width: width, opened: true, boardOver: true)
            #expect(over.board == .over && over.boardInSight)
            let overFrames = Columns.frames(width: width, arrangement: over, board: 300)
            #expect(overFrames.boardX + overFrames.board == width - 29, "at \(width)")
            #expect(overFrames.board == min(300, width - 29), "at \(width)")
            #expect(overFrames.opened == frames.opened, "popping the board resized the task")
        }
        // Wide enough, a popped board means nothing: it's the sidebar.
        #expect(Columns.layout(width: 779, opened: true, boardOver: true) == .beside)
        #expect(!Columns.Arrangement.narrow.boardInSight)
        #expect(Columns.Arrangement.workspace.boardInSight)
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
    /// window, lays out by its own width: the board collapses to its strip
    /// there, where a view that read the window would keep the sidebar and
    /// clip. At 1032 it's the sidebar.
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
                    focused: false, peek: false, boardWidth: .constant(300),
                    conversation: { Color.clear }, rail: { Color.clear }, board: { Color.clear },
                    strip: { Color.clear }, breadcrumb: { _ in Color.clear }, detail: { _, _ in Color.clear })
                .frame(width: width, height: 400)
                .frame(width: 1600, height: 400, alignment: .leading)
                .onPreferenceChange(WorkspaceArrangementPreference.self) { value in
                    MainActor.assumeIsolated { seen.arrangement = value }
                }
            }
        }
        for (width, expected) in [(600, WorkspaceColumns.Arrangement.narrowOpened), (1032, .beside)] as [(CGFloat, WorkspaceColumns.Arrangement)] {
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
    /// doesn't act on it. Filling the main area, at any width, or popped
    /// open over a task, it is (ov-89). What's opened is on screen while
    /// it's open. Nothing is, before the detail has been measured.
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
        #expect(columns(.narrowOpened) == [.task])
        #expect(columns(.alone) == [.task])
        #expect(columns(.workspace) == [.conversation])
        #expect(columns(.narrow) == [.conversation])
        // The board popped open over it leaves what's under it on screen,
        // dimmed, as a peek does.
        #expect(columns(Columns.layout(width: 600, opened: true, boardOver: true)) == [.task])
        #expect(columns(Columns.layout(width: 600, opened: false, boardOver: true)) == [.conversation])
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
