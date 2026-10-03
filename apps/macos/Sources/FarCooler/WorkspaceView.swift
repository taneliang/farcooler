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
/// Every structural change moves on one spring (`WorkspaceMotion.spring`):
/// the orchestrator and what's opened cross-fading as the selection moves
/// between them, one task switched for another cross-fading in place, and
/// the navigator leaving for Focus. Every part stays mounted while it moves,
/// the orchestrator's terminal included: it's mounted once, at the main
/// area's width, and kept, hidden while something else is selected, so
/// coming back to it is instant and nothing in it re-wraps. Nothing waits on
/// the motion: a click mid-flight retargets it, and what takes clicks and
/// the keyboard follows the window's state at once, not the motion's
/// (`WorkspaceStage`).
struct WorkspaceView<
    Item: Hashable, Conversation: View, Navigator: View, Crumbs: View, Opened: View
>: View {
    /// What's open in the main area: a task or a worktree, or nil for the
    /// orchestrator. Its identity is what's switched: a new one cross-fades
    /// in, the same one with another pane named stays.
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
    /// mounts a terminal or reads the runner.
    @ViewBuilder let detail: (Item, Bool) -> Opened
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
    /// The navigator's width while its trailing edge is dragged.
    @State private var dragging: CGFloat?
    /// The navigator's width when the drag began.
    @State private var dragStart: CGFloat?

    init(
        opened: Item?, hasConversation: Bool, hasBoard: Bool = true, cell: CGFloat, focused: Bool,
        navigatorWidth: Binding<Double>,
        @ViewBuilder conversation: @escaping () -> Conversation, @ViewBuilder navigator: @escaping () -> Navigator,
        @ViewBuilder breadcrumb: @escaping (Item) -> Crumbs,
        @ViewBuilder detail: @escaping (Item, Bool) -> Opened, motion: Animation = WorkspaceMotion.spring
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
        // Drawn as it is from the first frame: a window reopening on a task
        // doesn't fade the orchestrator out.
        _stage = State(initialValue: WorkspaceStage(open: opened, focused: focused))
        _settled = State(initialValue: opened)
    }

    private func arrangement(open: Bool, focused: Bool) -> WorkspaceColumns.Arrangement {
        WorkspaceColumns.layout(opened: open, hasConversation: hasConversation, hasBoard: hasBoard, focused: focused)
    }

    var body: some View {
        GeometryReader { proxy in
            let width = proxy.size.width
            let height = proxy.size.height
            // The window's state: what takes clicks, the keyboard and the
            // accessibility tree's notice, and what's on screen.
            let now = arrangement(open: opened != nil, focused: focused)
            // The motion's: where things are drawn.
            let drawn = arrangement(open: stage.open != nil, focused: stage.focused)
            let remembered = dragging ?? CGFloat(navigatorWidth)
            let frames = WorkspaceColumns.frames(width: width, arrangement: drawn, navigator: remembered, cell: cell)
            // The orchestrator's one width: the main area's beside the
            // navigator, in Focus too, where it's hidden, so going into Focus
            // and out never resizes its tmux window (ov-92 review).
            let kept = WorkspaceColumns.frames(
                width: width, arrangement: arrangement(open: false, focused: false), navigator: remembered, cell: cell)
            ZStack(alignment: .topLeading) {
                main(now: now, drawn: drawn, height: height, kept: kept.main)
                    .frame(width: frames.main, height: height)
                    .clipShape(Rectangle())
                    .offset(x: frames.mainX)
                HStack(spacing: 0) {
                    navigator()
                        .frame(width: frames.navigator)
                        .frame(maxHeight: .infinity)
                    Divider()
                }
                .frame(height: height)
                .background(WorkspaceStyle.canvas)
                .offset(x: drawn.navigator ? 0 : -(frames.navigator + WorkspaceColumns.divider + WorkspaceMotion.overhang))
                .allowsHitTesting(now.navigator)
                .accessibilityHidden(!now.navigator)
                .accessibilityIdentifier("workspace-board")
                navigatorEdge(
                    height: height, at: frames.navigator, current: frames.navigator, shown: drawn.navigator,
                    live: now.navigator, width: width)
            }
            .frame(width: width, height: height, alignment: .topLeading)
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

    /// The main area: the orchestrator, mounted and kept, faded out of
    /// sight while something else is selected; and what's opened, faded in
    /// over it. Both the main area's one width, so neither resizes as the
    /// selection moves between them.
    private func main(
        now: WorkspaceColumns.Arrangement, drawn: WorkspaceColumns.Arrangement, height: CGFloat, kept: CGFloat
    ) -> some View {
        let shown = now.conversation == .main
        return ZStack(alignment: .topLeading) {
            if hasConversation {
                conversation()
                    .frame(width: kept)
                    .frame(maxHeight: .infinity)
                    .background(WorkspaceStyle.canvas)
                    .opacity(drawn.conversation == .main ? 1 : 0)
                    .allowsHitTesting(shown)
                    .accessibilityHidden(!shown)
                    // Nothing in it takes the keyboard while it's hidden:
                    // not the terminal, not a chat composer, not a control.
                    .environment(\.outOfSight, !shown)
                    .disabled(!shown)
                    .accessibilityIdentifier("workspace-conversation")
            } else {
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

    /// `next` settles: at once when it opens from the orchestrator or goes
    /// back to it, and when it switches, only once it has stayed put for
    /// `WorkspaceMotion.settle`.
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

    /// What's opened, under its jump bar: the window's, or the one leaving.
    /// Another one switched in fades in over it on the spring, while the
    /// one leaving goes in a blink, so two records are never overprinted
    /// for long.
    private func openedPane(height: CGFloat) -> some View {
        ZStack {
            if let shown = stage.drawn {
                VStack(spacing: 0) {
                    breadcrumb(shown)
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

/// The one spring a workspace's structure moves on (ov-85), and what the
/// view needs to keep its parts in reach while they move.
enum WorkspaceMotion {
    /// One spring for the selection moving between the orchestrator, a
    /// task and a worktree, and the navigator leaving for Focus (ov-84's,
    /// which every one of them shares).
    static let spring = Animation.spring(response: 0.32, dampingFraction: 0.86)
    /// Past the leading edge when put away, so its divider is out of
    /// sight too.
    static let overhang: CGFloat = 24
    /// Each side of the navigator's trailing edge that takes a drag.
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
/// down to the one you're at, every one but that a way back to it, and the
/// close button at the far end (ov-85). Drawn from `pieces`, one style each
/// (`JumpBar`), in the shared header (`columnHeader`).
struct DrillBreadcrumb: View {
    let crumbs: [WorkspaceNavigation.Crumb]
    /// The trailing worktree segment (ov-86): "⎇ tax-rounding ⌄", a menu
    /// of the workspace's worktrees, task ones by their task, then the
    /// loose ones. It stands for a worktree opened whole, in place of its
    /// crumb, and follows a task as the worktree beneath it.
    var worktrees: WorktreeCrumb?
    var onGo: (ContentView.Selection) -> Void
    /// Close: what's opened goes, and the orchestrator is selected again.
    /// Nil where there's nothing to close to, and no button.
    var onClose: (() -> Void)?

    /// One mark the bar draws, and the style it's drawn in.
    struct Piece: Equatable {
        enum Kind: Equatable {
            case separator
            /// A crumb, by its index in `crumbs`.
            case crumb(Int)
            /// The worktree menu's icon, its title and its ⌄.
            case menuIcon, menuTitle, menuChevron
        }

        var kind: Kind
        var style: JumpBar.Style
    }

    /// Everything the bar draws, in order, each in its one style: what
    /// `ColumnHeaderTests` reads, and all the view draws from.
    static func pieces(_ crumbs: [WorkspaceNavigation.Crumb], worktrees: WorktreeCrumb?) -> [Piece] {
        var out: [Piece] = []
        for (index, crumb) in crumbs.enumerated() {
            if index > 0 { out.append(Piece(kind: .separator, style: JumpBar.chevron)) }
            out.append(Piece(kind: .crumb(index), style: JumpBar.style(crumb.target == nil ? .current : .ancestor)))
        }
        if let worktrees {
            if !crumbs.isEmpty { out.append(Piece(kind: .separator, style: JumpBar.chevron)) }
            out.append(Piece(kind: .menuIcon, style: JumpBar.icon(.menu, isHere: worktrees.isHere)))
            out.append(Piece(kind: .menuTitle, style: JumpBar.style(.menu, isHere: worktrees.isHere)))
            out.append(Piece(kind: .menuChevron, style: JumpBar.chevron))
        }
        return out
    }

    var body: some View {
        let pieces = Self.pieces(crumbs, worktrees: worktrees)
        HStack(spacing: JumpBar.spacing) {
            // The segments on one baseline; the bar centered in the header.
            HStack(alignment: .firstTextBaseline, spacing: JumpBar.spacing) {
            ForEach(Array(pieces.enumerated()), id: \.offset) { _, piece in
                switch piece.kind {
                case .separator:
                    Image(systemName: JumpBar.separatorGlyph)
                        .font(piece.style.font)
                        .foregroundStyle(piece.style.color)
                        .accessibilityHidden(true)
                case .crumb(let index):
                    crumb(crumbs[index], style: piece.style)
                case .menuIcon:
                    if let worktrees { worktreeMenu(worktrees, pieces: pieces) }
                case .menuTitle, .menuChevron:
                    // Drawn inside the menu's label, with its icon.
                    EmptyView()
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
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Breadcrumb")
    }

    /// A crumb: a way back while it has a target, else where you are.
    /// Ancestors keep their width; the current one, a task's long title,
    /// gives way, cut in its middle.
    @ViewBuilder
    private func crumb(_ crumb: WorkspaceNavigation.Crumb, style: JumpBar.Style) -> some View {
        if let target = crumb.target {
            Button { onGo(target) } label: {
                Text(crumb.title).font(style.font).foregroundStyle(style.color).lineLimit(1)
            }
            .buttonStyle(.plain)
            .fixedSize()
            .help("Go to \(crumb.title)")
        } else {
            Text(crumb.title)
                .font(style.font)
                .foregroundStyle(style.color)
                .lineLimit(1)
                .truncationMode(.middle)
                .layoutPriority(-1)
        }
    }

    /// The worktree segment: a menu whose label is drawn here, in the bar's
    /// type, not the pop-up button's own, which set "Worktrees" a size
    /// larger than its neighbors (owner, 2 Oct).
    private func worktreeMenu(_ worktrees: WorktreeCrumb, pieces: [Piece]) -> some View {
        let icon = pieces.first { $0.kind == .menuIcon }?.style ?? JumpBar.icon(.menu)
        let title = pieces.first { $0.kind == .menuTitle }?.style ?? JumpBar.style(.menu)
        let chevron = pieces.first { $0.kind == .menuChevron }?.style ?? JumpBar.chevron
        return Menu {
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
            HStack(alignment: .firstTextBaseline, spacing: 3) {
                Image(systemName: WorktreeSection.glyph)
                    .font(icon.font)
                    .foregroundStyle(icon.color)
                Text(worktrees.title)
                    .font(title.font)
                    .foregroundStyle(title.color)
                    .lineLimit(1)
                Image(systemName: JumpBar.menuGlyph)
                    .font(chevron.font)
                    .foregroundStyle(chevron.color)
            }
            .contentShape(Rectangle())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Go to another worktree in this workspace (⌃⌘↑ ⌃⌘↓)")
        .accessibilityIdentifier("breadcrumb-worktrees")
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
