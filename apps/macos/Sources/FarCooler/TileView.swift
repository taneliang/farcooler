import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// A layout on screen: tmux's panes, at tmux's rectangles.
///
/// This view computes no arrangement. It used to — a preset name and an ordered
/// member list went in, and frames came out — and the arrangement it produced was
/// a guess at what tmux was doing with the same panes. The two could not agree
/// about anything tmux can express and a preset cannot, which is most of what
/// people actually build.
///
/// What it does instead is one multiplication per pane: tmux reports a rectangle
/// in cells inside a window of `columns` × `rows` cells, and this scales that to
/// the view. Nesting, uneven splits, a dragged divider — all of it arrives for
/// free, because none of it is being re-derived.
///
/// The panes are still placed by absolute frame rather than by nested stacks, and
/// that part has not changed for the reason it never could: each pane holds a live
/// `NSView` streaming bytes, so it must keep its identity across a layout change.
/// Rebuilding a nested stack when the arrangement changes would tear down and
/// re-attach every terminal, wiping four screens to move a divider.
///
/// The other half of the deal is that tmux has to be laying out for the size of
/// this view rather than for some default. See `send(viewport:for:)`.
struct TileView: View {
    /// Every layout in the workspace, so the bar can offer them. The one drawn is
    /// whichever is active.
    let groups: [PaneGroup]
    let workspace: Workspace
    /// This worktree's diff, for whichever pane is showing it.
    ///
    /// One store per worktree rather than one per pane: two changes panes in a
    /// layout are two views of the same branch, and giving each its own store
    /// would have them read the same diffs twice and disagree about which file
    /// is open. Held even when no pane is in changes mode, which costs nothing
    /// — it reads on appear, not on creation.
    ///
    /// Passed, NOT observed. `@ObservedObject` here was a real bug with a
    /// stopwatch on it: this view owns every terminal in the window, and the
    /// store publishes once per file read plus once per loading flag — some
    /// hundred and thirty times for a forty-file branch. Each one invalidated
    /// this whole view, so every pane's `NSView` was re-attached over and over
    /// while a diff loaded, and for ten seconds the window did nothing else:
    /// a pane split during it sat empty, and so did the diff that caused it.
    /// Only `ChangesPane` observes it, which is the only view whose contents
    /// actually change when it publishes.
    let changes: ChangesStore
    let binary: String?
    let environment: [String: String]
    let hostArguments: [String]
    /// Which link the panes below were opened on, so a runner that dropped
    /// and came back gets fresh streams instead of frozen ones. See
    /// `DaemonClient.linkGeneration`.
    let linkGeneration: Int
    /// Why this workspace's runner cannot be acted on, or nil if it can —
    /// passed down to each `TilePane`, which hands it to `AgentSurface` for
    /// an agent pane. See `AgentStream.refusal`'s own doc comment.
    let refusal: () -> String?
    let onFocus: (String) -> Void
    let onSelectGroup: (PaneGroup) -> Void
    /// A terminal dropped on an edge of a pane: put it there.
    let onDropOnPane: (_ dragged: String, _ onto: String, _ side: TileDirection) -> Void
    /// How big this view is, in cells. tmux lays out into it.
    let onViewport: (Int, Int) async -> Void
    /// A divider dragged: which pane's border, and by how many cells. Returns
    /// whether it was accepted — a refused request has to be re-offered, not lost.
    let onResizeDivider: (String, TileDirection, Int) -> Bool
    /// The worktree file search behind an agent pane's `@` picker.
    let onSearchFiles: (String) async -> [String]
    /// Switch a pane between its terminal and its chat.
    let onSwitchPaneMode: (Terminal) -> Void

    @ObservedObject private var prefix = PrefixMode.shared
    @ObservedObject private var preferences = Preferences.shared

    /// The view size the last viewport report was sent for.
    ///
    /// Only here to tell the two things that can invalidate a viewport apart: a
    /// window being dragged, which arrives as a size changing on every frame and
    /// is worth waiting out, and a layout switch, which arrives once and is not.
    /// See the `.task(id:)` below.
    @State private var lastViewportSize: CGSize = .zero

