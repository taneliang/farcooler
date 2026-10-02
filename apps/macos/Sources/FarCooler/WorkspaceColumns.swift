import CoreGraphics
import Foundation

// How a workspace's detail is drawn (spec §4.3, ov-89): a main area on the
// left that shows one thing, and the board as a fixed sidebar on the right.
//
// With nothing open, the orchestrator fills the main area. A task or a
// worktree opened takes it instead, and the orchestrator springs down to a
// rail at its leading edge, which pops it open over what's opened. The
// board's width is remembered and dragged from its leading edge, and doesn't
// change between the two. Below the width where both keep their minimums,
// the board collapses to a strip at the trailing edge, which pops it open
// over the main area.
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
    /// What's opened in the main area, at the least, in terminal columns:
    /// the width at which no fixed line of any harness's permission prompt
    /// wraps (spec §4.3, measured for the old task column). Narrower, the
    /// board collapses to its strip to leave it these.
    static let openedColumns = 58
    /// The board sidebar's narrowest: room for a list card's key, a short
    /// title and its agent pill (ov-85).
    static let boardMinimum: CGFloat = 260
    /// The board sidebar's width until its leading edge is dragged (ov-89).
    static let boardListDefault: CGFloat = 300
    /// The orchestrator's rail: as wide as its icon and its sideways label
    /// need (ov-84). The board's collapsed strip mirrors it (ov-89).
    static let rail: CGFloat = 28
    /// Each divider: the rail's, and the board's.
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

    /// The narrowest detail that keeps the board as a sidebar: the rail,
    /// what's opened at its minimum, and the board at its own, each with its
    /// divider. Narrower, the board collapses to its strip (ov-89). One
    /// threshold for every state, rail or not, so the board never comes and
    /// goes as a task opens and closes: 779 pt at the default font.
    static func sidebarMinimum(cell: CGFloat = defaultCell) -> CGFloat {
        rail + divider + openedMinimum(cell: cell) + divider + boardMinimum
    }

    /// Whether a detail `width` pt wide collapses the board to its strip.
    static func collapses(width: CGFloat, cell: CGFloat = defaultCell) -> Bool {
        width < sidebarMinimum(cell: cell)
    }

    /// The board sidebar's width in a detail `width` pt wide: the remembered
    /// width, held between its minimum and what leaves the rail and what's
    /// opened their own. It depends on nothing else, so it's the same with
    /// a task open as without (ov-89).
    static func boardWidth(_ remembered: CGFloat, width: CGFloat, cell: CGFloat = defaultCell) -> CGFloat {
        let most = width - rail - divider - openedMinimum(cell: cell) - divider
        return max(boardMinimum, min(remembered, most))
    }

    /// How wide the conversation pops open over what's opened, in a main
    /// area `main` pt wide: its minimum, or what's left past the rail.
    static func peekWidth(in main: CGFloat, cell: CGFloat = defaultCell) -> CGFloat {
        max(0, min(conversationMinimum(cell: cell), main - rail - divider))
    }

    /// What's drawn (spec §4.3, ov-89): the main area on the left, which
    /// shows one thing, the orchestrator or what's opened, and the board on
    /// the right.
    struct Arrangement: Equatable {
        enum Conversation: Equatable {
            /// Filling the main area: nothing is open.
            case main
            /// The 28 pt rail at the main area's leading edge, beside what's
            /// opened. Clicking it pops the conversation open.
            case rail
            /// The rail, with the conversation popped open over what's
            /// opened, which stays where it is underneath.
            case peek
            /// Not drawn: Focus, or a workspace with no conversation (a
            /// runner without `workstreams`, or a loose worktree).
            case none
        }

        enum Board: Equatable {
            /// The sidebar at the trailing edge.
            case side
            /// Collapsed to its strip at a narrow width.
            case strip
            /// Collapsed, and popped open from the strip over the main area.
            case over
            /// Not drawn: Focus, or a loose worktree with no board.
            case none
        }

        var conversation: Conversation
        /// A task or a worktree is open in the main area.
        var opened: Bool
        var board: Board

        /// Whether the board's list is in sight to take the keyboard.
        var boardInSight: Bool { board == .side || board == .over }

        static let workspace = Arrangement(conversation: .main, opened: false, board: .side)
        static let beside = Arrangement(conversation: .rail, opened: true, board: .side)
        static let besidePeeked = Arrangement(conversation: .peek, opened: true, board: .side)
        static let narrow = Arrangement(conversation: .main, opened: false, board: .strip)
        static let narrowOpened = Arrangement(conversation: .rail, opened: true, board: .strip)
        static let alone = Arrangement(conversation: .none, opened: true, board: .none)
    }

    /// What a detail `width` pt wide draws (spec §4.3).
    ///
    /// - Nothing open: the conversation fills the main area; a peek means
    ///   nothing, since it's out already.
    /// - Something open (`opened`): the rail, and what's opened beside it;
    ///   `peek` pops the conversation open over it.
    /// - The board is the sidebar at the trailing edge in both, or its strip
    ///   below `sidebarMinimum`, popped open over the main area with
    ///   `boardOver`.
    /// - `focused` is Focus (⌃⌘↩): what's opened alone, with neither the rail
    ///   nor the board. It means nothing with nothing open.
    ///
    /// `cell` is the terminal font's cell width: the minimums are in columns,
    /// so a larger font collapses the board at a wider detail.
    static func layout(
        width: CGFloat, opened: Bool, cell: CGFloat = defaultCell, hasConversation: Bool = true,
        hasBoard: Bool = true, focused: Bool = false, peek: Bool = false, boardOver: Bool = false
    ) -> Arrangement {
        let focused = focused && opened
        let conversation: Arrangement.Conversation =
            !hasConversation || focused ? .none : !opened ? .main : peek ? .peek : .rail
        let board: Arrangement.Board =
            !hasBoard || focused ? .none : !collapses(width: width, cell: cell) ? .side : boardOver ? .over : .strip
        return Arrangement(conversation: conversation, opened: opened, board: board)
    }

    /// Where each part sits, in points from the detail's leading edge.
    struct Frames: Equatable {
        /// The main area's width, from the leading edge: what the board, or
        /// its strip, leaves.
        var main: CGFloat
        /// What's opened: its leading edge, past the rail, and its width.
        var openedX: CGFloat
        var opened: CGFloat
        /// The board's leading edge and width: at the trailing edge as the
        /// sidebar; over the main area, or tucked under the strip, collapsed.
        var boardX: CGFloat
        var board: CGFloat
        /// The conversation's width while it's in sight: the main area's, or
        /// popped open, its minimum. Nil on the rail, where it keeps the
        /// width it had (`WorkspaceView`), so it isn't resized on the way.
        var conversation: CGFloat?
        /// Where the conversation's panel starts: the rail's trailing edge,
        /// or the leading edge with no rail.
        var conversationX: CGFloat
    }

    /// The frames `arrangement` puts its parts at in a detail `width` pt
    /// wide, with the board `remembered` pt wide when it can be.
    static func frames(
        width: CGFloat, arrangement: Arrangement, list remembered: CGFloat, cell: CGFloat = defaultCell
    ) -> Frames {
        let sidebar = boardWidth(remembered, width: width, cell: cell)
        let trailing: CGFloat
        let boardX: CGFloat
        let board: CGFloat
        switch arrangement.board {
        case .side:
            trailing = sidebar + divider
            board = sidebar
            boardX = width - sidebar
        case .strip, .over:
            trailing = rail + divider
            // Over the main area, as wide as it would be beside it, or what
            // the main area has; tucked under the strip otherwise.
            board = max(0, min(max(boardMinimum, remembered), width - trailing))
            boardX = arrangement.board == .over ? width - trailing - board : width - rail
        case .none:
            trailing = 0
            board = sidebar
            boardX = width + divider
        }
        let main = max(0, width - trailing)
        let edge: CGFloat = [.rail, .peek].contains(arrangement.conversation) ? rail + divider : 0
        let conversation: CGFloat? =
            switch arrangement.conversation {
            case .main: main
            case .peek: peekWidth(in: main, cell: cell)
            case .rail, .none: nil
            }
        return Frames(
            main: main, openedX: edge, opened: max(0, main - edge), boardX: boardX, board: board,
            conversation: conversation, conversationX: edge)
    }
}
