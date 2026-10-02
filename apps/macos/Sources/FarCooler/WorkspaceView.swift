import AppKit
import SwiftUI

/// A workspace in the detail (spec §4.3, ov-89): a main area that shows one
/// thing, the orchestrator or the task or worktree opened in its place, and
/// the board as a fixed sidebar on the right.
///
/// Nothing here is navigation. The window says what's open (`opened`) and
/// this view places it, by `WorkspaceColumns`' values for its own measured
/// width, never the window's, which the detail shares with the sidebar.
///
/// Every structural change moves on one spring (`WorkspaceMotion.spring`):
/// the orchestrator springing down to its rail as something opens and back
/// to fill the main area as it closes, a task switched for another
/// cross-fading in place, the orchestrator popping open from the rail, the
/// rail leaving for Focus, and the board collapsing to its strip. Every part
/// stays mounted while it moves, the orchestrator's terminal included, which
/// is one view moved between the main area and the rail's panel; positions
/// are driven from state, and nothing waits on the motion: a click
/// mid-flight retargets it, and what takes clicks and the keyboard follows
/// the window's state at once, not the motion's (`WorkspaceStage`).
struct WorkspaceView<
    Item: Hashable, Conversation: View, Rail: View, Board: View, Strip: View, Crumbs: View, Opened: View