    private var group: PaneGroup? {
        groups.first { $0.isActive } ?? groups.first
    }

    var body: some View {
        VStack(spacing: 0) {
            // Only when there is more than one, so a single arrangement is not
            // labeled for the benefit of a choice nobody has.
            if groups.count > 1 {
                GroupBar(groups: groups, onSelect: onSelectGroup)
            }
            panels
        }
        .paneCanvas()
        .prefixHint()
        // Told from here, because this is the only place that knows a layout is
        // actually on screen. It gates the prefix-less ⌃hjkl bindings: while a
        // single pane is showing, ⌃L has to still clear it.
        .onAppear { prefix.tiledPanes = group?.panes.count ?? 0 }
        .onChange(of: group?.panes.count ?? 0) { _, count in prefix.tiledPanes = count }
        .onDisappear { prefix.tiledPanes = 0 }
        .navigationTitle(workspace.windowTitle)
        .navigationSubtitle(workspace.windowSubtitle)
    }

    /// How the panes move.
    ///
    /// The platform's own spring, not a hand-rolled curve. `.smooth` is critically
    /// damped — it settles without overshooting, which is what a terminal needs: a
    /// pane full of text that bounces past its position and comes back reads as a
    /// rendering glitch rather than as motion.
    /// Spring, not a curve on a stopwatch.
    ///
    /// `.smooth(duration:)` runs its full time whatever it is animating, so a
    /// divider nudged a few points took as long as one thrown across the
    /// window and the whole app felt slow. A spring settles in proportion to
    /// the distance it has to cover, which is what "snappy" actually means.
    private var motion: Animation { Motion.snap }

    /// Zoom gets the tiny bit of energy `.smooth` deliberately lacks.
    ///
    /// Changing the arrangement is a change of layout; zooming is a change of
    /// posture, and a trace of overshoot is what makes it feel like the pane came
    /// forward rather than being swapped. `extraBounce` is kept small — this is one
    /// pane arriving, not a notification.
    private var zoomMotion: Animation { Motion.arrive }

    @ViewBuilder
    private var panels: some View {
        if let group {
            GeometryReader { proxy in
                let size = proxy.size
                // One ForEach over every pane, always. Not `if zoomed { one } else
                // { many }`, which was a structural branch: swapping it changed
                // each pane's view identity, so zooming tore down four live
                // NSViews and re-attached them — four terminals replaying their
                // history to move one of them forward.
                //
                // Zoom needs no case in the geometry at all now: tmux reports a
                // zoomed pane at the full window rect, so it lands full-size from
                // the same multiplication as every other pane. All that is left is
                // to lift it above the ones it covers and hide them, which keeps
                // them mounted and streaming so un-zooming is instant.
                ZStack(alignment: .topLeading) {
                    ForEach(group.panes) { rect in
                        if let terminal = workspace.terminals.first(where: { $0.id == rect.id }) {
                            let frame = frame(of: rect, in: group, size: size)
                            pane(terminal, rect: rect, group: group, size: frame.size)
                                .frame(width: frame.width, height: frame.height)
                                .offset(x: frame.minX, y: frame.minY)
                                .opacity(group.zoomed == nil || rect.zoomed ? 1 : 0)
                                .zIndex(rect.zoomed ? 1 : 0)
                                // A pane joining or leaving grows and fades rather
                                // than appearing at full size, which at four panes
                                // is the difference between noticing which one
                                // arrived and not.
                                .transition(.scale(scale: 0.97).combined(with: .opacity))
                        }
                    }

                    // Over the panes rather than between them: a divider belongs
                    // to two cards, and drawing it on either would put the hit
                    // area at the mercy of which one is on top.
                    PaneDividers(group: group, size: size, onResize: onResizeDivider)
                }
                // tmux's own layout string, which changes exactly when the
                // arrangement does and never when it does not. Animating against
                // the pane list instead would miss a divider moving, and animating
                // against the rectangles would fight the animation it triggered.
                .animation(motion, value: group.layout)
                .animation(zoomMotion, value: group.zoomed)
                // Debounced, because a window drag produces one of these per frame
                // and each is a round trip that re-lays-out every pane.
                // `.task(id:)` cancels the previous run for us, which is the whole
                // debounce — a hand-rolled timer here would be one more thing to
                // cancel on disappear and get wrong.
                //
                // Only a changed SIZE is debounced, though. The other half of
                // `Viewport` is the arrangement tmux is currently in, and that
                // changes on a layout switch — one event, with nothing behind it
                // to coalesce. Waiting it out drew the incoming layout for a
                // quarter of a second against the cell grid of the layout it
                // replaced: every pane scaled from the wrong `group.columns`,
                // then snapping once tmux was finally told. That is the
                // wrong-size flash on every switch.
                .task(id: Viewport(size: size, group: group, font: preferences.revision)) {
                    if size != lastViewportSize {
                        try? await Task.sleep(for: .milliseconds(250))
                        guard !Task.isCancelled else { return }
                    }
                    lastViewportSize = size
                    await send(viewport: size, for: group)
                }
            }
        }
    }

