import AppKit
import CoreGraphics
import SwiftUI
import Testing

@testable import Far_Cooler

/// A workspace's layout at the widths 2A measured (spec §4.3, ov-92). The
/// detail is the window less a 248 pt sidebar: 1222 pt at a full-screen 1470,
/// 1192 at 1440, and 1032 in a 1280 pt window.
struct WorkspaceColumnsTests {
    private typealias Columns = WorkspaceColumns

    /// What's selected in the main area keeps 58 columns as the navigator is
    /// dragged wider: one point less loses one, and it grows with the font.
    @Test("The main area's minimum holds its measured columns and no more")
    func theMinimumHoldsItsMeasuredColumns() {
        func columns(_ width: CGFloat) -> Int { Int((width - Columns.chrome) / Columns.defaultCell) }
        #expect(columns(Columns.openedMinimum()) == 58)
        #expect(columns(Columns.openedMinimum() - 1) == 57)
        #expect(Columns.openedMinimum() == 489)
        let thirteen: CGFloat = 8.0361328125
        #expect(Columns.openedMinimum(cell: thirteen) > Columns.openedMinimum())
    }

    /// The navigator is on the left at its remembered width, and the main
    /// area is the rest, the same whether the orchestrator or a task is
    /// selected, so neither resizes as the selection moves (ov-92).
    @Test("The navigator is on the left and the main area one width in every selection", arguments: [1600, 1222, 1192, 1032, 779, 600] as [CGFloat])
    func theNavigatorIsOnTheLeft(width: CGFloat) {
        let orchestrator = Columns.layout(opened: false)
        let task = Columns.layout(opened: true)
        #expect(orchestrator == .workspace && task == .opened)
        let a = Columns.frames(width: width, arrangement: orchestrator, navigator: 280)
        let b = Columns.frames(width: width, arrangement: task, navigator: 280)
        #expect(a == b, "the main area moved as the selection did")
        // 280 pt, or what leaves the main area its own, never under 240.
        let navigator = max(240, min(280, width - 1 - Columns.openedMinimum()))
        #expect(a.navigator == navigator)
        #expect(a.mainX == navigator + 1 && a.main == width - navigator - 1)
        // No conversation: none drawn, and the navigator where it was.
        let bare = Columns.layout(opened: false, hasConversation: false)
        #expect(bare.conversation == .none && bare.navigator)
        #expect(Columns.frames(width: width, arrangement: bare, navigator: 280) == a)
    }

    /// Dragged, the navigator is held between its minimum and what leaves
    /// the main area 58 columns; it never goes under its minimum, however
    /// narrow the detail.
    @Test("The navigator's width is held between its minimum and the main area's")
    func theNavigatorIsHeld() {
        #expect(Columns.navigatorWidth(100, width: 1222) == Columns.navigatorMinimum)
        #expect(Columns.navigatorWidth(320, width: 1222) == 320)
        // Dragged far, held at the navigator's widest (ov-177), or at what
        // leaves the main area its own, whichever is narrower.
        #expect(Columns.navigatorWidth(2000, width: 1222) == min(Columns.navigatorMaximum, 1222 - 1 - Columns.openedMinimum()))
        #expect(Columns.navigatorWidth(2000, width: 3000) == Columns.navigatorMaximum)
        #expect(Columns.navigatorWidth(320, width: 500) == Columns.navigatorMinimum)
    }

    /// Focus is what's opened alone: the navigator goes, and the main area
    /// takes the detail. It means nothing with the orchestrator selected,
    /// and a loose worktree with no board has no navigator.
    @Test("Focus and a loose worktree with no board leave the main area the detail")
    func focusLeavesTheMainArea() {
        #expect(Columns.layout(opened: true, focused: true) == .alone)
        #expect(Columns.layout(opened: false, focused: true) == .workspace)
        let frames = Columns.frames(width: 1032, arrangement: .alone, navigator: 280)
        #expect(frames.mainX == 0 && frames.main == 1032 && frames.navigator == 280)
        #expect(!Columns.layout(opened: true, hasBoard: false).navigator)
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

    /// The drawn view, hosted in a detail 700 pt wide inside a 1600 pt
    /// window, lays out by its own width: the navigator gives way to keep
    /// the main area its columns there, where a view that read the window
    /// would keep it at 320 and leave the main area short.
    @MainActor
    @Test("The workspace lays out by its own width, not the window's")
    func theWorkspaceLaysOutByItsOwnWidth() async {
        final class Seen { var width: CGFloat? }
        let seen = Seen()
        struct Hosted: View {
            let seen: Seen
            let width: CGFloat
            var body: some View {
                WorkspaceView(
                    opened: "t" as String?, hasConversation: true, cell: WorkspaceColumns.defaultCell,
                    focused: false, navigatorWidth: .constant(320),
                    conversation: { Color.clear }, navigator: { Color.clear },
                    breadcrumb: { _ in Color.clear }, detail: { _, _ in Color.clear })
                .frame(width: width, height: 400)
                .frame(width: 1600, height: 400, alignment: .leading)
                .onPreferenceChange(WorkspaceWidthPreference.self) { value in
                    MainActor.assumeIsolated { seen.width = value }
                }
            }
        }
        for width in [700, 1032] as [CGFloat] {
            seen.width = nil
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
            #expect(seen.width == width, "at \(width)")
            let frames = WorkspaceColumns.frames(width: width, arrangement: .opened, navigator: 320)
            #expect(frames.main >= WorkspaceColumns.openedMinimum() || frames.navigator == WorkspaceColumns.navigatorMinimum)
        }
    }

    /// On screen means selected: the orchestrator kept hidden behind a
    /// task, or in Focus, isn't, so it isn't marked seen or watched and the
    /// keyboard doesn't act on it (ov-92). What's opened is on screen while
    /// it's open. Nothing is, before the detail has been measured.
    @Test("A hidden orchestrator isn't on screen")
    func aHiddenOrchestratorIsntOnScreen() {
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
        #expect(columns(.opened) == [.task])
        #expect(columns(.alone) == [.task])
        #expect(columns(.workspace) == [.conversation])
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
