import CoreGraphics
import Foundation

// How a workspace's detail is drawn, level by level (spec §4.3).
//
// Two levels, never four columns (ov-79). At the workspace level the
// conversation and the board share the detail's width, each with a minimum
// measured in 2A against the app's own code
// (`.claude/agent/reports/ui/2a-report.md`). `HSplitView` never collapses a
// child on its own: below the sum of its children's minimums it overflows the
// window and clips. So the collapse is decided here, as a value, before the
// split view is drawn. Opening a task or a worktree drills in: what's opened
// takes the detail, and the conversation shrinks to a rail at its leading
// edge, which pops it open over what's opened without leaving it.

enum WorkspaceColumns {
    /// A cell's width at the default terminal font, SF Mono 12.5
    /// (`TerminalMetrics.cell`), as 2A measured it.
    static let defaultCell: CGFloat = 7.72705078125

    /// What a tile column spends on other than cells: the canvas inset
    /// (`Pane.inset`, 10 each side) and the terminal's own padding
    /// (`TerminalMetrics.padding`, 10 each side). `TileGeometry.viewport`
    /// counts the same.
    static let chrome: CGFloat = 40

    /// The conversation's minimum, in terminal columns: every fixed line an
    /// orchestrator shows fits, and it's what an iPhone shows.
    static let conversationColumns = 48
    /// The board's minimum: one kanban column's outer width, a 260 pt card
    /// plus its 10 pt padding each side.
    static let boardMinimum: CGFloat = 280
    /// The board's width when there's room: a list with room for its rows'
    /// titles, and not so wide the conversation loses a column for it.
    static let boardIdeal: CGFloat = 340
    /// What the conversation shrinks to with a task or a worktree open.
    static let rail: CGFloat = 36
    /// Each `HSplitView` divider.
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

    /// How wide the conversation pops open over a task or a worktree in a
    /// detail `width` pt wide: its minimum, or what's left past the rail when
    /// that's less.
    static func peekWidth(in width: CGFloat, cell: CGFloat = defaultCell) -> CGFloat {
        max(0, min(conversationMinimum(cell: cell), width - rail - divider))
    }

    /// What's drawn, and how.
    struct Arrangement: Equatable {
        enum Conversation: Equatable {
            /// Its own column, beside the board.
            case column
            /// Shrunk to a 36 pt rail at the leading edge: its status and its
            /// dot. Clicking it pops the conversation open.
            case rail
            /// The rail, with the conversation popped open over what's
            /// opened, which stays where it is underneath.
            case peek
            /// Not drawn: Focus, or a workspace with no conversation (a
            /// runner without `workstreams`).
            case none
        }

        var conversation: Conversation
        var board: Bool
        /// A task or a worktree, drilled into: the detail is its.
        var drilled: Bool
        /// One column at a time, chosen by a segmented Orchestrator | Board
        /// control in the header: the phone's form, for a detail too narrow
        /// for both.
        var switcher: Bool

        static let two = Arrangement(conversation: .column, board: true, drilled: false, switcher: false)
        static let one = Arrangement(conversation: .column, board: true, drilled: false, switcher: true)
        static let boardAlone = Arrangement(conversation: .none, board: true, drilled: false, switcher: false)
        static let drilledIn = Arrangement(conversation: .rail, board: false, drilled: true, switcher: false)
        static let peeked = Arrangement(conversation: .peek, board: false, drilled: true, switcher: false)
        static let drilledAlone = Arrangement(conversation: .none, board: false, drilled: true, switcher: false)
    }

    /// What a detail `width` pt wide draws (spec §4.3).
    ///
    /// - At the workspace level (`drilled` false): the conversation and the
    ///   board when both fit, else one of them at a time, with
    ///   Orchestrator | Board in the header.
    /// - Drilled into a task or a worktree, at any width: it, with the
    ///   conversation as a rail; `peek` pops the conversation open over it.
    /// - `focused` is Focus (⌃⌘↩): what's opened alone, without the rail.
    /// - A workspace with no conversation (`hasConversation` false) has the
    ///   board, or what's opened, alone.
    ///
    /// `cell` is the terminal font's cell width: the conversation's minimum
    /// is in columns, so a larger font needs a wider detail for both.
    static func layout(
        width: CGFloat, drilled: Bool, cell: CGFloat = defaultCell, hasConversation: Bool = true,
        focused: Bool = false, peek: Bool = false
    ) -> Arrangement {
        guard drilled else {
            guard hasConversation else { return .boardAlone }
            return width >= conversationMinimum(cell: cell) + boardMinimum + divider ? .two : .one
        }
        if !hasConversation || focused { return .drilledAlone }
        return peek ? .peeked : .drilledIn
    }
}
