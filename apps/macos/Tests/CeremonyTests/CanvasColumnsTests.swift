import AppKit
import CoreGraphics
import Testing

@testable import Far_Cooler

/// The canvas beside the orchestrator's chat column (ov-298, Concept A): the
/// chat a fixed whole number of terminal columns, the canvas flexing, the
/// navigator floating where it doesn't fit beside them, and the canvas
/// folding away where it doesn't fit either.
struct CanvasColumnsTests {
    static let cell = WorkspaceColumns.defaultCell

    static func arrangement(
        _ width: CGFloat, sizing: WorkspaceColumns.Canvas = .init(), opened: Bool = false, focused: Bool = false,
        floating: Bool = false
    ) -> WorkspaceColumns.Arrangement {
        WorkspaceColumns.arrangement(
            width: width, canvas: sizing, opened: opened, hasConversation: true, hasBoard: true, boardExists: true,
            floating: floating, focused: focused)
    }

    static func frames(
        _ width: CGFloat, sizing: WorkspaceColumns.Canvas = .init(), opened: Bool = false, focused: Bool = false,
        floating: Bool = false
    ) -> WorkspaceColumns.Frames {
        WorkspaceColumns.frames(
            width: width, arrangement: arrangement(width, sizing: sizing, opened: opened, focused: focused, floating: floating),
            navigator: sizing.navigator, cell: cell, chatColumns: sizing.chatColumns)
    }

    @Test("Widening the window grows the canvas, never the chat")
    func wideningGrowsTheCanvas() {
        let a = Self.frames(1600), b = Self.frames(2200)
        #expect(a.chat == WorkspaceColumns.width(columns: 88, cell: Self.cell))
        #expect(a.chat == b.chat, "the chat re-wraps on a resize")
        #expect(b.main - a.main == 600)
        #expect(a.mainX == 321 && a.chatX == 1600 - a.chat)
    }

    /// Train 1004r, P2: a navigator wider than about 338 pt, or a chat wider
    /// than 88 columns, shrank the chat as the window crossed 1,500 pt.
    @Test("The chat is its columns in every tier with a canvas, at any navigator and chat width")
    func chatHoldsThroughTheTiers() {
        for navigator in [CGFloat(240), 320, 400, 600] {
            for columns in [58, 88, 120] {
                let sizing = WorkspaceColumns.Canvas(navigator: navigator, chatColumns: columns, cell: Self.cell)
                let want = WorkspaceColumns.width(columns: columns, cell: Self.cell)
                for width in stride(from: CGFloat(900), through: 2600, by: 1) where sizing.hasCanvas(width) {
                    for floating in [false, true] {
                        let frames = Self.frames(width, sizing: sizing, floating: floating)
                        #expect(frames.chat == want, "navigator \(navigator), \(columns) columns, at \(width): \(frames.chat)")
                        #expect(frames.main >= WorkspaceColumns.canvasMinimum, "the canvas squeezed at \(width)")
                    }
                }
            }
        }
    }

    @Test("Opening something, or Focus, leaves the chat's width alone, and keeps it on screen but in Focus")
    func openingKeepsTheChat() {
        let home = Self.frames(1600), opened = Self.frames(1600, opened: true)
        let focus = Self.frames(1600, opened: true, focused: true)
        #expect(home.chat == opened.chat && opened.chat == focus.chat)
        #expect(focus.mainX == 0 && focus.main == 1600, "Focus gives the canvas the whole width")
        #expect(Self.arrangement(1600, opened: true).showsConversation)
        #expect(Self.arrangement(1300, opened: true).showsConversation)
        #expect(!Self.arrangement(1600, opened: true, focused: true).showsConversation)
    }

    @Test("Between 1,180 and 1,500 the navigator floats; below 1,180 the canvas folds and the navigator floats over the chat")
    func tiers() {
        let sizing = WorkspaceColumns.Canvas()
        #expect(!sizing.navigatorFloats(1500) && sizing.navigatorFloats(1300))
        let floating = Self.frames(1300, floating: true), hidden = Self.frames(1300)
        #expect(floating.mainX == 0 && hidden.mainX == 0, "the navigator takes no room")
        #expect(Self.arrangement(1300, floating: true).navigator && !Self.arrangement(1300).navigator)
        #expect(!sizing.hasCanvas(1179) && sizing.hasCanvas(1180))
        // The ruling on P5: at 1,000 the navigator is hidden, and ⌘B floats it.
        let folded = Self.arrangement(1000)
        #expect(!folded.canvas && !folded.navigator && folded.conversation == .main)
        #expect(Self.arrangement(1000, floating: true).navigator && Self.frames(1000, floating: true).mainX == 0)
        #expect(Self.frames(1000).main == 1000, "the chat takes the main area, whatever the navigator does")
    }

    @Test("The chat's width is whole columns between 58 and 120")
    func chatRange() {
        #expect(WorkspaceColumns.chatColumns(40) == 58 && WorkspaceColumns.chatColumns(500) == 120)
        #expect(WorkspaceColumns.chatColumns(91.4) == 91 && WorkspaceColumns.chatColumns(.nan) == 88)
    }

    /// Train 1004r, P1: the window decided what's seen and keyed with the
    /// old layout, which hid the orchestrator whenever something was opened.
    @Test("Beside the canvas the orchestrator's layout is on screen with something opened")
    func chatIsVisibleWithSomethingOpened() {
        let worktree = Worktree(
            id: "w0", short: "w0", task: "main", branch: "main", repository: "r", host: "", path: "/tmp/w0",
            state: "active", terminals: [], repositoryID: "r", workspace: nil)
        let group = PaneGroup(
            id: "@1", name: "", active: true, columns: 80, rows: 24, layout: "@1",
            panes: [PaneRect(id: "t1", short: "t1", title: nil, left: 0, top: 0, columns: 80, rows: 24, focused: true, zoomed: false)])
        let chat = ShownLayout(column: .conversation, worktree: worktree, group: group, groups: [group])
        let task = ShownLayout(column: .task, worktree: worktree, group: group, groups: [group])
        let visible = WorkspaceScreen.visible([chat, task], arrangement: Self.arrangement(1600, opened: true))
        #expect(visible.contains(chat) && visible.contains(task))
        #expect(!WorkspaceScreen.visible([chat], arrangement: Self.arrangement(1000, opened: true)).contains(chat),
            "folded, what's opened replaces the chat")
    }
}
