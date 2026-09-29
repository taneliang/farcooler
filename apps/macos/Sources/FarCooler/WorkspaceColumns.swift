import CoreGraphics
import Foundation

// How a workspace's three columns share the detail's width (spec §4.3).
//
// Conversation, board and task, left to right, each with a minimum measured in
// 2A against the app's own code (`.claude/agent/reports/ui/2a-report.md`).
// `HSplitView` never collapses a child on its own: below the sum of its
// children's minimums it overflows the window and clips. So the collapse is
// decided here, as a value, before the split view is drawn.

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
    /// The task column's minimum, in terminal columns: no fixed line of any
    /// harness's permission prompt wraps.
    static let taskColumns = 58
    /// The board's minimum: one kanban column's outer width, a 260 pt card
    /// plus its 10 pt padding each side.
    static let boardMinimum: CGFloat = 280
    /// The board's width when there's room: a list with room for its rows'
    /// titles, and not so wide the conversation loses a column for it.
    static let boardIdeal: CGFloat = 340
    /// What the conversation collapses to beside an open task.
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

    static func taskMinimum(cell: CGFloat = defaultCell) -> CGFloat {
        width(columns: taskColumns, cell: cell)
    }

    /// What's drawn, and how.
    struct Arrangement: Equatable {
        enum Conversation: Equatable {
            /// Its own column.
            case column
            /// Collapsed to a 36 pt rail at the leading edge: its status and
            /// its dot. Clicking it closes the task column.
            case rail
            /// Not drawn: the task column alone, or a workspace with no
            /// conversation (a runner without `workstreams`).
            case none
        }

        var conversation: Conversation
        var board: Bool
        var task: Bool
        /// One column at a time, chosen by a segmented Orchestrator | Board
        /// control in the header: the phone's form, for a detail too narrow
        /// for both.
        var switcher: Bool

        static let all = Arrangement(conversation: .column, board: true, task: true, switcher: false)
        static let railed = Arrangement(conversation: .rail, board: true, task: true, switcher: false)
        static let taskAlone = Arrangement(conversation: .none, board: false, task: true, switcher: false)
        static let two = Arrangement(conversation: .column, board: true, task: false, switcher: false)
        static let one = Arrangement(conversation: .column, board: true, task: false, switcher: true)
    }

    /// The columns a detail `width` pt wide shows (spec §4.3's table).
    ///
    /// - With a task open: all three when they fit; else the conversation
    ///   collapses to its rail; else the task column alone, with Back.
    /// - With none: the conversation and the board when both fit, else one
    ///   of them at a time, with Orchestrator | Board in the header.
    /// - `focused` is Focus Column (⌃⌘↩): the task column over the other two.
    /// - A workspace with no conversation column (`hasConversation` false)
    ///   has the board and the task, or the task alone.
    ///
    /// `cell` is the terminal font's cell width: the minimums are in columns,
    /// so a larger font needs a wider detail.
    static func layout(
        width: CGFloat, taskOpen: Bool, cell: CGFloat = defaultCell, hasConversation: Bool = true,
        focused: Bool = false
    ) -> Arrangement {
        let conversation = conversationMinimum(cell: cell)
        let task = taskMinimum(cell: cell)
        guard hasConversation else {
            guard taskOpen else { return Arrangement(conversation: .none, board: true, task: false, switcher: false) }
            if focused || width < boardMinimum + task + divider { return .taskAlone }
            return Arrangement(conversation: .none, board: true, task: true, switcher: false)
        }
        guard taskOpen else {
            return width >= conversation + boardMinimum + divider ? .two : .one
        }
        if focused { return .taskAlone }
        if width >= conversation + boardMinimum + task + 2 * divider { return .all }
        if width >= rail + boardMinimum + task + 2 * divider { return .railed }
        return .taskAlone
    }
}
