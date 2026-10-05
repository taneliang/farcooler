import CoreGraphics
import Foundation

// How a workspace's detail is drawn (spec §4.3, ov-92): the navigator on the
// left, a source list in three sections (the orchestrator, the tasks and the
// worktrees), and the main area on the right, which shows what's selected in
// it: the orchestrator, by default, or a task or a worktree.
//
// The navigator's width is remembered and dragged from its trailing edge.
// The orchestrator's terminal is mounted once and kept, hidden while
// something else is selected, at the main area's one width, so going back to
// it is instant and nothing in it re-wraps. Focus (⌃⌘↩) puts the navigator
// away.
//
// All of that is decided here, as values, so the view only places what these
// say, and a test can read the widths at any window width.

enum WorkspaceColumns {
    /// A cell's width at the default terminal font, SF Mono 12.5
    /// (`TerminalMetrics.cell`), as 2A measured it.
    static let defaultCell: CGFloat = 7.72705078125

    /// What a tile column spends on other than cells: the canvas inset
    /// (`Pane.inset`, 10 each side on macOS 26 and 6 on 27) and the terminal's
    /// own padding (`TerminalMetrics.padding`, 10 each side).
    /// `TileGeometry.viewport` counts the same.
    static var chrome: CGFloat { 2 * Pane.inset + 20 }

    /// What the main area keeps, at the least, in terminal columns, as the
    /// navigator is dragged wider: the width at which no fixed line of any
    /// harness's permission prompt wraps (spec §4.3).
    static let openedColumns = 58
    /// The navigator's narrowest: room for a task's key, a short title and
    /// its agent pill, and the orchestrator's state (ov-92).
    static let navigatorMinimum: CGFloat = 240
    /// The navigator's width until its trailing edge is dragged: 320, from
    /// 280, so a task's title has room before it wraps (ov-177, the owner:
    /// "the sidebar is a little too narrow"). A width dragged is kept.
    static let navigatorDefault: CGFloat = 320
    /// The navigator's widest, however far it's dragged: past this, it's a
    /// main area of its own and not a list. 600, wide enough that no width
    /// dragged before there was a maximum is cut (ov-177 review).
    static let navigatorMaximum: CGFloat = 600
    /// The navigator's divider.
    static let divider: CGFloat = 1

    /// The narrowest a tile column holding `columns` terminal columns can be
    /// at `cell`, in whole points: `floor((W − 40) / cell)` is at least
    /// `columns` from here up.
    static func width(columns: Int, cell: CGFloat) -> CGFloat {
        (chrome + CGFloat(columns) * cell).rounded(.up)
    }

    static func openedMinimum(cell: CGFloat = defaultCell) -> CGFloat {
        width(columns: openedColumns, cell: cell)
    }

    /// The navigator's width in a detail `width` pt wide: the remembered
    /// width, held between its minimum and what leaves the main area its
    /// own. Never under its minimum: a detail narrower than both gives way
    /// in the main area.
    static func navigatorWidth(_ remembered: CGFloat, width: CGFloat, cell: CGFloat = defaultCell) -> CGFloat {
        let most = width - divider - openedMinimum(cell: cell)
        return max(navigatorMinimum, min(remembered, most, navigatorMaximum))
    }

    /// What's drawn (spec §4.3, ov-92).
    struct Arrangement: Equatable {
        enum Conversation: Equatable {
            /// In the main area: the orchestrator is selected.
            case main
            /// Its own column at the trailing edge, beside the canvas
            /// (ov-298): on screen whatever is selected.
            case column
            /// Mounted and kept, out of sight: something else is selected,
            /// or Focus.
            case hidden
            /// None to draw: a runner without `workstreams`.
            case none
        }