    /// A pane's cell rectangle, in points. See `TileGeometry.frame`.
    private func frame(of rect: PaneRect, in group: PaneGroup, size: CGSize) -> CGRect {
        TileGeometry.frame(
            of: rect, in: PaneGrid(columns: group.columns, rows: group.rows), size: size)
    }

    private func pane(
        _ terminal: Terminal, rect: PaneRect, group: PaneGroup, size: CGSize
    ) -> some View {
        let isFocused = rect.focused

        return TilePane(
            terminal: terminal,
            changes: changes,
            binary: binary,
            environment: environment,
            hostArguments: hostArguments,
            linkGeneration: linkGeneration,
            refusal: refusal,
            isFocused: isFocused,
            isZoomed: rect.zoomed,
            index: (group.panes.firstIndex(of: rect) ?? 0) + 1,
            size: size,
            // The pane's grid as tmux reports it, which is the only correct
            // answer to the question. See `TerminalRenderView.paneGrid`.
            grid: PaneGrid(columns: rect.columns, rows: rect.rows),
            onDrop: { dragged, side in onDropOnPane(dragged, terminal.id, side) },
            onSearchFiles: onSearchFiles,
            onSwitchPaneMode: onSwitchPaneMode
        )
        .onTapGesture { if !isFocused { onFocus(terminal.id) } }
    }

    // MARK: - Viewport

    /// Everything that changes what this view's size is worth in cells.
    ///
    /// The debounce key. Deliberately includes the grid tmux is CURRENTLY laid out
    /// for, because that is not only changed by us: moving a pane out of a window
    /// resizes the window it left, and without noticing that the app would have
    /// nothing left to react to — it would compare the size it last asked for
    /// against the size it wants, find them equal, and leave the panes laid out
    /// for a window a third of the height of the view showing it.
    private struct Viewport: Equatable {
        var size: CGSize
        var across: Int
        var down: Int
        var font: Int
        var haveColumns: Int
        var haveRows: Int

        init(size: CGSize, group: PaneGroup, font: Int) {
            self.size = size
            let depth = TileGeometry.depth(of: group.panes)
            self.across = depth.across
            self.down = depth.down
            self.font = font
            self.haveColumns = group.columns
            self.haveRows = group.rows
        }
    }

    /// Tell tmux how big this view is, in cells. See `TileGeometry.viewport`.
    ///
    /// This is the ONLY grid this app computes. Everything below it — how many
    /// columns any one pane has — is tmux's answer, read back off the layout and
    /// handed to that pane's renderer. See `TilePane`'s `grid`.
    private func send(viewport size: CGSize, for group: PaneGroup) async {
        let viewport = Viewport(size: size, group: group, font: preferences.revision)
        guard
            let window = TileGeometry.viewport(
                fitting: size, across: viewport.across, down: viewport.down,
                cell: TerminalMetrics.cell(preferences.terminalFont()))
        else { return }

        // Compared against what tmux HAS rather than against what we last asked
        // for. The two are not the same thing, and only the first one is a fact.
        guard window.columns != group.columns || window.rows != group.rows else { return }
        await onViewport(window.columns, window.rows)
    }
}

