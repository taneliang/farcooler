import CoreGraphics
import Testing

@testable import Far_Cooler

/// The canvas beside the orchestrator's chat column (ov-298, Concept A): the
/// chat a fixed whole number of terminal columns, the canvas flexing, the
/// navigator floating in a narrower window, and the canvas folding away in a
/// narrower one still.
struct CanvasColumnsTests {
    static let cell = WorkspaceColumns.defaultCell

    static func frames(_ width: CGFloat, opened: Bool = false, focused: Bool = false, floating: Bool = false,
        columns: Int = 88) -> WorkspaceColumns.Frames {
        let floats = WorkspaceColumns.navigatorFloats(width: width)
        let arrangement = WorkspaceColumns.canvasLayout(
            opened: opened, hasBoard: floats ? floating : true, focused: focused, floats: floats)
        return WorkspaceColumns.frames(width: width, arrangement: arrangement, navigator: 320, cell: cell, chatColumns: columns)
    }

    @Test("Widening the window grows the canvas, never the chat")
    func wideningGrowsTheCanvas() {
        let a = Self.frames(1600), b = Self.frames(2200)
        #expect(a.chat == WorkspaceColumns.width(columns: 88, cell: Self.cell))
        #expect(a.chat == b.chat, "the chat re-wraps on a resize")
        #expect(b.main - a.main == 600)
        #expect(a.mainX == 321 && a.chatX == 1600 - a.chat)
    }

    @Test("Opening something, or Focus, leaves the chat's width alone")
    func openingKeepsTheChat() {
        let home = Self.frames(1600), opened = Self.frames(1600, opened: true), focus = Self.frames(1600, opened: true, focused: true)
        #expect(home.chat == opened.chat && opened.chat == focus.chat)
        #expect(focus.mainX == 0 && focus.main == 1600, "Focus gives the canvas the whole width")
    }

    @Test("Between 1,180 and 1,500 the navigator floats over the canvas; below 1,180 the canvas folds")
    func tiers() {
        #expect(!WorkspaceColumns.navigatorFloats(width: 1500) && WorkspaceColumns.navigatorFloats(width: 1300))
        let floating = Self.frames(1300, floating: true), hidden = Self.frames(1300)
        #expect(floating.mainX == 0 && hidden.mainX == 0, "the navigator takes no room")
        #expect(floating.chat == hidden.chat)
        #expect(floating.main >= WorkspaceColumns.canvasMinimum)
        #expect(!WorkspaceColumns.hasCanvas(width: 1179) && WorkspaceColumns.hasCanvas(width: 1180))
    }

    @Test("The chat's width is whole columns between 58 and 120, and gives way to keep the canvas")
    func chatRange() {
        #expect(WorkspaceColumns.chatColumns(40) == 58 && WorkspaceColumns.chatColumns(500) == 120)
        #expect(WorkspaceColumns.chatColumns(91.4) == 91 && WorkspaceColumns.chatColumns(.nan) == 88)
        let wide = Self.frames(1300, columns: 120)
        #expect(wide.main >= WorkspaceColumns.canvasMinimum - 0.5, "the canvas keeps its least")
    }
}