        var conversation: Conversation
        /// A task or a worktree is selected, in the main area.
        var opened: Bool
        /// The navigator is drawn: not in Focus, nor for a loose worktree
        /// with no board.
        var navigator: Bool
        /// The main area is a canvas beside the chat column (ov-298): the
        /// plan, or what's opened in its place.
        var canvas = false
        /// The navigator floats over the canvas rather than taking room
        /// beside it: a window too narrow for all three (ov-298).
        var floats = false

        /// Whether the orchestrator is on screen.
        var showsConversation: Bool { conversation == .main || conversation == .column }

        static let workspace = Arrangement(conversation: .main, opened: false, navigator: true)
        static let opened = Arrangement(conversation: .hidden, opened: true, navigator: true)
        static let alone = Arrangement(conversation: .hidden, opened: true, navigator: false)
    }

    /// What's drawn for what's selected.
    ///
    /// - Nothing opened: the orchestrator, in the main area.
    /// - A task or a worktree (`opened`): it, with the orchestrator hidden.
    /// - `focused` is Focus (⌃⌘↩): what's opened alone, without the
    ///   navigator. It means nothing with nothing open.
    static func layout(
        opened: Bool, hasConversation: Bool = true, hasBoard: Bool = true, focused: Bool = false
    ) -> Arrangement {
        let focused = focused && opened
        let conversation: Arrangement.Conversation = !hasConversation ? .none : opened ? .hidden : .main
        return Arrangement(conversation: conversation, opened: opened, navigator: hasBoard && !focused)
    }

    // MARK: - The canvas and the chat column (ov-298, Concept A)

    /// The chat column's width in terminal columns until its edge is
    /// dragged: 88, about 720 pt at the default font.
    static let chatColumnsDefault = 88
    /// How narrow and how wide a drag takes it, in terminal columns: no
    /// narrower than the main area's own least (`openedColumns`).
    static let chatColumnsMinimum = openedColumns
    static let chatColumnsMaximum = 120
    /// The canvas's least: two theme cards side by side.
    static let canvasMinimum: CGFloat = 440
    /// Below this window width the navigator stops taking room and floats
    /// over the canvas when shown (⌘B), at the least: wider when the
    /// navigator or the chat is (`Canvas.navigatorBeside`).
    static let navigatorFloatsBelow: CGFloat = 1500
    /// Below this the canvas folds away, at the least: the chat takes the
    /// main area with the plan's strip, the navigator floats over it when
    /// shown, and what's opened replaces the chat as before.
    static let canvasFoldsBelow: CGFloat = 1180

    /// A remembered chat width, held to whole columns in its range.
    static func chatColumns(_ remembered: Double) -> Int {
        guard remembered.isFinite else { return chatColumnsDefault }
        return min(max(Int(remembered.rounded()), chatColumnsMinimum), chatColumnsMaximum)
    }

    /// What decides where the canvas, the chat and the navigator go
    /// (ov-298): the navigator's width and the chat's columns as kept, at
    /// the terminal's cell. Each tier starts where its parts fit, so the
    /// chat is its width in whole columns in every tier that has a canvas,
    /// whatever the navigator's width (train 1004r, P2).
    struct Canvas: Equatable {
        var navigator: CGFloat = navigatorDefault
        var chatColumns: Int = chatColumnsDefault
        var cell: CGFloat = defaultCell

        /// The chat column's width: its columns, whole.
        var chat: CGFloat { width(columns: chatColumns, cell: cell) }
        /// The navigator as it would be drawn beside the canvas.
        var navigatorShown: CGFloat { max(navigatorMinimum, min(navigator, navigatorMaximum)) }

        /// Whether `width` has room for the canvas beside the chat.
        func hasCanvas(_ width: CGFloat) -> Bool { width >= max(canvasFoldsBelow, chat + divider + canvasMinimum) }

        /// Whether `width` has room for the navigator beside the canvas
        /// and the chat; else it floats when shown.
        func navigatorBeside(_ width: CGFloat) -> Bool {
            width >= max(navigatorFloatsBelow, navigatorShown + divider + canvasMinimum + divider + chat)
        }