/// One pane, framed.
///
/// The frame is how you know which pane your keystrokes are going to, and with
/// four agents on screen that is the single most important thing the view says.
/// It is a border rather than a dimming of the others, because the others are
/// working and you are reading them.
private struct TilePane: View {
    /// The header's exact height, fixed rather than intrinsic.
    ///
    /// Fixed because the viewport arithmetic subtracts it: a header that sized
    /// itself to its font would make the app's estimate of how much terminal fits
    /// wrong by however much it had grown, and being wrong in that direction wraps
    /// every long line twice.
    static let headerHeight: CGFloat = WorkspaceStyle.paneHeaderHeight

    let terminal: Terminal
    /// The worktree's diff, drawn when this pane is in changes mode.
    ///
    /// Passed rather than observed, for the reason `TileView.changes` gives at
    /// length: a terminal pane must not be rebuilt because a diff two panes
    /// over read another file.
    let changes: ChangesStore
    let binary: String?
    let environment: [String: String]
    let hostArguments: [String]
    /// Which link the panes below were opened on, so a runner that dropped
    /// and came back gets fresh streams instead of frozen ones. See
    /// `DaemonClient.linkGeneration`.
    let linkGeneration: Int
    /// Why this pane's runner cannot be acted on, or nil if it can — see
    /// `AgentStream.refusal`'s own doc comment.
    let refusal: () -> String?
    let isFocused: Bool
    let isZoomed: Bool
    let index: Int
    /// The pane's own size, so a drop can be placed against its edges.
    let size: CGSize
    /// The pane's grid, in cells, straight from tmux — NOT measured from `size`.
    ///
    /// `size` is this pane's share of the view in points, and a share of the
    /// view does not carry a whole number of cells. Measuring it lands a cell or
    /// three away from what tmux actually split, and every cell of the
    /// difference is one tmux never repaints. See `TileGeometry`.
    let grid: PaneGrid
    let onDrop: (String, TileDirection) -> Void
    let onSearchFiles: (String) async -> [String]
    /// Switch this pane between its terminal and its chat.
    let onSwitchPaneMode: (Terminal) -> Void

    @ObservedObject private var preferences = Preferences.shared
    @ObservedObject private var drag = PaneDrag.shared

    /// Which protocol this pane's chat is running on, once the session says.
    ///
    /// Empty until then, and the badge stays hidden rather than guessing: a
    /// pane that has not connected has no protocol yet, and showing `acp`
    /// before anything answered would be a claim rather than a report.
    @State private var paneBackend: String = ""

    /// Which half of this pane a dragged terminal would land in, while it hovers.
    ///
    /// Read from the drag rather than held here. See `PaneDrag`.
    private var landing: TileDirection? { drag.landing(on: terminal.id) }

