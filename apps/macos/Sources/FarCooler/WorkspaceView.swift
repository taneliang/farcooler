import AgentKit
import AppKit
import SwiftUI

/// A workspace in the detail (spec §4.3, ov-92): the navigator on the left,
/// and the main area beside it, which shows what's selected there: the
/// orchestrator, or the task or worktree opened in its place.
///
/// Nothing here is navigation. The window says what's open (`opened`, nil
/// for the orchestrator) and this view places it, by `WorkspaceColumns`'
/// values for its own measured width, never the window's, which the detail
/// shares with the sidebar.
///
/// A selection change is drawn on the next frame, with no fade
/// (`WorkspaceMotion.swap`, ov-293): the orchestrator and what's opened
/// swapping as the selection moves between them, one task switched for
/// another in place. Only the navigator leaving for Focus moves, on one
/// spring (`WorkspaceMotion.spring`), and not under Reduce Motion. Every
/// part stays mounted while it moves,
/// the orchestrator's terminal included: it's mounted once, at the main
/// area's width, and kept, hidden while something else is selected, so
/// coming back to it is instant and nothing in it re-wraps. Nothing waits on
/// the motion: a click mid-flight retargets it, and what takes clicks and
/// the keyboard follows the window's state at once, not the motion's
/// (`WorkspaceStage`).
///
/// With `split` (ov-298, Concept A), a window wide enough has a canvas and a
/// chat column: the plan is the canvas's home, what's opened replaces it
/// there, and the orchestrator sits beside it at the trailing edge, a fixed
/// whole number of terminal columns wide, on screen whatever is opened.
/// Widening the window grows the canvas only, so the orchestrator never
/// re-wraps but for a drag of its edge. Narrower, the navigator floats when
/// shown; narrower still, the canvas folds away and the chat carries the
/// plan's strip, with the plan peeked over it (`WorkspaceColumns`).
struct WorkspaceView<
    Item: Hashable, Conversation: View, Navigator: View, Crumbs: View, Opened: View
