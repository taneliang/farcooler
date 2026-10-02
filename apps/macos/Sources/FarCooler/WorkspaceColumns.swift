import CoreGraphics
import Foundation

// How a workspace's detail is drawn (spec §4.3), as a chain of control read
// left to right: the orchestrator, the board, the work (ov-85).
//
// The orchestrator is a rail at the leading edge at every level, and pops
// open over the rest. With nothing open, the board fills the content. A
// task or a worktree opened narrows the board to a list column, its width
// remembered and its divider draggable, and opens beside it: the widest pane
// goes to the work. Where the two don't fit, what's opened covers the board,
// which stays drawn underneath for when it closes.
//
// All of that is decided here, as values, so the view only places what these
// say, and a test can read the widths at any window width.

enum WorkspaceColumns {
    /// A cell's width at the default terminal font, SF Mono 12.5
    /// (`TerminalMetrics.cell`), as 2A measured it.
    static let defaultCell: CGFloat = 7.72705078125

    /// What a tile column spends on other than cells: the canvas inset
    /// (`Pane.inset`, 10 each side) and the terminal's own padding
    /// (`TerminalMetrics.padding`, 10 each side). `TileGeometry.viewport`
    /// counts the same.
    static let chrome: CGFloat = 40

    /// The conversation's width popped open, in terminal columns: every
    /// fixed line an orchestrator shows fits, and it's what an iPhone shows.
    static let conversationColumns = 48
    /// What's opened beside the board, at the least, in terminal columns:
    /// the width at which no fixed line of any harness's permission prompt
    /// wraps (spec §4.3, measured for the old task column). Narrower, what's
    /// opened covers the board instead of sharing the width with it.
    static let openedColumns = 58
    /// The board list's narrowest: room for a list card's key, a short title
    /// and its agent pill (ov-85).
    static let boardMinimum: CGFloat = 260
    /// The board list's width beside what's opened until its divider is
    /// dragged.
    static let boardListDefault: CGFloat = 300
    /// The orchestrator's rail: as wide as its icon and its sideways label
    /// need (ov-84).
    static let rail: CGFloat = 28
    /// Each divider: the rail's, and the board list's.
    static let divider: CGFloat = 1

    /// The narrowest a tile column holding `columns` terminal columns can be
    /// at `cell`, in whole points: `floor((W − 40) / cell)` is at least
    /// `columns` from here up.
    static func width(columns: Int, cell: CGFloat) -> CGFloat {
        (chrome + CGFloat(columns) * cell).rounded(.up)
    }

    static func conversationMinimum(cell: CGFloat = defaultCell) -> CGFloat {
        width(columns: conversationColumns, cell: cell)
    }

    static func openedMinimum(cell: CGFloat = defaultCell) -> CGFloat {
        width(columns: openedColumns, cell: cell)
    }

    /// How wide the conversation pops open in a detail `width` pt wide: its
    /// minimum, or what's left past the rail when that's less.
    static func peekWidth(in width: CGFloat, cell: CGFloat = defaultCell) -> CGFloat {
        max(0, min(conversationMinimum(cell: cell), width - rail - divider))
    }

    /// What's drawn.
    struct Arrangement: Equatable {
        enum Conversation: Equatable {
            /// The 28 pt rail at the leading edge: its status and its dot.
            /// Clicking it pops the conversation open.
            case rail
            /// The rail, with the conversation popped open over the board
            /// and what's opened, which stay where they are underneath.
            case peek
            /// Not drawn: Focus, or a workspace with no conversation (a
            /// runner without `workstreams`, or a loose worktree).
            case none
        }

        var conversation: Conversation
        /// A task or a worktree is open beside the board, or over it.
        var opened: Bool
        /// The board is in sight: alone, or as the list beside what's
        /// opened. Covered by what's opened, it's still drawn, but not this.
        var board: Bool

        static let workspace = Arrangement(conversation: .rail, opened: false, board: true)
        static let peeked = Arrangement(conversation: .peek, opened: false, board: true)
        static let beside = Arrangement(conversation: .rail, opened: true, board: true)
        static let besidePeeked = Arrangement(conversation: .peek, opened: true, board: true)
        static let covering = Arrangement(conversation: .rail, opened: true, board: false)
        static let alone = Arrangement(conversation: .none, opened: true, board: false)
        static let boardAlone = Arrangement(conversation: .none, opened: false, board: true)
    }

    /// What a detail `width` pt wide draws (spec §4.3).
    ///
    /// - The rail, at every level, unless the workspace has no conversation
    ///   (`hasConversation` false) or it's Focus; `peek` pops it open.
    /// - Nothing open: the board, filling the content.
    /// - Something open (`opened`): the board as a list beside it when both
    ///   fit at their minimums, else what's opened over the board.
    /// - `focused` is Focus (⌃⌘↩): what's opened alone, with neither the rail
    ///   nor the board. It means nothing with nothing open.
    ///
    /// `cell` is the terminal font's cell width: what's opened has a minimum
    /// in columns, so a larger font needs a wider detail to keep the board.
    static func layout(
        width: CGFloat, opened: Bool, cell: CGFloat = defaultCell, hasConversation: Bool = true,
        focused: Bool = false, peek: Bool = false
    ) -> Arrangement {
        let focused = focused && opened
        let conversation: Arrangement.Conversation =
            !hasConversation || focused ? .none : peek ? .peek : .rail
        guard opened else { return Arrangement(conversation: conversation, opened: false, board: true) }
        let content = width - (conversation == .none ? 0 : rail + divider)
        let fits = !focused && content >= boardMinimum + divider + openedMinimum(cell: cell)
        return Arrangement(conversation: conversation, opened: true, board: fits)
    }

    /// The board list's width beside what's opened in a content `content`
    /// pt wide: the remembered width, held between the list's minimum and
    /// what leaves what's opened its own.
    static func listWidth(_ remembered: CGFloat, content: CGFloat, cell: CGFloat = defaultCell) -> CGFloat {
        let most = content - divider - openedMinimum(cell: cell)
        return max(boardMinimum, min(remembered, most))
    }

    /// Where each part sits, in points from the detail's leading edge.
    struct Frames: Equatable {
        /// The board's leading edge: past the rail and its divider, or 0.
        var content: CGFloat
        /// The board's width: the content's alone, or the list's beside
        /// what's opened. Covered, it keeps the list's, so it doesn't reflow
        /// under what covers it.
        var board: CGFloat
        /// What's opened: its leading edge open, and its width.
        var openedX: CGFloat
        var opened: CGFloat
    }

    /// The frames `arrangement` puts its parts at in a detail `width` pt
    /// wide, with the list `list` pt wide when it can be.
    static func frames(
        width: CGFloat, arrangement: Arrangement, list: CGFloat, cell: CGFloat = defaultCell
    ) -> Frames {
        let content = arrangement.conversation == .none ? 0 : rail + divider
        let room = width - content
        let listed = listWidth(list, content: room, cell: cell)
        if !arrangement.opened {
            return Frames(content: content, board: room, openedX: content + listed + divider, opened: room - listed - divider)
        }
        if arrangement.board {
            return Frames(content: content, board: listed, openedX: content + listed + divider, opened: room - listed - divider)
        }
        return Frames(content: content, board: listed, openedX: content, opened: room)
    }
}