        /// Whether the navigator floats, rather than taking room, at `width`.
        func navigatorFloats(_ width: CGFloat) -> Bool { !navigatorBeside(width) }
    }

    /// What's drawn for a scene `width` wide: with `canvas`, a workspace
    /// with an orchestrator gets the canvas and the chat column where they
    /// fit, and the chat alone, the canvas folded, where they don't; the
    /// navigator floats (shown only while `floating`) wherever it doesn't
    /// fit beside them. Without, `layout`. One rule for the view that draws
    /// it and the window that decides what's seen and keyed (train 1004r, P1).
    static func arrangement(
        width: CGFloat, canvas: Canvas?, opened: Bool, hasConversation: Bool, hasBoard: Bool, boardExists: Bool,
        floating: Bool, focused: Bool
    ) -> Arrangement {
        guard let canvas, hasConversation else {
            return layout(opened: opened, hasConversation: hasConversation, hasBoard: hasBoard, focused: focused)
        }
        guard canvas.hasCanvas(width) else {
            var folded = layout(
                opened: opened, hasConversation: true, hasBoard: boardExists && floating, focused: focused)
            folded.floats = true
            return folded
        }
        let floats = canvas.navigatorFloats(width)
        return canvasLayout(
            opened: opened, hasBoard: floats ? boardExists && floating : hasBoard, focused: focused, floats: floats)
    }

    /// What's drawn with a canvas beside the chat column: the chat on
    /// screen whatever is opened, except in Focus, which gives what's
    /// opened the whole width. `hasBoard` is whether the navigator is shown
    /// (`floats` says where).
    static func canvasLayout(opened: Bool, hasBoard: Bool, focused: Bool, floats: Bool) -> Arrangement {
        let focused = focused && opened
        return Arrangement(
            conversation: focused ? .hidden : .column, opened: opened, navigator: hasBoard && !focused, canvas: true,
            floats: floats)
    }

    /// Where each part sits, in points from the detail's leading edge.
    struct Frames: Equatable {
        /// The navigator's width, kept while it's put away.
        var navigator: CGFloat
        /// The main area's leading edge and width: past the navigator and
        /// its divider, or the whole detail without it. The orchestrator
        /// and what's opened are both this wide.
        var mainX: CGFloat
        var main: CGFloat
        /// The chat column's leading edge and width, with a canvas; zero
        /// without one. With a canvas, `main` is the canvas, overlapping the
        /// chat's leading gutter by `Pane.inset` while the chat is beside it.
        var chatX: CGFloat = 0
        var chat: CGFloat = 0
    }

    /// The frames `arrangement` puts its parts at in a detail `width` pt
    /// wide, with the navigator `remembered` pt wide when it can be.
    static func frames(
        width: CGFloat, arrangement: Arrangement, navigator remembered: CGFloat, cell: CGFloat = defaultCell,
        chatColumns: Int = chatColumnsDefault
    ) -> Frames {
        let navigator = navigatorWidth(remembered, width: width, cell: cell)
        let mainX = arrangement.navigator && !arrangement.floats ? navigator + divider : 0
        guard arrangement.canvas else { return Frames(navigator: navigator, mainX: mainX, main: max(0, width - mainX)) }
        // The chat's width is its columns, whole, with the navigator
        // floating, put away or beside it, and in Focus, where it's hidden:
        // only a drag of its edge changes it. The tiers leave it room.
        let chat = self.width(columns: chatColumns, cell: cell)
        guard arrangement.conversation == .column else {
            return Frames(navigator: navigator, mainX: mainX, main: max(0, width - mainX), chatX: width, chat: chat)
        }
        let chatX = width - chat
        // The canvas runs under the chat's leading gutter, so its card ends
        // where the chat's frame starts and the two cards are one gutter
        // apart, as tiled panes' are, not two gutters and the divider.
        return Frames(navigator: navigator, mainX: mainX, main: max(0, chatX + Pane.inset - mainX), chatX: chatX, chat: chat)
    }
}