>: View {
    /// What's open in the main area: a task or a worktree, or nil for the
    /// orchestrator. Its identity is what's switched: a new one replaces
    /// it, the same one with another pane named stays.
    let opened: Item?
    /// Whether this workspace has a conversation at all: not a repository's
    /// implicit workspace on a runner without `workstreams`.
    let hasConversation: Bool
    /// Whether there's a navigator to draw: not for a loose worktree whose
    /// board can't be found.
    var hasBoard = true
    /// The terminal font's cell width: the main area's minimum is in
    /// columns.
    let cell: CGFloat
    /// Focus (⌃⌘↩): what's opened alone, without the navigator.
    let focused: Bool
    /// The navigator's width as its trailing edge was last dropped. Kept
    /// per device by the window.
    @Binding var navigatorWidth: Double
    @ViewBuilder let conversation: () -> Conversation
    @ViewBuilder let navigator: () -> Navigator
    /// The jump bar over what's opened: Workspace › Task › Worktree, and its
    /// close button.
    @ViewBuilder let breadcrumb: (Item) -> Crumbs
    /// What's opened, drawn for `Item`: the one the window has open, or the
    /// one leaving, which takes no keyboard and isn't seen. The flag says
    /// whether it has settled (`WorkspaceMotion.settle`): until it has, only
    /// what's cheap is drawn, its header and its text, and nothing that
    /// mounts a terminal or reads the runner. A click settles at once; only
    /// a quick walk through the list waits.
    @ViewBuilder let detail: (Item, Bool) -> Opened
    /// How the navigator moves for Focus: `WorkspaceMotion.spring`, slowed
    /// only by a test that reads it mid-flight.
    var motion: Animation = WorkspaceMotion.spring
    /// How a selection change is drawn: `WorkspaceMotion.swap`, no motion at
    /// all, given one only by a test that reads it mid-flight.
    var swap: Animation? = WorkspaceMotion.swap
    /// The canvas and the chat column, where the window is wide enough.
    var split = false
    /// The canvas's home, with nothing opened: the plan.
    var home: () -> AnyView = { AnyView(EmptyView()) }
    /// The chat's width in terminal columns, as its edge was last dropped.
    var chatColumns: Binding<Double> = .constant(Double(WorkspaceColumns.chatColumnsDefault))
    /// Whether a board exists to float the navigator for, and whether ⌘B
    /// has floated it, in a window too narrow for it beside the canvas.
    var boardExists = true
    var navigatorFloating: Binding<Bool> = .constant(false)
    /// With the canvas folded away: the plan's strip over the chat, and
    /// the plan peeked over it while `peeking`.
    var strip: () -> AnyView = { AnyView(EmptyView()) }
    var peek: () -> AnyView = { AnyView(EmptyView()) }
    var peeking: Binding<Bool> = .constant(false)

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// What the motion is drawing, a step behind the window's state. See
    /// `WorkspaceStage`.
    @State private var stage: WorkspaceStage<Item>
    /// What's opened that has settled: the window's, once it's stayed put
    /// for `WorkspaceMotion.settle`. Glancing through tasks with a held
    /// arrow passes the rest without mounting or reading them.
    @State private var settled: Item?
    @State private var settling: Task<Void, Never>?
    /// When the last switch from one opened item to another came: one
    /// sooner than `WorkspaceMotion.settle` after it is a step in a walk.
    @State private var lastSwitch: ContinuousClock.Instant?
    /// The navigator's width while its trailing edge is dragged.
    @State private var dragging: CGFloat?
    /// The navigator's width when the drag began.
    @State private var dragStart: CGFloat?
    /// The chat's width in columns while its edge is dragged, and at the
    /// drag's start.
    @State private var chatDragging: Int?
    @State private var chatDragStart: Int?

    init(
        opened: Item?, hasConversation: Bool, hasBoard: Bool = true, cell: CGFloat, focused: Bool,
        navigatorWidth: Binding<Double>,
        @ViewBuilder conversation: @escaping () -> Conversation, @ViewBuilder navigator: @escaping () -> Navigator,
        @ViewBuilder breadcrumb: @escaping (Item) -> Crumbs,
        @ViewBuilder detail: @escaping (Item, Bool) -> Opened, motion: Animation = WorkspaceMotion.spring,
        swap: Animation? = WorkspaceMotion.swap, split: Bool = false,
        home: @escaping () -> AnyView = { AnyView(EmptyView()) },
        chatColumns: Binding<Double> = .constant(Double(WorkspaceColumns.chatColumnsDefault)),
        boardExists: Bool = true, navigatorFloating: Binding<Bool> = .constant(false),
        strip: @escaping () -> AnyView = { AnyView(EmptyView()) },
        peek: @escaping () -> AnyView = { AnyView(EmptyView()) }, peeking: Binding<Bool> = .constant(false)
    ) {
        self.opened = opened
        self.hasConversation = hasConversation
        self.hasBoard = hasBoard
        self.cell = cell
        self.focused = focused
        _navigatorWidth = navigatorWidth
        self.conversation = conversation
        self.navigator = navigator
        self.breadcrumb = breadcrumb
        self.detail = detail
        self.motion = motion
        self.swap = swap
        self.split = split
        self.home = home
        self.chatColumns = chatColumns
        self.boardExists = boardExists
        self.navigatorFloating = navigatorFloating
        self.strip = strip
        self.peek = peek
        self.peeking = peeking
        // Drawn as it is from the first frame: a window reopening on a task
        // doesn't fade the orchestrator out.
        _stage = State(initialValue: WorkspaceStage(open: opened, focused: focused))
        _settled = State(initialValue: opened)
    }

    private func arrangement(open: Bool, focused: Bool, width: CGFloat) -> WorkspaceColumns.Arrangement {
        WorkspaceColumns.arrangement(
            width: width, canvas: split ? sizing : nil, opened: open, hasConversation: hasConversation,
            hasBoard: hasBoard, boardExists: boardExists, floating: navigatorFloating.wrappedValue, focused: focused)
    }

    /// The canvas's tiers, from the widths as kept, or as dragged.
    private var sizing: WorkspaceColumns.Canvas {
        WorkspaceColumns.Canvas(navigator: dragging ?? CGFloat(navigatorWidth), chatColumns: columns, cell: cell)
    }

    /// The chat's columns: as dragged, else as kept.
    private var columns: Int { chatDragging ?? WorkspaceColumns.chatColumns(chatColumns.wrappedValue) }

    var body: some View {
        GeometryReader { proxy in
            let width = proxy.size.width
            let height = proxy.size.height
            // The window's state: what takes clicks, the keyboard and the
            // accessibility tree's notice, and what's on screen.
            let now = arrangement(open: opened != nil, focused: focused, width: width)
            // The motion's: where things are drawn.
            let drawn = arrangement(open: stage.open != nil, focused: stage.focused, width: width)
            let remembered = dragging ?? CGFloat(navigatorWidth)
            let frames = WorkspaceColumns.frames(
                width: width, arrangement: drawn, navigator: remembered, cell: cell, chatColumns: columns)
            // The orchestrator's one width: the main area's beside the
            // navigator, in Focus too, where it's hidden, so going into Focus
            // and out never resizes its tmux window (ov-92 review). Beside
            // the canvas, the chat column's, whatever is opened.
            let kept = WorkspaceColumns.frames(
                width: width, arrangement: arrangement(open: false, focused: false, width: width), navigator: remembered,
                cell: cell, chatColumns: columns)
            let canvas = drawn.canvas
            ZStack(alignment: .topLeading) {
                main(drawn: drawn, height: height)
                    .frame(width: frames.main, height: height)
                    .clipShape(Rectangle())
                    .offset(x: frames.mainX)
                // The orchestrator: in the main area, or its own column
                // beside the canvas. One view in one place in the tree either
                // way, so crossing the width that folds the canvas moves it
                // and never mounts it again.
                conversationLayer(now: now, drawn: drawn, kept: canvas ? kept.chat : kept.main)
                    .frame(width: canvas ? frames.chat : frames.main, height: height, alignment: .topLeading)
                    .clipShape(Rectangle())
                    .offset(x: canvas ? frames.chatX : frames.mainX)
                if canvas, drawn.conversation == .column {
                    chatEdge(height: height, at: frames.chatX, width: width, beside: frames.mainX)
                }
                HStack(spacing: 0) {
                    navigator()
                        .frame(width: frames.navigator)
                        .frame(maxHeight: .infinity)
                        // Floating over the canvas, on the window's own plane.
                        .background { if drawn.floats { WindowPlane() } }
                    // The plane shows between the navigator and the paper
                    // beside it: the paper's edge is the boundary, not a rule.
                    Color.clear.frame(width: WorkspaceColumns.divider)
                }
                .frame(height: height)
                .offset(x: drawn.navigator ? 0 : -(frames.navigator + WorkspaceColumns.divider + WorkspaceMotion.overhang))
                .allowsHitTesting(now.navigator)
                .accessibilityHidden(!now.navigator)
                .accessibilityIdentifier("workspace-board")
                navigatorEdge(
                    height: height, at: frames.navigator, current: frames.navigator, shown: drawn.navigator,
                    live: now.navigator, width: width)
            }
            .frame(width: width, height: height, alignment: .topLeading)
            // Only the navigator springs, for Focus; a selection change is
            // in the transaction `onChange(of: opened)` gives it.
            .animation(reduceMotion ? nil : motion, value: drawn.navigator)
            .clipShape(Rectangle())
            .contentShape(Rectangle())
            // One frosted plane behind the navigator, the gutters and the
            // headers; nothing drawn over it but the work's own paper.
            .background { WindowPlane().ignoresSafeArea() }
            .preference(key: WorkspaceArrangementPreference.self, value: now)
            .preference(key: WorkspaceWidthPreference.self, value: width)
        }
        .onChange(of: opened) { _, next in
            settle(next, switching: stage.open != nil && next != nil)
            let generation = stage.generation + 1
            if let swap {
                // Let go of what a close was drawing once its motion ends.
                withAnimation(swap) {
                    stage.show(next)
                } completion: {
                    stage.settle(generation)
                }
            } else {
                var still = Transaction(animation: nil)
                still.disablesAnimations = true
                withTransaction(still) {
                    stage.show(next)
                    stage.settle(generation)
                }
            }
        }
        .onChange(of: focused) { _, focused in
            withAnimation(reduceMotion ? nil : motion) { stage.focused = focused }
        }
        // A floated navigator goes once a row in it opens something.
        .onChange(of: opened) { _, _ in navigatorFloating.wrappedValue = false }
    }

    /// The main area, or the canvas beside the chat: what's opened, over
    /// the canvas's home (the plan) when there's a canvas. The orchestrator
    /// is `conversationLayer`'s, drawn beside or under this.
    private func main(drawn: WorkspaceColumns.Arrangement, height: CGFloat) -> some View {
        ZStack(alignment: .topLeading) {
            if drawn.canvas {
                // The canvas's home, the plan, while nothing is opened.
                home()
                    .opacity(stage.open == nil ? 1 : 0)
                    .allowsHitTesting(opened == nil)
                    .accessibilityHidden(opened != nil)
                    .environment(\.outOfSight, opened != nil)
                    .accessibilityIdentifier("workspace-home")
            } else if !hasConversation {
                WorkspaceMain.nothingOpen
                    .opacity(stage.open == nil ? 1 : 0)
                    .accessibilityHidden(opened != nil)
            }
            openedPane(height: height)
                // One piece: a task switched in is placed inside it and
                // moves with it.
                .geometryGroup()
                .opacity(stage.open != nil ? 1 : 0)
                .allowsHitTesting(opened != nil)
                .accessibilityHidden(opened == nil)
        }
    }

    /// The orchestrator, mounted and kept at its one width `kept`: in the
    /// main area while it's selected, or in its column beside the canvas
    /// (ov-298), and hidden otherwise. With the canvas folded away, it
    /// carries the plan's strip, and the plan peeked over it.
    @ViewBuilder
    private func conversationLayer(
        now: WorkspaceColumns.Arrangement, drawn: WorkspaceColumns.Arrangement, kept: CGFloat
    ) -> some View {
        let shown = now.showsConversation
        if hasConversation {
            // Folded, with a plan: its one line on a row of its own under
            // the terminal, never over it, so the terminal's last rows (its
            // input) stay in sight; it costs rows, never columns (train
            // 1004r, P4). Always a stack, so the terminal is one view in one
            // place whether the strip shows or not.
            VStack(spacing: 0) {
                conversation()
                    .frame(width: kept)
                    .frame(maxHeight: .infinity)
                    .probed("workspace-conversation-body")
                if split, !drawn.canvas { strip() }
            }
                .overlay {
                    if split, !drawn.canvas, peeking.wrappedValue { peek() }
                }
                .opacity(drawn.showsConversation ? 1 : 0)
                .allowsHitTesting(shown)
                .accessibilityHidden(!shown)
                // Nothing in it takes the keyboard while it's hidden:
                // not the terminal, not a chat composer, not a control.
                .environment(\.outOfSight, !shown)
                .disabled(!shown)
                .accessibilityIdentifier("workspace-conversation")
        }
    }

    /// The chat column's leading edge, dragged to set its width in whole
    /// terminal columns, kept once dropped; double-clicked, back to 88.
    /// Dragged left, the chat widens.
    private func chatEdge(height: CGFloat, at x: CGFloat, width: CGFloat, beside: CGFloat) -> some View {
        Color.clear
            .frame(width: WorkspaceColumns.divider + 2 * WorkspaceMotion.grip, height: height)
            .contentShape(Rectangle())
            .pointerStyle(.columnResize)
            .gesture(
                DragGesture(minimumDistance: 1, coordinateSpace: .global)
                    .onChanged { value in
                        let start = chatDragStart ?? columns
                        chatDragStart = start
                        chatDragging = WorkspaceColumns.chatColumns(
                            Double(start) - Double(value.translation.width / max(cell, 1)))
                    }
                    .onEnded { _ in
                        if let chatDragging { chatColumns.wrappedValue = Double(chatDragging) }
                        chatDragging = nil
                        chatDragStart = nil
                    })
            .onTapGesture(count: 2) { chatColumns.wrappedValue = Double(WorkspaceColumns.chatColumnsDefault) }
            .offset(x: x - WorkspaceColumns.divider - WorkspaceMotion.grip)
            .accessibilityHidden(true)
            .probed("workspace-chat-edge")
    }

    /// `next` settles: at once when it opens from the orchestrator, goes
    /// back to it, or is a click on another; and when it's a step in a quick
    /// walk through the list, one sooner than `WorkspaceMotion.settle` after
    /// the last, only once it has stayed put that long.
    private func settle(_ next: Item?, switching: Bool) {
        settling?.cancel()
        let now = ContinuousClock.now
        let walking = switching && lastSwitch.map { $0.duration(to: now) < WorkspaceMotion.settle } == true
        lastSwitch = switching ? now : nil
        guard walking else {
            settled = next
            return
        }
        settling = Task { @MainActor in
            try? await Task.sleep(for: WorkspaceMotion.settle)
            guard !Task.isCancelled else { return }
            settled = next
        }
    }

    /// What's opened, under its jump bar: the window's, or the one leaving.
    /// Another one switched in replaces it on the next frame, with no fade:
    /// a content swap, as Apple's apps make one.
    private func openedPane(height: CGFloat) -> some View {
        ZStack {
            if let shown = stage.drawn {
                VStack(spacing: 0) {
                    breadcrumb(shown)
                    detail(shown, settled == shown)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                .id(shown)
                .transition(.identity)
                // Leaving, nothing in it takes the keyboard or a click.
                .environment(\.outOfSight, opened != shown)
                .allowsHitTesting(opened == shown)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityIdentifier("workspace-opened")
    }

    /// The navigator's trailing edge, dragged to set its width, kept once
    /// dropped. Dragged right, the navigator widens. The line itself is the
    /// navigator's divider; this is the grip over it.
    private func navigatorEdge(
        height: CGFloat, at x: CGFloat, current: CGFloat, shown: Bool, live: Bool, width: CGFloat
    ) -> some View {
        Color.clear
            .frame(width: WorkspaceColumns.divider + 2 * WorkspaceMotion.grip, height: height)
            .contentShape(Rectangle())
            .pointerStyle(.columnResize)
            .gesture(
                // Measured in the window: the edge moves with the drag, so
                // its own coordinates would chase it.
                DragGesture(minimumDistance: 1, coordinateSpace: .global)
                    .onChanged { value in
                        if dragStart == nil { dragStart = current }
                        dragging = WorkspaceColumns.navigatorWidth(
                            (dragStart ?? current) + value.translation.width, width: width, cell: cell)
                    }
                    .onEnded { _ in
                        if let dragging { navigatorWidth = Double(dragging) }
                        dragging = nil
                        dragStart = nil
                    })
            .offset(x: x - WorkspaceMotion.grip)
            .opacity(shown ? 1 : 0)
            .allowsHitTesting(live)
            .accessibilityHidden(true)
    }
}

/// What the main area shows with nothing selected where no orchestrator
/// can run: a repository's board on a runner without `workstreams`, which
/// has no workspaces to seat one in. It's the orchestrator's place, so it's
/// the orchestrator's empty state, saying why there's nothing to start. A
/// workspace that can have one and hasn't draws the conversation's own
/// (`ConversationPlaceholder`), with Start Orchestrator and Use a Running
/// Terminal….
enum WorkspaceMain {
    static let title = "No Orchestrator"
    static let message = "Update Far Cooler on this runner to use an orchestrator here."

    @MainActor static var nothingOpen: some View {
        VStack(spacing: 10) {
            Text(title).font(.headline)
            Text(message)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 320)
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// How a workspace's structure moves (ov-85, ov-293), and what the
/// view needs to keep its parts in reach while they move.
enum WorkspaceMotion {
    /// The navigator leaving for Focus, and the board's lists (ov-84's).
    static let spring = Animation.spring(response: 0.32, dampingFraction: 0.86)
    /// A selection change, between the orchestrator, a task and a worktree:
    /// none (ov-293). It's drawn on the next frame. The spring this was
    /// took about 200 ms to be 95% drawn and barely moved in its first
    /// 30 ms, which read as navigation lagging. It stays nil: a fade of
    /// any length goes red in `SelectionSwapTimingTests`, which allows no
    /// frame part of the way.
    static let swap: Animation? = nil
    /// Past the leading edge when put away, so its divider is out of
    /// sight too.
    static let overhang: CGFloat = 24
    /// Each side of the navigator's trailing edge that takes a drag.
    static let grip: CGFloat = 3
    /// How soon after the last a switch is a step in a walk, and how long
    /// such a step stays put before it's settled, and its terminal is
    /// mounted and its record read. A lone click settles at once.
    static let settle: Duration = .milliseconds(150)
}

/// What a workspace's detail draws, as the window opens a task, switches to
/// another and closes it, kept apart from the window's state so the motion
/// can run a step behind it without ever holding it up (ov-85).
///
/// `open` is what the motion is heading for, set in the same transaction as
/// the spring; `drawn` is what's drawn in the detail, which outlives `open`
/// while it slides away and is let go only when that close settles. A close
/// that settles after something else has opened lets go of nothing: each
/// `show` counts a generation, and `settle` acts only on the latest.
struct WorkspaceStage<Item: Hashable>: Equatable {
    private(set) var open: Item?
    private(set) var drawn: Item?
    private(set) var generation = 0
    /// Focus, as the motion is drawing it.
    var focused: Bool

    init(open: Item?, focused: Bool = false) {
        self.open = open
        self.drawn = open
        self.focused = focused
    }

    /// The window opened `item`, switched to it, or, with nil, closed.
    mutating func show(_ item: Item?) {
        generation += 1
        open = item
        if let item { drawn = item }
    }

    /// The motion `generation` started has finished: a close lets go of
    /// what it was drawing, unless something has opened since.
    mutating func settle(_ generation: Int) {
        guard generation == self.generation, open == nil else { return }
        drawn = nil
    }
}

extension EnvironmentValues {
    /// Mounted but out of sight: the orchestrator kept while something else
    /// is selected (ov-84, ov-92). A terminal or a composer under it takes
    /// no keyboard, by click, Tab or its own claim, and lets go of one it
    /// holds.
    @Entry var outOfSight = false
}

/// What an AppKit view that can take the keyboard does about `outOfSight`.
enum KeyboardFence {
    /// Called as `takesKeyboard` turns off: the window's first responder,
    /// if it's `view`, lets go, and what's selected takes it up from there.
    @MainActor
    static func release(_ view: NSView) {
        guard let window = view.window, window.firstResponder === view else { return }
        window.makeFirstResponder(nil)
    }
}

/// The orchestrator's terminal, kept mounted whether or not it's selected
/// (ov-92): what it draws, and when it's seen, watched and typed into.
enum KeptOrchestrator {
    /// The layout the conversation's view draws, and whether it's on
    /// screen. It draws the conversation's layout selected or not, so it's
    /// one terminal view; it's on screen, seen, watched and given the
    /// keyboard only when `visible` (`WorkspaceScreen.visible`) has it,
    /// which is while it's selected.
    static func conversation(
        visible: [ShownLayout], drawable: [ShownLayout]
    ) -> (layout: ShownLayout?, onScreen: Bool) {
        let shown = visible.contains { $0.column == .conversation }
        return (drawable.first { $0.column == .conversation }, shown)
    }

    /// Whether a terminal view drawing `layout` has the keyboard: never
    /// while it's off screen, else as `WorkspaceScreen.hasKeyboard` says.
    static func takesKeyboard(_ layout: ShownLayout, onScreen: Bool, key: PaneRef?, onBoard: Bool) -> Bool {
        onScreen && WorkspaceScreen.hasKeyboard(layout, key: key, onBoard: onBoard)
    }
}

/// The breadcrumb over what's opened, a jump bar as Xcode's: each level
/// down to the one you're at, and the close button at the far end (ov-85).
/// Drawn from `pieces`, one style each (`JumpBar`), in the shared header
/// (`columnHeader`).
///
/// Every segment is a menu (ov-192): a click opens it, on the item that
/// segment stands for, checked, so going up a level is a click and Return.
/// Its siblings, and where it helps its children, are `JumpMenus`'. The
/// keyboard reaches it with ⌘L (`focusRequest`) and moves by
/// `JumpBarKeys`.
struct DrillBreadcrumb: View {
    let crumbs: [WorkspaceNavigation.Crumb]
    /// The trailing worktree segment (ov-86, ov-185): "⎇ tax-rounding ⌄".
    /// Standing for a worktree opened whole, in place of its crumb, it's a
    /// menu of what's beside it: the workspace's worktrees, or its task's.
    /// After a task's crumb it's that task's own: a way into its one, or a
    /// menu of its several (`WorkspaceWorktrees.segment`).
    var worktrees: WorktreeCrumb?
    var onGo: (ContentView.Selection) -> Void
    /// Close: what's opened goes, and the orchestrator is selected again.
    /// Nil where there's nothing to close to, and no button.
    var onClose: (() -> Void)?
    /// A worktree chosen from the segment, which may keep its task as the
    /// way back (`MenuItem.trail`); `onGo` with its target when nil.
    var onOpen: ((WorkspaceWorktrees.MenuItem) -> Void)? = nil
    /// Each crumb's menu, by its index (ov-192), built only when one opens
    /// or the bar takes the keyboard, never on a redraw. A crumb past
    /// `count` is a plain way back to its target.
    var menus = JumpMenuSource.none
    /// An item chosen from a menu.
    var onJump: (JumpTarget) -> Void = { _ in }
    /// Bumped by ⌘L: the bar takes the keyboard, on its last segment.
    var focusRequest = 0
    /// The keyboard left the bar with Esc, for what's opened to take back.
    var onLeave: () -> Void = {}
    /// Whether the bar has the keyboard, for the window's Esc-as-Back to
    /// leave its Esc alone.
    var onActive: (Bool) -> Void = { _ in }

    @State private var keys = JumpBarFocus.away
    /// The segments' menus while the bar is in use, built once for it.
    @State private var built: [JumpMenu]?
    @FocusState private var barFocused: Bool

    /// One mark the bar draws, and the style it's drawn in.
    struct Piece: Equatable {
        enum Kind: Equatable {
            case separator
            /// A crumb, by its index in `crumbs`, and its ⌄ where it has a
            /// menu (ov-267).
            case crumb(Int)
            case crumbCaret(Int)
            /// The worktree menu's icon, its title and its ⌄.
            case menuIcon, menuTitle, menuChevron
            /// The terminal open in the worktree, and its ⌄ (ov-267).
            case terminalTitle, terminalChevron
        }

        var kind: Kind
        var style: JumpBar.Style
    }

    /// Everything the bar draws, in order, each in its one style: what
    /// `ColumnHeaderTests` reads, and all the view draws from. The first
    /// `menus` crumbs have a menu, and a ⌄ to open it.
    static func pieces(_ crumbs: [WorkspaceNavigation.Crumb], worktrees: WorktreeCrumb?, menus: Int = 0) -> [Piece] {
        var out: [Piece] = []
        for (index, crumb) in crumbs.enumerated() {
            if index > 0 { out.append(Piece(kind: .separator, style: JumpBar.chevron)) }
            out.append(Piece(kind: .crumb(index), style: JumpBar.style(crumb.target == nil ? .current : .ancestor)))
            if index < menus { out.append(Piece(kind: .crumbCaret(index), style: JumpBar.chevron)) }
        }
        if let worktrees {
            let terminalHere = worktrees.terminal != nil
            if !crumbs.isEmpty { out.append(Piece(kind: .separator, style: JumpBar.chevron)) }
            out.append(Piece(kind: .menuIcon, style: JumpBar.icon(.menu, isHere: worktrees.isHere && !terminalHere)))
            out.append(Piece(kind: .menuTitle, style: JumpBar.style(.menu, isHere: worktrees.isHere && !terminalHere)))
            // A way into a task's one worktree is no menu, and has no ⌄.
            if worktrees.opens == nil { out.append(Piece(kind: .menuChevron, style: JumpBar.chevron)) }
            if terminalHere {
                out.append(Piece(kind: .separator, style: JumpBar.chevron))
                out.append(Piece(kind: .terminalTitle, style: JumpBar.style(.current)))
                out.append(Piece(kind: .terminalChevron, style: JumpBar.chevron))
            }
        }
        return out
    }

    /// How many segments the keyboard moves through.
    var segmentCount: Int {
        crumbs.count + (worktrees == nil ? 0 : 1) + (worktrees?.terminal == nil ? 0 : 1)
    }

    /// The segments' menus, built now and kept until the bar lets go.
    private func materialize() -> [JumpMenu] {
        if let built { return built }
        let made = Self.segmentMenus(crumbs: crumbs.count, menus: menus.build(), worktrees: worktrees)
        built = made
        return made
    }

    /// Open segment `index`'s menu, as a click does.
    private func open(segment index: Int) { keys = JumpBarKeys.open(index, menus: materialize()) }

    /// Each segment's menu, in the bar's order: the crumbs', then the
    /// worktree segment's. What the keyboard moves through.
    static func segmentMenus(crumbs: Int, menus: [JumpMenu], worktrees: WorktreeCrumb?) -> [JumpMenu] {
        var out = (0..<crumbs).map { menus.indices.contains($0) ? menus[$0] : JumpMenu([]) }
        if let worktrees { out.append(worktrees.jumpMenu) }
        if let terminal = worktrees?.terminal { out.append(terminal.jumpMenu) }
        return out
    }

    var body: some View {
        let pieces = Self.pieces(crumbs, worktrees: worktrees, menus: menus.count)
        let segments = built ?? []
        HStack(spacing: JumpBar.spacing) {
            // The segments on one center and one baseline: every piece is one
            // `JumpBar.cell` tall, its baseline at `JumpBar.baseline` in it, so
            // labels, carets and separators share a line (ov-290).
            HStack(alignment: .center, spacing: JumpBar.spacing) {
            ForEach(Array(pieces.enumerated()), id: \.offset) { offset, piece in
                switch piece.kind {
                case .separator:
                    Image(systemName: JumpBar.separatorGlyph)
                        .font(piece.style.font)
                        .foregroundStyle(piece.style.color)
                        .baselineProbed("jump-separator-\(offset)-baseline")
                        .jumpCell()
                        .frame(width: JumpBar.separatorWidth)
                        .accessibilityHidden(true)
                        .identified("jump-separator-\(offset)")
                case .crumb(let index):
                    crumb(crumbs[index], index: index, style: piece.style)
                case .crumbCaret(let index):
                    caret(piece.kind, segment: index, style: piece.style, segments: segments)
                case .menuIcon:
                    if let worktrees { worktreeLabel(worktrees, pieces: pieces) }
                case .menuTitle:
                    // Drawn inside the label, with its icon.
                    EmptyView()
                case .menuChevron:
                    caret(piece.kind, segment: crumbs.count, style: piece.style, segments: segments)
                case .terminalTitle:
                    if let terminal = worktrees?.terminal {
                        HStack(alignment: .firstTextBaseline, spacing: 3) {
                            Image(systemName: "terminal").font(piece.style.font)
                            Text(terminal.title).font(piece.style.font).lineLimit(1)
                        }
                        .foregroundStyle(piece.style.color)
                        .baselineProbed("breadcrumb-terminal-baseline")
                        .padding(.horizontal, JumpBar.labelInset)
                        .jumpCell()
                        .background(ring(crumbs.count + 1))
                        .accessibilityElement(children: .combine)
                        .identified("breadcrumb-terminal")
                    }
                case .terminalChevron:
                    caret(piece.kind, segment: crumbs.count + 1, style: piece.style, segments: segments)
                }
            }
            }
            Spacer(minLength: 0)
            if let onClose {
                Button(action: onClose) {
                    Image(systemName: "xmark")
                        .font(.system(size: 10, weight: .semibold))
                        .frame(width: SidebarGrid.control, height: SidebarGrid.control)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.borderless)
                .help("Close (Esc)")
                .accessibilityLabel("Close")
                .accessibilityIdentifier("workspace-close")
            }
        }
        .padding(.leading, 12)
        .padding(.trailing, 6)
        .columnHeader()
        .focusable(keys.isActive)
        .focusEffectDisabled()
        .focused($barFocused)
        .onKeyPress(phases: .down) { press in
            guard keys.isActive, let key = JumpBarKeys.key(press) else { return .ignored }
            return route(key, segments: materialize()) ? .handled : .ignored
        }
        .onChange(of: focusRequest) { _, _ in
            keys = JumpBarKeys.focus(segments: segmentCount) ?? .away
            barFocused = keys.isActive
        }
        .onChange(of: keys.isActive) { _, now in
            if !now { built = nil }
            onActive(now)
        }
        .onChange(of: barFocused) { _, now in
            // A click elsewhere takes the keyboard: the bar lets go of it.
            if !now, keys.isActive, !keys.open { keys = .away }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Jump Bar")
    }

    /// A key, routed by `JumpBarKeys`, and what it asks done.
    private func route(_ key: JumpKey, segments: [JumpMenu]) -> Bool {
        var state = keys
        let effect = JumpBarKeys.handle(key, state: &state, menus: segments)
        keys = state
        switch effect {
        case .jump(let target):
            barFocused = false
            dispatch(target)
        case .leave:
            barFocused = false
            onLeave()
        case .none:
            // A menu closed with Esc: the bar keeps the keyboard.
            if state.isActive, !state.open { barFocused = true }
        }
        return true
    }

    /// Whether segment `index`'s menu is up, and closing it from outside (a
    /// click away) lets the keyboard go.
    private func presented(_ index: Int) -> Binding<Bool> {
        Binding(
            get: { keys.open && keys.segment == index },
            set: { up in if !up, keys.open, keys.segment == index { keys = .away } })
    }

    /// Segment `index`'s menu, in its popover.
    private func menu(_ index: Int, segments: [JumpMenu]) -> some View {
        JumpMenuView(
            menu: segments.indices.contains(index) ? segments[index] : JumpMenu([]), state: keys,
            onKey: { route($0, segments: segments) },
            onPick: { target in
                keys = .away
                dispatch(target)
            })
    }

    /// An item chosen: the worktree's own items and a task's worktree by
    /// the routes its menu took before (ov-185); anything else the
    /// window's.
    private func dispatch(_ target: JumpTarget) {
        switch Self.routed(target) {
        case .perform(let item): worktrees?.perform(item)
        case .open(let item): open(item)
        case .jump(let target): onJump(target)
        }
    }

    /// How a chosen item is carried out (`dispatch`).
    enum Routed: Equatable {
        /// The worktree's own item, as its menu performs it.
        case perform(WorktreeMenu.Item)
        /// A task's worktree, by `onOpen`, which keeps the task as the way back.
        case open(WorkspaceWorktrees.MenuItem)
        /// Anything else, by the window.
        case jump(JumpTarget)
    }

    static func routed(_ target: JumpTarget) -> Routed {
        switch target {
        case .worktree(let item): .perform(item)
        case .open(let next, let from): .open(WorkspaceWorktrees.MenuItem(title: "", target: next, current: false, trail: from))
        default: .jump(target)
        }
    }

    /// The keyboard's place in the bar, while its menu is closed.
    private func ring(_ index: Int) -> some View {
        RoundedRectangle.control
            .fill(Fill.selection(active: true))
            .padding(-3)
            .opacity(keys.segment == index && !keys.open ? 1 : 0)
    }

    /// Carry out a piece's click (`click`).
    private func perform(_ click: Click) {
        switch click {
        case .go(let target): dispatch(target)
        case .menu(let index): open(segment: index)
        case .none: break
        }
    }

    /// A crumb's label: a way to the level it names, the level you're at
    /// plain. Ancestors keep their width; the current one, a task's long
    /// title, gives way, cut in its middle.
    @ViewBuilder
    private func crumb(_ crumb: WorkspaceNavigation.Crumb, index: Int, style: JumpBar.Style) -> some View {
        let text = Text(crumb.title).font(style.font).foregroundStyle(style.color).lineLimit(1)
            .baselineProbed("jump-label-\(index)-baseline")
        if crumb.target != nil {
            JumpLabelButton(name: Self.labelName(crumb.title)) {
                perform(Self.click(.crumb(index), crumbs: crumbs, worktrees: worktrees))
            } label: {
                text
            }
            .fixedSize()
            .background(ring(index))
            .identified("jump-label-\(index)")
        } else {
            text
                .truncationMode(.middle)
                .padding(.horizontal, JumpBar.labelInset)
                .jumpCell()
                .layoutPriority(-1)
                .background(ring(index))
                .accessibilityIdentifier("jump-segment-\(index)")
        }
    }

    /// A segment's ⌄: its menu, in a popover hung from it.
    private func caret(_ kind: Piece.Kind, segment index: Int, style: JumpBar.Style, segments: [JumpMenu]) -> some View {
        JumpCaretButton(
            name: Self.caretName(index, crumbs: crumbs.count, title: crumbs.indices.contains(index) ? crumbs[index].title : nil),
            style: style, probe: "jump-caret-\(index)-baseline"
        ) {
            perform(Self.click(kind, crumbs: crumbs, worktrees: worktrees))
        }
        .popover(isPresented: presented(index), arrowEdge: .bottom) { menu(index, segments: segments) }
        .identified("jump-caret-\(index)")
    }

    /// The worktree segment's label, the glyph and the title: a way to the
    /// worktree it names (back to it from one of its terminals), into a
    /// task's one worktree, or, for "Worktrees" beside a task with several,
    /// its menu. Drawn in the bar's type, not a pop-up button's, which set
    /// "Worktrees" a size larger than its neighbors (owner, 2 Oct).
    @ViewBuilder
    private func worktreeLabel(_ worktrees: WorktreeCrumb, pieces: [Piece]) -> some View {
        let icon = pieces.first { $0.kind == .menuIcon }?.style ?? JumpBar.icon(.menu)
        let title = pieces.first { $0.kind == .menuTitle }?.style ?? JumpBar.style(.menu)
        let label = HStack(alignment: .firstTextBaseline, spacing: 3) {
            Image(systemName: WorktreeSection.glyph)
                .font(icon.font)
                .foregroundStyle(icon.color)
            Text(worktrees.title)
                .font(title.font)
                .foregroundStyle(title.color)
                .lineLimit(1)
        }
        .baselineProbed("breadcrumb-worktrees-baseline")
        let click = Self.click(.menuIcon, crumbs: crumbs, worktrees: worktrees)
        if click == .none {
            label
                .jumpCell()
                .fixedSize()
                .background(ring(crumbs.count))
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("breadcrumb-worktrees")
        } else {
            JumpLabelButton(name: worktrees.opens != nil || worktrees.target != nil ? Self.labelName(worktrees.title) : worktrees.help) {
                perform(click)
            } label: {
                label
            }
            .fixedSize()
            .background(ring(crumbs.count))
            .identified("breadcrumb-worktrees")
        }
    }

    private func open(_ item: WorkspaceWorktrees.MenuItem) {
        if let onOpen { onOpen(item) } else { onGo(item.target) }
    }
}

/// The breadcrumb's worktree menu, as the window builds it.
struct WorktreeCrumb {
    /// What the segment says, after the branch glyph: "tax-rounding", the
    /// worktree it stands for or a task's one, or "Worktrees" beside a task
    /// with several.
    var title: String
    /// Whether it's the level you're at, a worktree opened whole, rather
    /// than the one beneath a task.
    var isHere: Bool
    var tasks: [WorkspaceWorktrees.MenuItem]
    var loose: [WorkspaceWorktrees.MenuItem]
    /// The worktree the segment names, and its menu's items.
    var worktree: String?
    var actions: [WorktreeMenu.Item] = []
    var perform: (WorktreeMenu.Item) -> Void = { _ in }
    /// Beside a task with one worktree: a way into it, and no menu.
    var opens: WorkspaceWorktrees.MenuItem? = nil
    /// Its tooltip, which says whose worktrees it goes among.
    var help = Self.workspaceHelp
    /// The worktree it stands for, opened whole: its terminals, lost ones
    /// apart, for its menu's children (ov-192).
    var children: [JumpSection] = []
    /// Where a click on its label goes: back to the worktree it names from
    /// one of its terminals (ov-267). Nil where it's the level you're at,
    /// or names no one worktree.
    var target: JumpTarget? = nil
    /// The terminal open in it, its last segment (ov-267).
    var terminal: TerminalCrumb? = nil

    /// Its menu (ov-192): its siblings, then its children, then the
    /// worktree's own items. Beside a task with one worktree, that one.
    var jumpMenu: JumpMenu {
        if let opens { return JumpMenus.worktree(siblings: (tasks: [], loose: [opens]), children: [], actions: [], named: nil) }
        return JumpMenus.worktree(siblings: (tasks, loose), children: children, actions: actions, named: worktree)
    }

    static let workspaceHelp = "Go to another worktree in this workspace (⌃⌘↑ ⌃⌘↓)"
}

/// The arrangement a `WorkspaceView` drew, published from the value it
/// switched on, so a test can read which columns a width got.
struct WorkspaceArrangementPreference: PreferenceKey {
    static let defaultValue: WorkspaceColumns.Arrangement? = nil
    static func reduce(value: inout WorkspaceColumns.Arrangement?, nextValue: () -> WorkspaceColumns.Arrangement?) {
        value = nextValue() ?? value
    }
}

/// The detail's width as a `WorkspaceView` measured it: what the window reads
/// to say which of a workspace's columns are on screen.
struct WorkspaceWidthPreference: PreferenceKey {
    static let defaultValue: CGFloat? = nil
    static func reduce(value: inout CGFloat?, nextValue: () -> CGFloat?) {
        value = nextValue() ?? value
    }
}