>: View {
    /// What's open in the main area: a task or a worktree, or nil for the
    /// orchestrator. Its identity is what's switched: a new one cross-fades
    /// in, the same one with another pane named stays.
    let opened: Item?
    /// Whether this workspace has a conversation at all: not a repository's
    /// implicit workspace on a runner without `workstreams`, nor a loose
    /// worktree.
    let hasConversation: Bool
    /// Whether there's a board to draw: not for a loose worktree whose board
    /// can't be found.
    var hasBoard = true
    /// The terminal font's cell width: the minimums are in columns.
    let cell: CGFloat
    /// Focus (⌃⌘↩): what's opened alone, without the rail or the board.
    let focused: Bool
    /// The conversation popped open from its rail over what's opened.
    let peek: Bool
    /// The board popped open from its strip over the main area, where the
    /// detail is too narrow for the sidebar.
    var boardOver = false
    /// The board's width as its leading edge was last dropped. Kept per
    /// device by the window.
    @Binding var boardWidth: Double
    @ViewBuilder let conversation: () -> Conversation
    @ViewBuilder let rail: () -> Rail
    @ViewBuilder let board: () -> Board
    /// The board collapsed: its icon, "Board" sideways and its count.
    @ViewBuilder let strip: () -> Strip
    /// The path over what's opened: Workspace › Task › Worktree, and its
    /// close button.
    @ViewBuilder let breadcrumb: (Item) -> Crumbs
    /// What's opened, drawn for `Item`: the one the window has open, or the
    /// one leaving, which takes no keyboard and isn't seen. The flag says
    /// whether it has settled (`WorkspaceMotion.settle`): until it has, only
    /// what's cheap is drawn, its header and its text, and nothing that
    /// mounts a terminal or reads the runner.
    @ViewBuilder let detail: (Item, Bool) -> Opened
    /// A click outside the popped-open conversation: it closes.
    var onDismissPeek: () -> Void = {}
    /// A click outside the popped-open board: it closes.
    var onDismissBoard: () -> Void = {}
    /// How everything moves: `WorkspaceMotion.spring`, slowed only by a test
    /// that reads it mid-flight.
    var motion: Animation = WorkspaceMotion.spring

    /// What the motion is drawing, a step behind the window's state. See
    /// `WorkspaceStage`.
    @State private var stage: WorkspaceStage<Item>
    /// What's opened that has settled: the window's, once it's stayed put
    /// for `WorkspaceMotion.settle`. Glancing through tasks with a held
    /// arrow passes the rest without mounting or reading them.
    @State private var settled: Item?
    @State private var settling: Task<Void, Never>?
    /// The board's width while its leading edge is dragged.
    @State private var dragging: CGFloat?
    /// The board's width when the drag began.
    @State private var dragStart: CGFloat?

    init(
        opened: Item?, hasConversation: Bool, hasBoard: Bool = true, cell: CGFloat, focused: Bool, peek: Bool,
        boardOver: Bool = false, boardWidth: Binding<Double>,
        @ViewBuilder conversation: @escaping () -> Conversation, @ViewBuilder rail: @escaping () -> Rail,
        @ViewBuilder board: @escaping () -> Board, @ViewBuilder strip: @escaping () -> Strip,
        @ViewBuilder breadcrumb: @escaping (Item) -> Crumbs,
        @ViewBuilder detail: @escaping (Item, Bool) -> Opened, onDismissPeek: @escaping () -> Void = {},
        onDismissBoard: @escaping () -> Void = {}, motion: Animation = WorkspaceMotion.spring
    ) {
        self.opened = opened
        self.hasConversation = hasConversation
        self.hasBoard = hasBoard
        self.cell = cell
        self.focused = focused
        self.peek = peek
        self.boardOver = boardOver
        _boardWidth = boardWidth
        self.conversation = conversation
        self.rail = rail
        self.board = board
        self.strip = strip
        self.breadcrumb = breadcrumb
        self.detail = detail
        self.onDismissPeek = onDismissPeek
        self.onDismissBoard = onDismissBoard
        self.motion = motion
        // Drawn as it is from the first frame: a window reopening on a task
        // doesn't spring the orchestrator away.
        _stage = State(initialValue: WorkspaceStage(open: opened, focused: focused))
        _settled = State(initialValue: opened)
    }

    private func arrangement(width: CGFloat, open: Bool, focused: Bool) -> WorkspaceColumns.Arrangement {
        WorkspaceColumns.layout(
            width: width, opened: open, cell: cell, hasConversation: hasConversation, hasBoard: hasBoard,
            focused: focused, peek: peek, boardOver: boardOver)
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
            let remembered = dragging ?? CGFloat(boardWidth)
            let frames = WorkspaceColumns.frames(width: width, arrangement: drawn, board: remembered, cell: cell)
            // What's opened stands where it stands open, coming and going
            // too: the orchestrator slides over it and away, and nothing in
            // it, a task's terminals included, is resized on the way.
            let openFrames = WorkspaceColumns.frames(
                width: width, arrangement: arrangement(width: width, open: true, focused: stage.focused),
                board: remembered, cell: cell)
            let railed = [.rail, .peek].contains(drawn.conversation)
            ZStack(alignment: .topLeading) {
                if !hasConversation {
                    WorkspaceMain.nothingOpen
                        .frame(width: frames.main, height: height)
                        .opacity(stage.open == nil ? 1 : 0)
                        .accessibilityHidden(opened != nil)
                }
                openedPane(height: height)
                    // One piece: a task switched in is placed inside it and
                    // moves with it.
                    .geometryGroup()
                    .frame(width: max(0, openFrames.opened), height: height)
                    .offset(x: openFrames.openedX)
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
                    .offset(x: railed ? 0 : -(WorkspaceColumns.rail + WorkspaceColumns.divider))
                    .allowsHitTesting([.rail, .peek].contains(now.conversation))
                    .accessibilityHidden(![.rail, .peek].contains(now.conversation))
                    // Over the orchestrator as it springs away to it, so the
                    // rail is in sight from the first frame; under it as it
                    // comes back to fill the main area.
                    .zIndex(railed ? 2 : 0)
                    // The orchestrator: filling the main area, or popped open
                    // from the rail over what's opened, which resizes nothing
                    // under it. One view, never two.
                    ConversationPanel(
                        state: now.conversation, drawn: drawn.conversation, width: frames.conversation,
                        main: frames.main, content: conversation, onDismiss: onDismissPeek)
                    .frame(width: max(0, frames.main - frames.conversationX), height: height)
                    .offset(x: frames.conversationX)
                    .zIndex(1)
                }
                // A click anywhere over the main area puts the popped-open
                // board away, under a dimming that comes and goes with it.
                Color.black
                    .opacity(drawn.board == .over ? OrchestratorPeek.dimming : 0)
                    .contentShape(Rectangle())
                    .onTapGesture(perform: onDismissBoard)
                    .frame(width: frames.main, height: height)
                    .allowsHitTesting(now.board == .over)
                    .accessibilityHidden(true)
                    .zIndex(3)
                board()
                    .frame(width: max(0, frames.board), height: height)
                    .background(WorkspaceStyle.canvas)
                    // Popped open over the main area, its edge is drawn.
                    .overlay(alignment: .leading) {
                        Rectangle()
                            .fill(Color(nsColor: .separatorColor))
                            .frame(width: WorkspaceColumns.divider)
                            .opacity(drawn.board == .over ? 1 : 0)
                            .allowsHitTesting(false)
                    }
                    .compositingGroup()
                    .shadow(color: .black.opacity(drawn.board == .over ? OrchestratorPeek.shadow : 0), radius: 8, x: -2)
                    .offset(x: frames.boardX)
                    .allowsHitTesting(now.boardInSight)
                    .accessibilityHidden(!now.boardInSight)
                    .accessibilityIdentifier("workspace-board")
                    .zIndex(3)
                boardEdge(
                    height: height, at: frames.boardX, current: frames.board,
                    shown: drawn.board == .side, live: now.board == .side, width: width)
                .zIndex(3)
                HStack(spacing: 0) {
                    Divider()
                    strip()
                        .frame(width: WorkspaceColumns.rail)
                        .frame(maxHeight: .infinity)
                }
                .frame(height: height)
                .background(WorkspaceStyle.canvas)
                .offset(
                    x: [.strip, .over].contains(drawn.board)
                        ? width - WorkspaceColumns.rail - WorkspaceColumns.divider : width + WorkspaceMotion.overhang)
                .allowsHitTesting([.strip, .over].contains(now.board))
                .accessibilityHidden(![.strip, .over].contains(now.board))
                .zIndex(3)
            }
            .frame(width: width, height: height, alignment: .topLeading)
            // Peeking, popping the board and collapsing it move on the same
            // spring as opening and closing, which `stage` already animates.
            .animation(motion, value: drawn)
            .clipShape(Rectangle())
            .contentShape(Rectangle())
            .background(WorkspaceStyle.canvas)
            .preference(key: WorkspaceArrangementPreference.self, value: now)
            .preference(key: WorkspaceWidthPreference.self, value: width)
        }
        .onChange(of: opened) { _, next in
            settle(next, switching: stage.open != nil && next != nil)
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

    /// `next` settles: at once when it opens from closed or closes, and when
    /// it switches, only once it has stayed put for `WorkspaceMotion.settle`.
    private func settle(_ next: Item?, switching: Bool) {
        settling?.cancel()
        guard switching else {
            settled = next
            return
        }
        settling = Task { @MainActor in
            try? await Task.sleep(for: WorkspaceMotion.settle)
            guard !Task.isCancelled else { return }
            settled = next
        }
    }

    /// What's opened, under its breadcrumb: the window's, or the one
    /// leaving. Another one switched in fades in over it on the spring,
    /// while the one leaving goes in a blink, so two records are never
    /// overprinted for long.
    private func openedPane(height: CGFloat) -> some View {
        ZStack {
            if let shown = stage.drawn {
                VStack(spacing: 0) {
                    breadcrumb(shown)
                    Divider()
                    detail(shown, settled == shown)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                .background(WorkspaceStyle.canvas)
                .id(shown)
                .transition(.asymmetric(insertion: .opacity, removal: .opacity.animation(WorkspaceMotion.leave)))
                // Leaving, nothing in it takes the keyboard or a click.
                .environment(\.outOfSight, opened != shown)
                .allowsHitTesting(opened == shown)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(WorkspaceStyle.canvas)
        .accessibilityIdentifier("workspace-opened")
    }

    /// The board's leading edge, dragged to set its width, kept once
    /// dropped. Dragged left, the board widens.
    private func boardEdge(
        height: CGFloat, at x: CGFloat, current: CGFloat, shown: Bool, live: Bool, width: CGFloat
    ) -> some View {
        Rectangle()
            .fill(Color(nsColor: .separatorColor))
            .frame(width: WorkspaceColumns.divider, height: height)
            // A grip wider than the line, centered on it.
            .padding(.horizontal, WorkspaceMotion.grip)
            .contentShape(Rectangle())
            .pointerStyle(.columnResize)
            .gesture(
                // Measured in the window: the edge moves with the drag, so
                // its own coordinates would chase it.
                DragGesture(minimumDistance: 1, coordinateSpace: .global)
                    .onChanged { value in
                        if dragStart == nil { dragStart = current }
                        dragging = WorkspaceColumns.boardWidth(
                            (dragStart ?? current) - value.translation.width, width: width, cell: cell)
                    }
                    .onEnded { _ in
                        if let dragging { boardWidth = Double(dragging) }
                        dragging = nil
                        dragStart = nil
                    })
            .offset(x: x - WorkspaceColumns.divider - WorkspaceMotion.grip)
            .opacity(shown ? 1 : 0)
            .allowsHitTesting(live)
            .accessibilityHidden(true)
    }
}

/// What the main area shows with nothing open where no orchestrator can
/// run: a repository's board on a runner without `workstreams`, which has
/// no workspaces to seat one in. It's the orchestrator's place, so it's the
/// orchestrator's empty state, saying why there's nothing to start. A
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
    /// Each side of the board's leading edge that takes a drag.
    static let grip: CGFloat = 3
    /// How long the one leaving takes to fade when another is switched in.
    static let leave = Animation.easeOut(duration: 0.08)
    /// How long a task switched to stays put before it's settled, and its
    /// terminal is mounted and its record read.
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

/// The orchestrator, in the main area or popped open from its rail over
/// what's opened, as one view (ov-89).
///
/// Mounted with the workspace and kept: springing down to the rail slides
/// it off to the side, and popping it open or closing what's opened slides
/// the same terminal view back, never a new one. It's the same width in
/// every state, the main area less the rail, so none of those moves resizes
/// its terminal or the tmux window behind it, and popped open → filling the
/// main area is one slide on the spring. Filling the main area, it's drawn
/// the rail's width wider, up to the board, but its terminal reports the
/// same grid to tmux (`viewportSlack`): the difference is the terminal's own
/// background at its trailing edge. Its width
/// changes only with the window's, or the board collapsing, and then snaps,
/// so it's never resized on every frame of a spring.
///
/// Whether it's in sight is `state`, the window's, and nothing here waits
/// on the motion. The moment it reads railed, neither the panel nor the
/// click-outside catcher takes a click or the accessibility tree's notice,
/// however far it has left to slide; and it's clipped to the main area, past
/// the rail once that's drawn. The rail is drawn over it while it springs
/// away (`WorkspaceView`), so the rail is in sight and clickable from the
/// first frame. Both of those are what made the rail seem dead while it
/// moved (ov-84). The clip covers clicks as well as drawing.
struct ConversationPanel<Content: View>: View {
    typealias Place = WorkspaceColumns.Arrangement.Conversation

    /// Where the window has it.
    let state: Place
    /// Where the motion draws it.
    let drawn: Place
    /// Its width (`WorkspaceColumns.Frames.conversation`), nil in Focus.
    let width: CGFloat?
    /// The main area's width: its width before it has ever been in sight.
    let main: CGFloat
    @ViewBuilder let content: () -> Content
    var onDismiss: () -> Void

    /// The width it last had, kept through Focus.
    @State private var held: CGFloat?

    var body: some View {
        let shownWidth = width ?? held ?? main
        // Filling the main area, it runs to the board: the rail's width
        // more than its terminal's, drawn by the terminal in its own
        // background and never told to tmux (`viewportSlack`).
        let extra = drawn == .main ? WorkspaceColumns.rail + WorkspaceColumns.divider : 0
        let open = state == .main || state == .peek
        let out = drawn == .main || drawn == .peek
        ZStack(alignment: .leading) {
            // A click anywhere else over what's opened puts it away, under a
            // dimming that comes and goes with it.
            Color.black
                .opacity(drawn == .peek ? OrchestratorPeek.dimming : 0)
                .contentShape(Rectangle())
                .onTapGesture(perform: onDismiss)
                .allowsHitTesting(state == .peek)
                .accessibilityHidden(true)
            HStack(spacing: 0) {
                content()
                    .environment(\.viewportSlack, extra)
                    .frame(width: shownWidth + extra)
                    .frame(maxHeight: .infinity)
                    .background(WorkspaceStyle.canvas)
                    // Snapped: a terminal resized on every frame of the
                    // spring would resize its tmux window on every frame.
                    .animation(nil, value: shownWidth + extra)
                Divider().opacity(drawn == .main ? 0 : 1)
            }
            .compositingGroup()
            .shadow(color: .black.opacity(drawn == .peek ? OrchestratorPeek.shadow : 0), radius: 8, x: 2)
            .offset(x: out ? 0 : OrchestratorPeek.hidden(width: shownWidth))
            .allowsHitTesting(open)
            .accessibilityHidden(!open)
            // Nothing in it takes the keyboard while it's on the rail: not
            // the terminal, not a chat composer, not a SwiftUI control.
            .environment(\.outOfSight, !open)
            .disabled(!open)
            .accessibilityIdentifier("workspace-conversation")
        }
        // Drawn and clicked only over the main area past the rail: in
        // flight, its far side is over the rail and the sidebar, and a clip
        // alone stops the drawing there but not the clicks.
        .clipShape(Rectangle())
        .contentShape(Rectangle())
        .allowsHitTesting(open)
        .onChange(of: width, initial: true) { _, width in
            if let width { held = width }
        }
    }
}

extension EnvironmentValues {
    /// Mounted but out of sight: the orchestrator tucked away beside a task
    /// (ov-84). A terminal or a composer under it takes no keyboard, by
    /// click, Tab or its own claim, and lets go of one it holds.
    @Entry var outOfSight = false
    /// Points of a terminal view's width it doesn't report to tmux: the
    /// orchestrator filling the main area is drawn the rail's width wider
    /// than its grid, so its tmux window keeps one width in every state
    /// (ov-89). Zero everywhere else.
    @Entry var viewportSlack: CGFloat = 0
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
    /// What's opened, dimmed under it.
    static let dimming = 0.08
    static let shadow = 0.18
    /// Where the panel sits on its rail: wholly off to the side, past its
    /// own width and its shadow (`WorkspaceMotion.overhang`).
    static func hidden(width: CGFloat) -> CGFloat {
        -(width + WorkspaceColumns.divider + WorkspaceMotion.overhang)
    }

    /// The state after a press of the rail or ⌥⌘1.
    static func pressed(open: Bool) -> Bool { !open }

    /// The layout the conversation's view draws, and whether it's on
    /// screen. The panel draws the conversation's layout open or closed, so
    /// it's one terminal view; it's on screen, seen, watched and given the
    /// keyboard only when `visible` (`WorkspaceScreen.visible`) has it,
    /// which is while it fills the main area or is popped open.
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
    /// The trailing worktree segment (ov-86): "⎇ tax-rounding ▾", a menu
    /// of the workspace's worktrees, task ones by their task, then the
    /// loose ones. It stands for a worktree opened whole, in place of its
    /// crumb, and follows a task as the worktree beneath it.
    var worktrees: WorktreeCrumb?
    var onGo: (ContentView.Selection) -> Void
    /// Close: what's opened goes, and the orchestrator fills the main area
    /// again. Nil where
    /// there's nothing to close to, and no button.
    var onClose: (() -> Void)?

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
            if let worktrees {
                if !crumbs.isEmpty { Text("›").foregroundStyle(.tertiary) }
                Menu {
                    if !worktrees.tasks.isEmpty {
                        Section("Tasks") {
                            ForEach(worktrees.tasks) { item in menuItem(item) }
                        }
                    }
                    if !worktrees.loose.isEmpty {
                        Section("Worktrees") {
                            ForEach(worktrees.loose) { item in menuItem(item) }
                        }
                    }
                    if worktrees.tasks.isEmpty && worktrees.loose.isEmpty {
                        Text("No worktrees yet")
                    }
                    // The worktree the segment stands for, and what its
                    // sidebar row's menus did (review M1).
                    if let name = worktrees.worktree, !worktrees.actions.isEmpty {
                        Section(name) {
                            WorktreeMenuItems(items: worktrees.actions, perform: worktrees.perform)
                        }
                    }
                } label: {
                    HStack(spacing: 3) {
                        Image(systemName: WorktreeSection.glyph)
                            .font(.system(size: 10, weight: .medium))
                        Text(worktrees.title)
                            .fontWeight(worktrees.isHere ? .semibold : .regular)
                            .lineLimit(1)
                    }
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .help("Go to another worktree in this workspace (⌃⌘↑ ⌃⌘↓)")
                .accessibilityIdentifier("breadcrumb-worktrees")
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
        .font(.system(size: 12))
        .padding(.leading, 12)
        .padding(.trailing, 6)
        .frame(height: 30)
        .background(WorkspaceStyle.canvas)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Breadcrumb")
    }
}

extension DrillBreadcrumb {
    @ViewBuilder
    private func menuItem(_ item: WorkspaceWorktrees.MenuItem) -> some View {
        // A toggle, for the menu's own checkmark on where you are: a
        // button's image beside a subtitle was dropped (live, ov-86).
        Toggle(isOn: Binding(get: { item.current }, set: { _ in onGo(item.target) })) {
            if let subtitle = item.subtitle {
                Text(item.title)
                Text(subtitle)
            } else {
                Label(item.title, systemImage: WorktreeSection.glyph)
            }
        }
    }
}

/// The breadcrumb's worktree menu, as the window builds it.
struct WorktreeCrumb {
    /// What the segment says, after the branch glyph: "tax-rounding", the
    /// worktree it stands for, or "Worktrees" beside a task with none.
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