/// Where the keyboard goes for each command that moves it (ov-89 review,
/// ov-92), as values: Focus, whether the orchestrator is selected, and the
/// keyboard's target, from what's open and whether it's in Focus.
/// `ContentView` does what these say and decides nothing.
///
/// The rule under all of it: the keyboard never stays on a navigator that
/// isn't drawn, and a command with nothing to act on changes nothing.
extension WorkspaceNavigation {
    /// Where the keyboard goes.
    enum KeyTarget: Equatable {
        /// The navigator's list.
        case board
        /// What the main area shows: what's opened, else the orchestrator.
        case main
        /// The orchestrator.
        case conversation
        /// What's opened.
        case opened
        /// Where it is.
        case unchanged
    }

    struct BoardState: Equatable {
        /// A task or a worktree is open.
        var opened: Bool
        /// Focus (⌃⌘↩).
        var focus = false
        /// The navigator has the keyboard.
        var onBoard = false
        /// There's a navigator at all: not for a loose worktree with no
        /// board.
        var hasNavigator = true

        /// Whether the navigator is drawn for its list to take the keyboard.
        var boardInSight: Bool { hasNavigator && !focus }
    }

    enum BoardCommand: Equatable {
        /// ⌥⌘1: the orchestrator selected.
        case conversation
        /// ⌥⌘2: the navigator.
        case board
        /// ⌥⌘3: the main area.
        case task
        /// ⌃⌘↩.
        case toggleFocus
        /// A row in the navigator clicked, or glanced at with ↑ or ↓.
        case choose(glance: Bool)
        /// What's opened closed: the ×, a click on it in the navigator.
        case close
    }

    struct BoardStep: Equatable {
        var focus: Bool
        var keyboard: KeyTarget
        /// The orchestrator becomes the selection.
        var selectsOrchestrator = false
    }

    /// A task opened from outside its navigator, by the palette or a click
    /// on its notice: opened as a row clicked in it is
    /// (`choosing(toggles: false)`, `boardStep(.choose)`), with the
    /// navigator taking the keyboard, never closed for being open already.
    /// `s` is the window as it is, perhaps another workspace's or a loose
    /// worktree's; the task's workspace always has its navigator.
    static func openingTask(
        _ task: String, host: String, workspace: String, from s: BoardState
    ) -> (selection: Selection, step: BoardStep) {
        var there = s
        there.hasNavigator = true
        return (
            choosing(task: task, host: host, workspace: workspace, from: nil, toggles: false),
            boardStep(.choose(glance: false), from: there)
        )
    }

    static func boardStep(_ command: BoardCommand, from s: BoardState) -> BoardStep {
        let nothing = BoardStep(focus: s.focus, keyboard: .unchanged)
        switch command {
        case .conversation:
            return BoardStep(focus: false, keyboard: .conversation, selectsOrchestrator: true)
        case .board:
            // Out of Focus, so it's drawn to take it.
            guard s.hasNavigator else { return nothing }
            return BoardStep(focus: false, keyboard: .board)
        case .task:
            return BoardStep(focus: s.focus, keyboard: .main)
        case .toggleFocus:
            guard s.opened else { return nothing }
            // Into Focus the navigator goes: the keyboard follows to what's
            // opened if it had it.
            if !s.focus { return BoardStep(focus: true, keyboard: s.onBoard ? .opened : .unchanged) }
            return BoardStep(focus: false, keyboard: .unchanged)
        case .choose:
            // A row chosen in the navigator keeps it the keyboard, to go on.
            return BoardStep(focus: false, keyboard: s.hasNavigator ? .board : .main)
        case .close:
            return BoardStep(focus: false, keyboard: s.hasNavigator ? .board : .main)
        }
    }
}
