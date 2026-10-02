import AppKit
import SwiftUI

/// A workspace in the detail (spec §4.3), as a chain of control left to
/// right (ov-85): the orchestrator's rail, the board, and what's opened from
/// it, a task or a worktree, beside the board as a list.
///
/// Nothing here is navigation. The window says what's open (`opened`) and
/// this view places it, by `WorkspaceColumns`' values for its own measured
/// width, never the window's, which the detail shares with the sidebar.
///
/// Every structural change moves on one spring (`WorkspaceMotion.spring`):
/// the board narrowing and widening, what's opened sliding in and out, a
/// task switched for another cross-fading in place, the rail leaving for
/// Focus and coming back, and the orchestrator popping open. Every part stays
/// mounted while it moves, positions and widths are driven from state, and
/// nothing waits on the motion: a click mid-flight retargets it, and what
/// takes clicks and the keyboard follows the window's state at once, not the
/// motion's (`WorkspaceStage`).
struct WorkspaceView<
    Item: Hashable, Conversation: View, Rail: View, Board: View, Crumbs: View, Opened: View
>: View {
    /// What's open beside the board: a task or a worktree, or nil for the
    /// board alone. Its identity is what's switched: a new one cross-fades
    /// in, the same one with another pane named stays.
    let opened: Item?
    /// Whether this workspace has a conversation at all: not a repository's
    /// implicit workspace on a runner without `workstreams`, nor a loose
    /// worktree.
    let hasConversation: Bool
    /// Whether there's a board to draw beside what's opened: not for a loose
    /// worktree whose board can't be found.
    var hasBoard = true
    /// The terminal font's cell width: the minimums are in columns.
    let cell: CGFloat
    /// Focus (⌃⌘↩): what's opened alone, without the rail or the board.
    let focused: Bool
    /// The conversation popped open over the rest.
    let peek: Bool
    /// The board list's width beside what's opened, as its divider was last
    /// dropped. Kept per device by the window.
    @Binding var listWidth: Double
    @ViewBuilder let conversation: () -> Conversation
    @ViewBuilder let rail: () -> Rail
    @ViewBuilder let board: () -> Board
    /// The path over what's opened: Workspace › Task › Worktree, and its
    /// close button.
    @ViewBuilder let breadcrumb: (Item) -> Crumbs
    /// What's opened, drawn for `Item`: the one the window has open, or the
    /// one leaving, which takes no keyboard and isn't seen.
    @ViewBuilder let detail: (Item) -> Opened
    /// A click outside the popped-open conversation: it closes.
    var onDismissPeek: () -> Void = {}
    /// How everything moves: `WorkspaceMotion.spring`, slowed only by a test
    /// that reads it mid-flight.
    var motion: Animation = WorkspaceMotion.spring

    /// What the motion is drawing, a step behind the window's state. See
    /// `WorkspaceStage`.
    @State private var stage: WorkspaceStage<Item>
    /// The detail's width, as last measured: what a closing detail's width
    /// is worked out from.
    @State private var measured: CGFloat = 0
    /// The width of what's opened as it leaves: held, so its terminals and
    /// their tmux windows keep their size on the way out.
    @State private var leavingWidth: CGFloat?
    /// The board list's width while its divider is dragged.
    @State private var dragging: CGFloat?
    /// The list's width when the drag began.
    @State private var dragStart: CGFloat?

    init(
        opened: Item?, hasConversation: Bool, hasBoard: Bool = true, cell: CGFloat, focused: Bool, peek: Bool,
        listWidth: Binding<Double>,
        @ViewBuilder conversation: @escaping () -> Conversation, @ViewBuilder rail: @escaping () -> Rail,
        @ViewBuilder board: @escaping () -> Board, @ViewBuilder breadcrumb: @escaping (Item) -> Crumbs,
        @ViewBuilder detail: @escaping (Item) -> Opened, onDismissPeek: @escaping () -> Void = {},
        motion: Animation = WorkspaceMotion.spring
    ) {
        self.opened = opened
        self.hasConversation = hasConversation
        self.hasBoard = hasBoard
        self.cell = cell
        self.focused = focused
        self.peek = peek
        _listWidth = listWidth
        self.conversation = conversation
        self.rail = rail
        self.board = board
        self.breadcrumb = breadcrumb
        self.detail = detail
        self.onDismissPeek = onDismissPeek
        self.motion = motion
        // Drawn as it is from the first frame: a window reopening on a task
        // doesn't slide it in.
        _stage = State(initialValue: WorkspaceStage(open: opened, focused: focused))
    }

    private func arrangement(width: CGFloat, open: Bool, focused: Bool) -> WorkspaceColumns.Arrangement {
        var arrangement = WorkspaceColumns.layout(
            width: width, opened: open, cell: cell, hasConversation: hasConversation, focused: focused, peek: peek)
        if !hasBoard && open { arrangement.board = false }
        return arrangement
    }

    var body: some View {
        GeometryReader { proxy in
            let width = proxy.size.width
            let height = proxy.size.height
            // The window's state: what takes clicks, the keyboard and the
            // accessibility tree's notice, and what's on screen.
            let now = arrangement(width: width, open: opened != nil, focused: focused)
            // The motion's: where things are drawn.
            let drawn = arrangement(width: width, open: stage.open != nil, focused: stage.focused)
            let list = dragging ?? CGFloat(listWidth)
            let frames = WorkspaceColumns.frames(width: width, arrangement: drawn, list: list, cell: cell)
            let edge = hasConversation ? WorkspaceColumns.rail + WorkspaceColumns.divider : 0
            ZStack(alignment: .topLeading) {
                board()
                    .frame(width: max(0, frames.board), height: height)
                    .offset(x: frames.content)
                    .allowsHitTesting(now.board)
                    .accessibilityHidden(!now.board)
                    .accessibilityIdentifier("workspace-board")
                listDivider(
                    height: height, at: frames.content + frames.board, current: frames.board,
                    shown: drawn.opened && drawn.board, live: now.opened && now.board, content: width - frames.content)
                openedPane(height: height)
                    // One piece: a task switched in, or opened from
                    // closed, is placed inside it and moves with it, rather
                    // than appearing where the motion ends and fading in.
                    .geometryGroup()
                    .frame(width: max(0, stage.open == nil ? (leavingWidth ?? frames.opened) : frames.opened), height: height)
                    .offset(x: stage.open == nil ? width + WorkspaceMotion.overhang : frames.openedX)
                    .allowsHitTesting(opened != nil)
                    .accessibilityHidden(opened == nil)
                if hasConversation {
                    HStack(spacing: 0) {
                        rail()
                            .frame(width: WorkspaceColumns.rail)
                            .frame(maxHeight: .infinity)
                        Divider()
                    }
                    .frame(height: height)
                    .background(WorkspaceStyle.canvas)
                    .offset(x: drawn.conversation == .none ? -edge : 0)
                    .allowsHitTesting(now.conversation != .none)
                    .accessibilityHidden(now.conversation == .none)
                    // Over the board and what's opened, never beside them:
                    // popping the conversation open resizes nothing under it,
                    // a task's terminals and their tmux windows included.
                    OrchestratorPeekPanel(
                        open: now.conversation == .peek,
                        width: WorkspaceColumns.peekWidth(in: width, cell: cell),
                        motion: motion, content: conversation, onDismiss: onDismissPeek)
                    .frame(width: max(0, width - edge), height: height)
                    .offset(x: edge)
                }
            }
            .frame(width: width, height: height, alignment: .topLeading)
            .clipShape(Rectangle())
            .contentShape(Rectangle())
            .background(WorkspaceStyle.canvas)
            .preference(key: WorkspaceArrangementPreference.self, value: now)
            .preference(key: WorkspaceWidthPreference.self, value: width)
            .onChange(of: width, initial: true) { _, width in measured = width }
        }
        .onChange(of: opened) { _, next in
            if next == nil, stage.open != nil {
                // Held at the width it has now, for the way out.
                let leaving = arrangement(width: measured, open: true, focused: stage.focused)
                leavingWidth = WorkspaceColumns.frames(
                    width: measured, arrangement: leaving, list: dragging ?? CGFloat(listWidth), cell: cell
                ).opened
            } else if next != nil {
                leavingWidth = nil
            }
            let generation = stage.generation + 1
            withAnimation(motion) {
                stage.show(next)
            } completion: {
                stage.settle(generation)
            }
        }
        .onChange(of: focused) { _, focused in
            withAnimation(motion) { stage.focused = focused }
        }
    }

    /// What's opened, under its breadcrumb: the window's, or the one
    /// leaving. Another one switched in cross-fades over it, on the spring.
    private func openedPane(height: CGFloat) -> some View {
        ZStack {
            if let shown = stage.drawn {
                VStack(spacing: 0) {
                    breadcrumb(shown)
                    Divider()
                    detail(shown)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                .background(WorkspaceStyle.canvas)
                .id(shown)
                .transition(.opacity)
                // Leaving, nothing in it takes the keyboard.
                .environment(\.outOfSight, opened != shown)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(WorkspaceStyle.canvas)
        .accessibilityIdentifier("workspace-opened")
    }

    /// The board list's trailing edge beside what's opened, dragged to set
    /// its width, kept once dropped.
    private func listDivider(
        height: CGFloat, at x: CGFloat, current: CGFloat, shown: Bool, live: Bool, content: CGFloat
    ) -> some View {
        Rectangle()
            .fill(Color(nsColor: .separatorColor))
            .frame(width: WorkspaceColumns.divider, height: height)
            // A grip wider than the line, centered on it.
            .padding(.horizontal, WorkspaceMotion.grip)
            .contentShape(Rectangle())
            .pointerStyle(.columnResize)
            .gesture(
                // Measured in the window: the divider moves with the drag,
                // so its own coordinates would chase it.
                DragGesture(minimumDistance: 1, coordinateSpace: .global)
                    .onChanged { value in
                        if dragStart == nil { dragStart = current }
                        dragging = WorkspaceColumns.listWidth(
                            (dragStart ?? current) + value.translation.width, content: content, cell: cell)
                    }
                    .onEnded { _ in
                        if let dragging { listWidth = Double(dragging) }
                        dragging = nil
                        dragStart = nil
                    })
            .offset(x: x - WorkspaceMotion.grip)
            .opacity(shown ? 1 : 0)
            .allowsHitTesting(live)
            .accessibilityHidden(true)
    }
}

/// The one spring a workspace's structure moves on (ov-85), and what the
/// view needs to keep its parts in reach while they move.
enum WorkspaceMotion {
    /// One spring for the board narrowing and widening, what's opened
    /// sliding in, out and switching, the rail, and the orchestrator popping
    /// open (ov-84's, which every one of them now shares).
    static let spring = Animation.spring(response: 0.32, dampingFraction: 0.86)
    /// Past the trailing edge when closed, so a shadow or a divider is out
    /// of sight too.
    static let overhang: CGFloat = 24
    /// Each side of the board list's divider that takes a drag.
    static let grip: CGFloat = 3
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

/// The orchestrator popped open from the rail, over the board and what's
/// opened beside it.
///
/// Mounted on its first open and kept, closed or not, so each open slides
/// the same terminal view in rather than building a new one; it stays
/// through Focus, and goes with the workspace. It moves on an offset,
/// on one spring, never by inserting and removing it: a press mid-flight
/// retargets the spring from wherever the panel is.
///
/// Whether it's open is `open`, the window's state, and nothing here waits
/// on the motion. The moment it reads closed, neither the panel nor the
/// click-outside catcher takes a click or the accessibility tree's notice,
/// however far it has left to slide; and it's clipped to what's opened, so
/// in flight it never passes over the rail. Both of those are what made the
/// rail seem dead while it moved (ov-84): the old panel slid in and out by
/// its whole width, catcher and all, across the rail, still hit-testable
/// while removed, so a click on the rail mid-close landed on the catcher,
/// whose close found it already closed. The clip covers clicks as well as
/// drawing: a clip alone lets the panel's far side take clicks over the
/// rail and the sidebar.
struct OrchestratorPeekPanel<Content: View>: View {
    let open: Bool
    /// The conversation's width open (`WorkspaceColumns.peekWidth`).
    let width: CGFloat
    var motion: Animation = WorkspaceMotion.spring
    @ViewBuilder let content: () -> Content
    var onDismiss: () -> Void

    /// Opened once: from then on it stays, off to the side when closed.
    @State private var mounted = false

    var body: some View {
        // Out only once it's on screen to move: the first open mounts it
        // closed, and its appearing slides it out.
        let out = open && mounted
        ZStack(alignment: .leading) {
            // A click anywhere else over what's opened puts it away, under a
            // dimming that comes and goes with it.
            Color.black
                .opacity(out ? OrchestratorPeek.dimming : 0)
                .contentShape(Rectangle())
                .onTapGesture(perform: onDismiss)
                .allowsHitTesting(OrchestratorPeek.takesClicks(open: open))
                .accessibilityHidden(true)
            if mounted || open {
                HStack(spacing: 0) {
                    content()
                        .frame(width: width)
                        .frame(maxHeight: .infinity)
                        .background(WorkspaceStyle.canvas)
                    Divider()
                }
                .compositingGroup()
                .shadow(color: .black.opacity(out ? OrchestratorPeek.shadow : 0), radius: 8, x: 2)
                .offset(x: OrchestratorPeek.offset(open: out, width: width))
                .allowsHitTesting(OrchestratorPeek.takesClicks(open: open))
                .accessibilityHidden(!open)
                // Nothing in it takes the keyboard while it's closed: not the
                // terminal, not a chat composer, not a SwiftUI control.
                .environment(\.outOfSight, !open)
                .disabled(!open)
                .accessibilityIdentifier("workspace-conversation-peek")
                .onAppear { mounted = true }
            }
        }
        .animation(motion, value: out)
        // Drawn and clicked only over what's opened: in flight, the panel's
        // far side is over the rail and the sidebar, and a clip alone stops
        // the drawing there but not the clicks.
        .clipShape(Rectangle())
        .contentShape(Rectangle())
        .allowsHitTesting(OrchestratorPeek.takesClicks(open: open))
    }
}

extension EnvironmentValues {
    /// Mounted but out of sight: the orchestrator tucked away beside a task
    /// (ov-84). A terminal or a composer under it takes no keyboard, by
    /// click, Tab or its own claim, and lets go of one it holds.
    @Entry var outOfSight = false
}

/// What an AppKit view that can take the keyboard does about `outOfSight`.
enum KeyboardFence {
    /// Called as `takesKeyboard` turns off: the window's first responder,
    /// if it's `view`, lets go, and what's opened takes it up from there
    /// (`ContentView.closePeek`).
    @MainActor
    static func release(_ view: NSView) {
        guard let window = view.window, window.firstResponder === view else { return }
        window.makeFirstResponder(nil)
    }
}

/// How the popped-open orchestrator moves, and what a press of the rail,
/// ⌥⌘1, Esc or a click outside does to it: always the opposite of what it
/// is now, at once, with nothing waiting on the motion (ov-84).
enum OrchestratorPeek {
    /// One spring for the panel, its shadow and the dimming, retargeted from
    /// where it is by a press mid-flight: the workspace's one spring.
    static let spring = WorkspaceMotion.spring
    /// What's opened, dimmed under it.
    static let dimming = 0.08
    static let shadow = 0.18
    /// Past its own width when closed, so the shadow is out of sight too.
    static let overhang: CGFloat = 24

    /// Where the panel sits: at the rail's edge open, wholly off to the
    /// side closed.
    static func offset(open: Bool, width: CGFloat) -> CGFloat {
        open ? 0 : -(width + WorkspaceColumns.divider + overhang)
    }

    /// Whether the panel and the catcher around it take clicks: exactly
    /// while it's open, whatever the motion is doing.
    static func takesClicks(open: Bool) -> Bool { open }

    /// The state after a press of the rail or ⌥⌘1.
    static func pressed(open: Bool) -> Bool { !open }

    /// The layout the conversation's view draws, and whether it's on
    /// screen. The panel draws the conversation's layout open or closed, so
    /// it's one terminal view; it's on screen, seen, watched and given the
    /// keyboard only when `visible` (`WorkspaceScreen.visible`) has it,
    /// which is while it's popped open.
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

/// The breadcrumb over what's opened: each level down to the one you're at,
/// every one but that a way back to it, and the close button at the far end,
/// beside the board rather than across the window from it (ov-85).
struct DrillBreadcrumb: View {
    let crumbs: [WorkspaceNavigation.Crumb]
    var onGo: (ContentView.Selection) -> Void
    /// Close: what's opened goes, and the board widens back.
    var onClose: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            ForEach(Array(crumbs.enumerated()), id: \.offset) { index, crumb in
                if index > 0 { Text("›").foregroundStyle(.tertiary) }
                if let target = crumb.target {
                    Button(crumb.title) { onGo(target) }
                        .buttonStyle(.plain)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .help("Go to \(crumb.title)")
                } else {
                    Text(crumb.title)
                        .fontWeight(.semibold)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
            Spacer(minLength: 0)
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
        .font(.system(size: 12))
        .padding(.leading, 12)
        .padding(.trailing, 6)
        .frame(height: 30)
        .background(WorkspaceStyle.canvas)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Breadcrumb")
    }
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