    /// See `TerminalPane.isLive`, which this mirrors for the same reasons.
    ///
    /// It matters more here: a tiled layout holds several of these, so a single
    /// unreadable tick tore down every pane in the window at once rather than
    /// one.
    private var isLive: Bool {
        let kind = StateKind.parse(terminal.state)
        return kind == .running || kind == .starting || kind == .unknown
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            if terminal.isChangesPane {
                // Not gated on `isLive`, unlike the two surfaces below it. The
                // process behind a changes pane exists to hold the rectangle
                // and nothing else, so its liveness says nothing about whether
                // the diff can be read — that comes from the daemon over the
                // same channel the sidebar's counts do. A pane whose host was
                // killed still shows the branch; it just cannot be split.
                ChangesPane(changes: changes)
                    .id("\(terminal.id)#changes")
            } else if isLive, terminal.isAgentPane {
                // Same empty `onResize` as the terminal case just below, and
                // for the identical reason: `TileView.send(viewport:for:)`
                // already tells tmux the WHOLE window's grid once, from the
                // view's own pixel size and the same font metrics this pane
                // would use — it does not consult what any one pane draws.
                // An agent pane reporting its own geometry here would be
                // exactly the bug the terminal case's comment describes,
                // just for a chat instead of a VT grid.
                AgentSurface(
                    terminal: terminal,
                    binary: binary,
                    environment: environment,
                    hostArguments: hostArguments,
                    linkGeneration: linkGeneration,
                    refusal: refusal,
                    isFocused: isFocused,
                    searchFiles: onSearchFiles,
                    onResize: { _, _ in },
                    onBackend: { paneBackend = $0 }
                )
                // Identity includes the PANE MODE, not just the terminal.
                //
                // A mode switch respawns the pane: new process, new epoch, new
                // stream and input channel. With an id that ignores mode,
                // SwiftUI reuses the existing view, which stays bound to the
                // process that is gone — so the terminal comes back black and
                // swallows every keystroke, and only switching layouts (which
                // rebuilds everything) appears to fix it.
                .id("\(terminal.id)#\(terminal.paneMode ?? "terminal")")
            } else if isLive {
                TerminalSurface(
                    terminal: terminal.short,
                    binary: binary,
                    environment: environment,
                    hostArguments: hostArguments,
                    linkGeneration: linkGeneration,
                    // Deliberately empty. A pane's size is a property of the layout
                    // it is in, so a pane reporting its own grid would resize the
                    // whole tmux window to fit itself and squash its neighbours —
                    // which is exactly what happened while each pane was its own
                    // window and the call survived the change. The view tells tmux
                    // its total size once, in `TileView.send(viewport:for:)`, and
                    // every pane's size falls out of that.
                    onResize: { _, _ in },
                    fontRevision: preferences.revision,
                    isFocused: isFocused,
                    // And falls back IN here, which is the other half of that
                    // deal. Without it the pane's emulator sized itself from its
                    // own pixels and held a grid tmux does not have.
                    grid: grid
                )
                // Identity includes the PANE MODE, not just the terminal.
                //
                // A mode switch respawns the pane: new process, new epoch, new
                // stream and input channel. With an id that ignores mode,
                // SwiftUI reuses the existing view, which stays bound to the
                // process that is gone — so the terminal comes back black and
                // swallows every keystroke, and only switching layouts (which
                // rebuilds everything) appears to fix it.
                .id("\(terminal.id)#\(terminal.paneMode ?? "terminal")")
            } else {
                ZStack {
                    Color(nsColor: Palette.background)
                    StatusGlyph(status: terminal.status, size: 10)
                }
            }
        }
        .paneCard(focused: isFocused || landing != nil)
        // Not animated at all.
        //
        // Focus was animated on the theory that a moving highlight is easier to
        // follow. It is not: moving between panes is a keystroke, the answer is
        // already known before the animation starts, and watching it arrive is
        // the app being slow at something instantaneous.
        .overlay { dropIndicator }
        // A delegate rather than `dropDestination`, for one reason: the indicator
        // has to follow the pointer, and only a delegate is told where the pointer
        // is while the drag is still in progress. `isTargeted:` answers "is it over
        // this pane", which is not enough to say which half of it.
        .onDrop(
            of: [.text],
            delegate: PaneDropTarget(pane: terminal.id, size: size, onDrop: onDrop))
    }

    /// Where the dragged pane would land, drawn over the half it would take.
    ///
    /// A half rather than an outline of the whole pane, because that is the
    /// question being answered: not "will this land here" — it obviously will —
    /// but "which side". VS Code, Xcode and every tiling window manager say it
    /// this way, and the shape IS the answer, so nothing has to be read.
    @ViewBuilder
    private var dropIndicator: some View {
        if let landing {
            let half = landing.half
            RoundedRectangle(cornerRadius: Pane.radius)
                .fill(Color.accentColor.opacity(0.28))
                .overlay(
                    RoundedRectangle(cornerRadius: Pane.radius)
                        .strokeBorder(Color.accentColor.opacity(0.8), lineWidth: 1.5)
                )
                .frame(width: size.width * half.width, height: size.height * half.height)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: half.alignment)
                .allowsHitTesting(false)
                // Animated on which side it is, not on whether it exists: the
                // indicator sliding between halves as the pointer crosses the
                // middle IS the feedback, and it has to keep up with the hand.
                .animation(.snappy(duration: 0.12), value: landing)
        }
    }

    /// One line, and only what a pane needs that a single terminal does not.
    ///
    /// With one terminal on screen the window title says which it is. With four,
    /// nothing does — so each pane names itself, carries the number `prefix N`
    /// selects, and shows its own status dot. That is the whole header; anything
    /// more would be four copies of chrome.
    /// The header is the grab handle, not the whole card.
    ///
    /// Dragging anywhere on a pane used to move it, which put the gesture in
    /// direct competition with two others that live in the same pixels: selecting
    /// text in the terminal, and dragging the divider along its edge. A drag
    /// starting a few points inside a pane moved the pane when it was meant to
    /// move the boundary, and the arrangement rearranged itself under the hand.
    ///
    /// A title bar is what you drag to move a window everywhere else, so this is
    /// also the thing people already try.
    private var header: some View {
        headerContent
            // `.onDrag` rather than `.draggable`, because the id has to be
            // readable synchronously when the drop lands — see `PaneDrag`.
            .onDrag {
                MainActor.assumeIsolated { PaneDrag.shared.begin(terminal.id) }
                return NSItemProvider(object: terminal.id as NSString)
            }
    }

    private var headerContent: some View {
        HStack(spacing: 6) {
            Text("\(index)")
                .font(.system(size: 9, weight: .semibold, design: .monospaced))
                .foregroundStyle(
                    isFocused ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(.tertiary))
                .frame(minWidth: 9)

            Text(terminal.label)
                .font(WorkspaceStyle.paneTitle)
                .foregroundStyle(isFocused ? .primary : .secondary)
                .lineLimit(1)
                .layoutPriority(1)

            if terminal.isChangesPane {
                changesControls
            }

            Spacer(minLength: 4)

            // Which protocol this chat is on, beside the name it belongs to.
            //
            // Only on an agent pane, because a terminal has no protocol — a
            // badge on a shell would be answering a question nobody asked.
            if isLive, terminal.isAgentPane, !paneBackend.isEmpty {
                let native = paneBackend != "acp"
                // "ACP" is an acronym — Agent Client Protocol — and lowercasing
                // it made a proper noun look like a status word.
                Text(native ? "Native" : "ACP")
                    .font(.system(size: 9, weight: .medium))
                    .fixedSize()
                    .padding(.horizontal, 5)
                    .padding(.vertical, 1)
                    .foregroundStyle(
                        native ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(.tertiary))
                    .background(
                        native
                            ? AnyShapeStyle(Color.accentColor.opacity(0.15))
                            : AnyShapeStyle(Color.primary.opacity(0.06)),
                        in: Capsule()
                    )
                    .help(
                        native
                            ? "Native: driven through \(paneBackend)’s own protocol, with no adapter and no npx"
                            : "ACP: driven through an Agent Client Protocol adapter"
                    )
            }

            // Offered only where it would work, and only on a live pane.
            //
            // Chat mode was reachable solely by ⌃B a, which meant it was
            // reachable by whoever already knew about it. The pane that CAN be
            // switched is the one place the offer belongs, and the daemon has
            // already worked out which panes those are — a client cannot,
            // because Claude Code renames its own process.
            if isLive, terminal.canSwitchPaneMode {
                Button {
                    onSwitchPaneMode(terminal)
                } label: {
                    Image(systemName: terminal.isAgentPane ? "terminal" : "bubble.left.and.text.bubble.right")
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                        .frame(width: WorkspaceStyle.controlTarget, height: WorkspaceStyle.controlTarget)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(terminal.isAgentPane ? "Show the terminal (⌃B a)" : "Show the chat (⌃B a)")
            }

            if isZoomed {
                Image(systemName: "arrow.down.right.and.arrow.up.left")
                    .font(.system(size: 8))
                    .foregroundStyle(.tertiary)
            }
            StatusGlyph(status: terminal.status, size: 6)
        }
        .padding(.horizontal, 8)
        .frame(height: Self.headerHeight)
        .background { PaneHeaderBackground(focused: isFocused) }
    }

    /// A changes pane owns its comparison controls in the same strip that names
    /// it. This removes the second title bar that used to begin immediately
    /// below “Changes” and gives every pane one clear piece of chrome.
    private var changesControls: some View {
        HStack(spacing: 7) {
            Divider().frame(height: 14).padding(.horizontal, 2)

            Picker(
                "Comparison",
                selection: Binding(
                    get: { changes.scope },
                    set: { changes.scope = $0 })
            ) {
                ForEach(DiffScope.allCases) { scope in
                    Text(scope.label).tag(scope)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .controlSize(.mini)
            .fixedSize()
            .onChange(of: changes.scope) { _, _ in
                Task { await changes.load(fresh: true) }
            }

            changeCount

            if changes.changeSet.isDirty {
                Circle()
                    .fill(.orange)
                    .frame(width: 5, height: 5)
                    .help("This worktree has uncommitted changes")
            }

            Button {
                Task { await changes.load(fresh: true) }
            } label: {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 10))
                    .frame(width: WorkspaceStyle.controlTarget, height: WorkspaceStyle.controlTarget)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("Refresh changes")
        }
    }

    private var changeCount: Text {
        let files = changes.files
        let fileCount = files.count
        var value = AttributedString(fileCount == 1 ? "1 file" : "\(fileCount) files")
        value.foregroundColor = .secondary

        let insertions =
            changes.scope == .branch
            ? changes.changeSet.insertions
            : files.reduce(0) { $0 + $1.insertions }
        let deletions =
            changes.scope == .branch
            ? changes.changeSet.deletions
            : files.reduce(0) { $0 + $1.deletions }

        if insertions > 0 {
            var added = AttributedString("  +\(insertions)")
            added.foregroundColor = .green
            value.append(added)
        }
        if deletions > 0 {
            var removed = AttributedString(" −\(deletions)")
            removed.foregroundColor = .red
            value.append(removed)
        }
        return Text(value).font(.system(size: 10.5, design: .monospaced))
    }
}

/// A pane as a drop target, tracking which of its edges the pointer is nearest.
///
/// The whole reason this is a `DropDelegate` and not a closure: `dropUpdated` is
/// called for every pointer move over the pane and carries the location, which is
/// what turns a drop into "split this pane on that side" rather than "put it
/// somewhere in here".
private struct PaneDropTarget: DropDelegate {
    let pane: String
    let size: CGSize
    let onDrop: (String, TileDirection) -> Void

    /// A pane cannot be dropped on itself: tmux would be asked to move a pane
    /// against itself, and the gesture means nothing.
    private var dragged: String? {
        guard let id = PaneDrag.shared.terminal, id != pane else { return nil }
        return id
    }

    func validateDrop(info: DropInfo) -> Bool { dragged != nil }

    func dropEntered(info: DropInfo) {
        PaneDrag.shared.hover(pane, TileDirection.drop(at: info.location, in: size))
    }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        PaneDrag.shared.hover(pane, TileDirection.drop(at: info.location, in: size))
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) {
        PaneDrag.shared.leave(pane)
    }

    func performDrop(info: DropInfo) -> Bool {
        let side = TileDirection.drop(at: info.location, in: size)
        let moving = dragged
        // Ended BEFORE the move is asked for, so an update arriving late from a
        // drag that is already over finds nothing in progress and is ignored.
        // With the flag held per-pane, one such update repainted a half of a pane
        // that stayed lit until the next drag.
        PaneDrag.shared.end()
        guard let moving else { return false }
        onDrop(moving, side)
        return true
    }
}
