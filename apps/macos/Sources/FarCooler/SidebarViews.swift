import AgentKit
import SwiftUI
import UniformTypeIdentifiers

/// Measurements the sidebar takes from its own data.
enum SidebarMetrics {
    /// How wide the diff column has to be for every row in the fleet.
    ///
    /// Measured from the widest pair actually present, not guessed at, and
    /// computed once per fleet update rather than per row. The font is fixed and
    /// the strings are short, so this is a handful of `NSString.size` calls
    /// against a list that is already in hand.
    ///
    /// Guessing was tried twice and has no good value: 78pt lines the numbers
    /// up and takes the room out of the branch name, while anything narrow
    /// enough to leave the branch alone is too narrow for a lockfile's counts
    /// and lets the column go ragged again — which is the bug it was added for.
    static func countsWidth(_ rows: [InboxRow]) -> CGFloat {
        let font = NSFont.monospacedSystemFont(ofSize: 10, weight: .regular)
        var widest: CGFloat = 0
        for row in rows where row.hasDiff {
            // Built the same way the row builds it, character for character,
            // so the measurement is of the string that gets drawn — see
            // `DiffCounts`, which both of them go through.
            let text = DiffCounts.pair(
                insertions: row.insertions, deletions: row.deletions)
            let w = (text as NSString).size(withAttributes: [.font: font]).width
            widest = max(widest, w)
        }
        // Nothing in the fleet has a diff: no column, no cost.
        return widest == 0 ? 0 : ceil(widest) + 2
    }
}

extension View {
    /// `.help`, when there is something to say, and nothing at all otherwise.
    ///
    /// An empty tooltip string is not the same as no tooltip: it still hands
    /// AppKit a tracking rectangle for a box with nothing in it. A conditional
    /// modifier keeps a clean worktree's row from having one.
    @ViewBuilder
    func help(ifAny text: String?) -> some View {
        if let text {
            self.help(text)
        } else {
            self
        }
    }
}

/// One worktree, and its terminals when opened.
///
/// Collapsed to a single line by default. The previous version made a worktree
/// a two-line heading with its terminals always beneath it, which was built on
/// a wrong assumption about scale: there are a handful of projects, a handful
/// of terminals per worktree, and potentially hundreds of worktrees — some
/// dormant for weeks and resurrected later. The thing there are hundreds of has
/// to be the lightest row in the app, not the heaviest.
///
/// It expands on its own when something inside wants the user, because a
/// collapsed row that hides the agent asking a question defeats the point.
struct WorktreeSection: View {
    /// What a worktree is drawn as in its glyph column: a branch.
    static let glyph = "arrow.triangle.branch"

    /// The one drag in flight anywhere in the sidebar. An `@ObservedObject` on
    /// the shared instance rather than a parameter: `worktreeRow` already
    /// passes eighteen arguments and its own comment records that a nineteenth
    /// pushed the enclosing expression past the type checker's budget.
    @ObservedObject private var drag = WorktreeDrag.shared
    /// This header's measured height, for deciding above-or-below.
    @State private var headerHeight: CGFloat = 0
    let worktree: Worktree
    let isExpanded: Bool
    /// What of this worktree the window is showing: the worktree itself
    /// (`.some(nil)`), one of its terminals, or nothing (`nil`).
    let selected: String??
    /// Open the worktree, or one of its terminals, in the window.
    let onSelect: (_ terminal: String?) -> Void
    let onToggle: () -> Void
    let onNewTerminal: () -> Void
    let onHide: () -> Void
    let onUnhide: () -> Void
    let onRemove: () -> Void
    let onTerminalAction: (Terminal, TerminalAction) -> Void
    /// Where a terminal can be sent: each existing layout, plus one of its own.
    ///
    /// The answer to "how do I put these two together, and only these two", for
    /// people who would rather not drag. Dragging is the better gesture — it says
    /// which pane and which edge, which a menu cannot ask — so this is the coarse
    /// version of it, and deliberately so.
    var layouts: [PaneGroup] = []
    /// `nil` means a layout of its own.
    var onMoveToLayout: (Terminal, PaneGroup?) -> Void = { _, _ in }
    /// Dropping one terminal on another puts them side by side.
    var onDropTogether: (_ dragged: String, _ onto: Terminal) -> Void = { _, _ in }
    /// The terminals currently on screen together, if any.
    ///
    /// Passed in rather than looked up, because the sidebar's job is to show
    /// which of these rows you are looking at — the terminals that are tiled and
    /// the ones running behind them are the same list, and the difference is one
    /// mark, not a second section.
    var tiled: Set<String> = []
    /// Where an editor's refusal to start goes.
    ///
    /// A closure rather than a write from here: `DaemonClient` is `ContentView`'s
    /// and is not in the environment, and the banner that shows this belongs to
    /// the window, not to one sidebar row.
    var onEditorError: (String) -> Void = { _ in }
    /// Whether this worktree's runner can be acted on right now.
    ///
    /// Reads stay live even when this is `false` — the row is still
    /// selectable and its terminals still show whatever was last read from
    /// it — but the affordances that would MUTATE something on a runner
    /// already known to be unreachable are dimmed and inert rather than
    /// left to fail silently or hang on a dead connection.
    var usable: Bool = true
    /// Whether this row can be dragged into a new place in the sidebar.
    ///
    /// Decided by `WorktreeDrag.offersDrag(usable:runner:)` where the runner's
    /// build is known, and false by default: a row nobody said could move
    /// offers no drag.
    var reorderable: Bool = false

    /// The workspaces Move to Workspace ▸ offers: exactly those a drag of
    /// this row onto a workspace row would move it to. See
    /// `ContentView.moveTargets`.
    var moveTargets: [WorkspaceSummary] = []
    var onMove: (WorkspaceSummary) -> Void = { _ in }
    /// What each terminal's menu offers about its workspace's orchestrator:
    /// see `OrchestratorAdoption.offer`.
    var roleOffer: (Terminal) -> OrchestratorAdoption.Offer? = { _ in nil }
    /// Show Changes, on the row's menus: the worktree opened with its
    /// changes pane, which opens on its own when it has no terminal
    /// (ov-78). Nil when its runner can't read changes.
    var onShowChanges: (() -> Void)?

    /// Diff status for this worktree, when the fleet inbox has been read.
    ///
    /// Absent is a real state and shows nothing at all, rather than a confident
    /// `+0 -0` for a worktree nobody has looked at yet.
    var changes: InboxRow?

    /// How much room the diff column takes on every row in this list.
    ///
    /// One number for the whole sidebar — see `SidebarMetrics.countsWidth` —
    /// because a column each row sizes for itself is not a column. Zero when
    /// nothing in the fleet has a diff, so the space costs nothing until there
    /// is something to put in it.
    var countsWidth: CGFloat = 0

    @State private var hovering = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.boardMotionSlowdown) private var slowdown
    /// See `WorkspaceStyle.navigatorSelection(active:)`.
    ///
    /// `.key` and not `!= .inactive`: with two Far Cooler windows open the
    /// second one is `.active` — the app has focus, this window does not — and
    /// that is precisely the case the dimming exists for.
    @Environment(\.controlActiveState) private var controlActiveState
    private var windowActive: Bool { controlActiveState == .key }

    private var isSelected: Bool {
        if case .some(nil) = selected { return true }
        return false
    }

    /// Open whether or not the user opened it.
    private var showsTerminals: Bool { isExpanded || !worktree.attention.isEmpty }

    private func row(_ terminal: Terminal, ordinal: Int?) -> some View {
        TerminalRow(
            terminal: terminal,
            isSelected: selected == .some(terminal.id),
            onSelect: { onSelect(terminal.id) },
            isTiled: tiled.contains(terminal.id),
            layouts: layouts,
            onMoveToLayout: { onMoveToLayout(terminal, $0) },
            onDropTogether: { onDropTogether($0, terminal) },
            onAction: { onTerminalAction(terminal, $0) },
            ordinal: ordinal,
            usable: usable,
            roleOffer: roleOffer(terminal)
        )
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header

            if showsTerminals {
                let numbering = worktree.ordinals()
                // Creation order, always. Sorting whatever needs you to the top
                // put the sidebar in motion at the exact moment you were
                // reaching for it: an agent three rows down finishes, every row
                // under it slides, and the click you had already committed to
                // lands on something else. Attention is a mark on a row — the
                // glyph, the count in the status bar, ⌘⇧A — and a mark you can
                // find in a stable list beats one that comes to you by moving
                // the list.
                ForEach(worktree.terminals) { t in
                    row(t, ordinal: numbering[t.id])
                        .transition(BoardMotion.rowTransition(reduceMotion: reduceMotion, slowedBy: slowdown))
                }
            }
        }
    }

    /// A two-level heading: the worktree is what you choose, and the branch is
    /// orientation. Giving each its own baseline keeps long branch names from
    /// competing with the worktree name and makes terminals below read as real
    /// children rather than another run of equal rows.
    private var header: some View {
        HStack(alignment: .center, spacing: 0) {
            // Chevron, branch glyph, then the title over its branch: one
            // column each (ov-83), so under a workspace the chevron is at B,
            // the glyph at C and both lines at D.
            // The shared chevron (ov-101): its terminals are this view's
            // own rows, but the row around it is a control of its own.
            DisclosureButton(
                expanded: showsTerminals, accessibilityLabel: "Terminals in \(worktree.task)",
                gridRow: "worktree", width: SidebarGrid.chevronColumn, action: onToggle)

            Image(systemName: Self.glyph)
                .font(.system(size: 10, weight: .medium))
                .foregroundStyle(.tertiary)
                .frame(width: SidebarGrid.glyphColumn, height: 16, alignment: .leading)
                .gridMark("worktree", .icon)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 1) {
                // With its open tasks' keys, "fc-3-webhooks · bil-9", so a
                // row says which task it's for without a board read.
                Text(worktree.rowTitle)
                    .font(WorkspaceStyle.sidebarPrimary)
                    .lineLimit(1)
                    .gridMark("worktree", .text)

                HStack(spacing: 5) {
                    Text(worktree.isMainCheckout ? "Main checkout" : worktree.branch)
                        .font(WorkspaceStyle.sidebarMetadata)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .gridMark("worktree.branch", .text)

                    if worktree.worktreeMissing {
                        Text("worktree gone")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(.orange)
                            .help(
                                "This worktree’s directory is gone. If you moved it with git worktree "
                                    + "move, it’s listed again under its new path as its own row. This row "
                                    + "keeps the terminals and agent transcripts from before the move.")
                    }
                }
            }
            .layoutPriority(1)

            Spacer(minLength: 6)

            if !showsTerminals, let waiting = worktree.attentionStatus {
                // Attention is the only collapsed status worth permanent room.
                // Terminal counts and "no terminals" repeat what expanding the
                // row already says, while this dot is the reason to expand it.
                //
                // Its color comes from the same rule the expanded row's glyph
                // uses. Hardcoded orange here meant a worktree whose one
                // pending item was a failed agent showed orange collapsed and
                // red expanded — the same terminal, two colors, decided by
                // whether a disclosure triangle happened to be open.
                // Drawn by `StatusGlyph` rather than by a `Circle` of its
                // own, so the collapsed row and the expanded row are the same
                // mark and not two drawings that agree by hand. 6pt is below
                // anything §03 names, which is what the in-app initialiser is
                // for — see `StatusGlyph.Geometry`.
                StatusGlyph(status: waiting, inAppDiameter: 6)
                    .accessibilityLabel(
                        "\(worktree.attention.count) waiting on you in \(worktree.task)")
            }

            // What changed in here, at a glance, without opening anything.
            //
            // Changed rows use one FIXED width. A minimum pins the right edge
            // and lets the LEFT edge float, so the cell is only as wide as its
            // own digits and the numbers start somewhere different on every
            // changed row. A fixed cell gives those rows one left edge.
            //
            // The width comes from the fleet's own widest count, measured once
            // per update rather than guessed: a guess is either too small, and
            // the column it was supposed to fix goes ragged again, or too big,
            // and it takes the room out of the branch name. An unchanged row
            // does not render an invisible placeholder: alignment between
            // values that do not exist is not worth truncating every branch.
            //
            // Inside the cell the pair stays tight and hugs the trailing edge.
            // A width per NUMBER was tried and is worse than the problem: `+1`
            // in a box sized for `+35,870` leaves a hole you could park a word
            // in, and the pair stops reading as one thing.
            // Counts and hover actions are two states of one trailing cell.
            // Keeping invisible controls after the counts still made SwiftUI
            // reserve their width, which stranded the diff in the middle of
            // the row. Sharing the cell pins both states to the native trailing
            // edge and prevents the actions from shifting the row on hover.
            ZStack(alignment: .trailing) {
                if let changes, changes.hasDiff {
                    // One attributed Text means one baseline. Two sibling Text
                    // views can still land on adjacent device pixels after the
                    // stack reconciles their independently rounded font metrics.
                    changeCountsText(changes)
                        .font(.system(size: 10, design: .monospaced))
                        .lineLimit(1)
                        .fixedSize()
                        .frame(width: countsWidth, alignment: .trailing)
                        // Hidden on HOVER and on nothing else. `isSelected`
                        // used to be in here too, which meant the one row you
                        // had chosen was the one row that could never show its
                        // own diff — permanently, since selection has no
                        // pointer to move away. Hover-to-reveal is a pointer
                        // idiom; selection is not a pointer state.
                        .opacity(hovering ? 0 : 1)
                        .accessibilityElement(children: .ignore)
                        // Named the arithmetic and not the subject until now:
                        // "6 insertions, 3 deletions" is spoken by a row that
                        // never says what was counted, and this number counts
                        // more than the branch has committed. The worktree is
                        // in it for the same reason the attention dot's label
                        // carries it — this is its own element, reached on its
                        // own, with no row around it to say which one it is.
                        //
                        // The read state is spoken because it is drawn in
                        // color and in nothing else. A row that has been
                        // marked reviewed dims its pair and says nothing
                        // otherwise, so a label that left it out would describe
                        // two different rows identically.
                        .accessibilityLabel(
                            "\(changes.insertions) insertions, \(changes.deletions) deletions "
                                + "in \(worktree.task), including work that isn’t committed yet"
                                + (changes.changedSinceReviewed
                                    ? ", changed since you last looked" : ", reviewed"))
                }

                HStack(spacing: 2) {
                    Button(action: onNewTerminal) {
                        Image(systemName: "plus")
                            .font(.system(size: 10, weight: .semibold))
                            .frame(
                                width: WorkspaceStyle.controlTarget,
                                height: WorkspaceStyle.controlTarget)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                    .help("New terminal in \(worktree.task)")
                    .opacity(hovering ? (usable ? 1 : 0.45) : 0)
                    .allowsHitTesting(hovering && usable)

                    Menu {
                        // First, because it is what you do with a worktree you can see
                        // in a list but are not currently looking at — which is the
                        // only thing this menu can do that the title bar cannot.
                        EditorMenuItems(
                            worktree: worktree, onError: onEditorError,
                            showsSettingsItem: false)
                        Divider()
                        Button("New Terminal", action: onNewTerminal)
                        if let onShowChanges { Button("Show Changes", action: onShowChanges) }
                        Divider()
                        if worktree.isHidden {
                            Button("Unhide", action: onUnhide)
                        } else {
                            Button("Hide", action: onHide)
                        }
                        // Absent, not disabled, for the main checkout. A daemon-side
                        // refusal is a safety net; the button should not be there to
                        // press.
                        if worktree.worktreeMissing && !worktree.isMainCheckout {
                            Button("Dismiss", action: onRemove)
                        } else if !worktree.isMainCheckout {
                            Button("Remove Worktree…", role: .destructive, action: onRemove)
                        }
                    } label: {
                        Image(systemName: "ellipsis")
                            .font(.system(size: 11, weight: .semibold))
                            .frame(
                                width: WorkspaceStyle.controlTarget,
                                height: WorkspaceStyle.controlTarget)
                            .contentShape(Rectangle())
                    }
                    .menuStyle(.borderlessButton)
                    .menuIndicator(.hidden)
                    .fixedSize()
                    .opacity(hovering ? (usable ? 1 : 0.55) : 0)
                    .allowsHitTesting(hovering && usable)
                }
            }
            .frame(
                width: max(countsWidth, WorkspaceStyle.controlTarget * 2 + 2),
                alignment: .trailing)
            .padding(.leading, SidebarGrid.cellGap)
        }
        .frame(minHeight: ColumnGrid.twoLineRowHeight - 2 * SidebarGrid.rowVerticalPadding)
        .padding(.vertical, SidebarGrid.rowVerticalPadding)
        .padding(.horizontal, SidebarGrid.rowInset)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(
                    isSelected
                        ? WorkspaceStyle.navigatorSelection(active: windowActive)
                        : (hovering ? SidebarGrid.hoverFill : .clear))
        )
        .padding(.horizontal, SidebarGrid.highlightInset)
        .animation(Motion.snap, value: hovering)
        .contentShape(Rectangle())
        .onTapGesture { onSelect(nil) }
        .onHover { hovering = $0 }
        // On the ROW, not on the counts, and that is the whole of it.
        //
        // Its own height, because above-or-below is decided at the midpoint of
        // this row and only this row knows what that is. A background reader
        // rather than a fixed number: the row grows a second line when a branch
        // wraps, and a hardcoded midpoint would then be in the wrong place for
        // exactly the rows that are hardest to aim at.
        .background(
            GeometryReader { proxy in
                Color.clear
                    .onAppear { headerHeight = proxy.size.height }
                    .onChange(of: proxy.size.height) { _, new in headerHeight = new }
            }
        )
        // The header alone is the grab handle and the drop target, not the whole
        // section. A worktree's terminal rows are already a drag source and a
        // drop target of their own — that gesture tiles two panes together — and
        // two overlapping targets accepting the same type is how a drop lands on
        // whichever one happened to win the hit test.
        //
        // Offered only when `reorderable` says so, and ABSENT otherwise rather
        // than a drag that goes nowhere: a runner too old to keep an order
        // used to be handed a drag it answered with "unknown method", and the
        // row sprang back with nothing said. See `WorktreeDrag.offersDrag`.
        // The drag's menu equivalent (ruling 5): a drag nobody has been
        // told about needs somewhere to be discovered.
        .contextMenu {
            if let onShowChanges, usable { Button("Show Changes", action: onShowChanges) }
            if !moveTargets.isEmpty, usable {
                Menu("Move to Workspace") {
                    ForEach(moveTargets) { target in Button(target.name) { onMove(target) } }
                }
            }
        }
        .modifier(WorktreeDragSource(worktree: worktree.id, enabled: reorderable))
        .onDrop(
            of: [.text],
            delegate: WorktreeDropTarget(worktree: worktree.id, height: headerHeight))
        // Where it would land, drawn on the edge the drop would insert at. An
        // insertion line rather than highlighting the row: the question is which
        // GAP the card goes in, and a lit row says "on top of this one", which
        // is a thing this gesture cannot do.
        .overlay(alignment: dropEdge == .above ? .top : .bottom) {
            if dropEdge != nil {
                Rectangle()
                    .fill(Color.accentColor)
                    .frame(height: 2)
                    .padding(.horizontal, SidebarGrid.highlightInset)
            }
        }
        .animation(Motion.snap, value: dropEdge)
    }

    /// The edge an insertion line goes on, or nil when this row is not where
    /// the drag would land.
    private var dropEdge: WorktreeOrder.Edge? {
        drag.landing(on: worktree.id)
    }

    /// The pair, in color while there is something new here and gray once it
    /// has been read.
    ///
    /// The daemon has kept a per-worktree watermark since this row was written
    /// and the sidebar drew none of it: a worktree you had finished reading
    /// looked exactly like one an agent had just rewritten. The phone gated its
    /// whole Needs You list on `changedSinceReviewed && hasDiff`
    /// (`FleetView.rule(for:inbox:)`) while the Mac's answer to the same
    /// question was the counts alone, which stay true forever and therefore
    /// answer "is there a branch here" rather than "is there anything new".
    ///
    /// A state on the element already in the row rather than a new one beside
    /// it. This adds no glyph, no column and no width — `countsWidth` measures
    /// a string this does not change — which is what keeps a watermark from
    /// becoming a second badge in a list that is meant to be hundreds of rows
    /// of one line each. And the contrast is the one the app has already
    /// written down: `WorkspaceStyle.navigatorSelection(active:)` grays an
    /// inactive selection because "the question an inactive selection answers
    /// is 'this is remembered, not live'", which is exactly what a read diff is
    /// saying. Gray against color is also a saturation channel and not a hue
    /// one, so it survives the color blindness `+`/`−` against green/red does
    /// not — and the accessibility label above says it in words regardless.
    ///
    /// Nothing changes appearance until somebody uses it. The daemon treats a
    /// worktree that was never marked read as changed, so on a Mac that has
    /// never marked anything — and never had a way to — every row draws exactly
    /// what it drew before. The gray state appears the first time the reader
    /// chooses Mark as Reviewed, or the first time their phone does.
    private func changeCountsText(_ changes: InboxRow) -> Text {
        guard changes.changedSinceReviewed else {
            // One run, one color: the sign characters carry the polarity on
            // their own, and two grays either side of a space would be a
            // distinction the reader is asked to look for and find nothing in.
            var read = AttributedString(
                DiffCounts.pair(insertions: changes.insertions, deletions: changes.deletions))
            read.foregroundColor = .secondary
            return Text(read)
        }
        var additions = AttributedString(DiffCounts.added(changes.insertions))
        additions.foregroundColor = .green
        var deletions = AttributedString(" " + DiffCounts.removed(changes.deletions))
        deletions.foregroundColor = .red
        additions.append(deletions)
        return Text(additions)
    }
}

/// A project heading.
///
/// Projects are chrome, not content: there are three or four and they change
/// rarely. A quiet label separating groups of worktrees is all the weight they
/// deserve.
struct ProjectHeader: View {
    let name: String
    let count: Int
    /// Adding to THIS project, rather than to whichever one a sheet defaults to.
    ///
    /// The only way to start a worktree was the sidebar's own `+`, which opens a
    /// sheet with a repository picker — so adding to the project you were
    /// already looking at meant re-choosing it. With several projects that is
    /// the common case, not the edge one.
    var onNewWorktree: (() -> Void)?
    /// A terminal in the repository's own checkout, not in a worktree.
    var onNewTerminal: (() -> Void)?
    /// Opens `RemoveRepositorySheet` for this project. `nil` for a silent
    /// host's placeholder header — it names a runner, not a repository,
    /// and there is nothing there to remove.
    var onRemove: (() -> Void)?
    /// Which runner this project's worktrees are on.
    var host: String = ""
    var hostState: HostState = .connected
    /// This runner's daemon, when it is one the app is offering to replace.
    /// Nil is the ordinary case — a runner running the build this app ships
    /// has nothing to say here. See `HostDot`.
    var daemonUpdate: DaemonUpdateTarget?
    /// Whether to name the runner at all — noise on a fleet of one.
    var showHost: Bool = false
    var onReconnect: () -> Void = {}
    /// Whether this project's worktrees are hidden right now.
    var isCollapsed: Bool = false
    /// `nil` for a silent host's placeholder header, which names a runner and
    /// has no worktrees under it to collapse.
    var onToggleCollapse: (() -> Void)?

    @State private var hovering = false

    /// A repository's header in the shared section's sidebar metrics, at
    /// the height its `+` and `…` need.
    static let metrics = SectionMetrics(
        spacing: 0, chevronWidth: SidebarGrid.chevronColumn, minHeight: SidebarGrid.control,
        headerInsets: EdgeInsets(top: 0, leading: SidebarGrid.edge, bottom: 0, trailing: SidebarGrid.edge),
        isHeading: true)

    var body: some View {
        Group {
            if let onToggleCollapse {
                // The shared section (ov-101): its chevron at A and its name
                // at B, a workspace's glyph column, where ov-83 had the name
                // flush at A and Finder's Show and Hide words trailing. The
                // owner's rule is one collapsible, its chevron on the left.
                // Its workspaces are its siblings in the sidebar's flat list,
                // not its content, so the section holds nothing; they come
                // and go with `BoardMotion.rowTransition` as it toggles.
                CollapsibleSection(
                    id: "repository.\(name)", metrics: Self.metrics,
                    isExpanded: Binding(
                        get: { !isCollapsed }, set: { open in if open == isCollapsed { onToggleCollapse() } }),
                    accessibilityLabel: name, fillsRow: false,
                    label: { _ in title },
                    accessory: { trailing },
                    content: { EmptyView() })
            } else {
                // A silent runner's header keeps its machine icon at A,
                // which is what says it names a runner, and its name at B,
                // like Needs You. It has nothing under it to collapse.
                SidebarRow {
                    HStack(spacing: 0) {
                        Image(systemName: "desktopcomputer")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(.tertiary)
                            .frame(width: SidebarGrid.glyphColumn, alignment: .leading)
                            .gridMark("runner", .icon)
                        title
                        trailing
                    }
                    .frame(minHeight: SidebarGrid.control)
                }
            }
        }
        .padding(.top, SidebarGrid.projectTopPadding)
        .padding(.bottom, SidebarGrid.projectBottomPadding)
        .onHover { hovering = $0 }
        // The `+` and the `…` arrive at the same instant. Cut hard, that is
        // things appearing out of nothing under a pointer that only grazed
        // the row. `Motion.snap` is the app's own "instant, but not a cut" —
        // 0.22s.
        .animation(Motion.snap, value: hovering)
    }

    private var title: some View {
        HStack(spacing: 6) {
            Text(name)
                .font(WorkspaceStyle.sectionTitle)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .gridMark(onToggleCollapse == nil ? "runner" : "repository", .text)
            if showHost {
                Text(host.isEmpty ? "This Mac" : host)
                    .font(.system(size: 10.5))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
            }
        }
    }

    /// The connection's dot after the name, and the `+` and `…` at the
    /// trailing edge, on hover.
    @ViewBuilder private var trailing: some View {
        HostDot(state: hostState, onReconnect: onReconnect, update: daemonUpdate)
            .padding(.leading, SidebarGrid.cellGap)
        Spacer(minLength: SidebarGrid.gap)
        HStack(spacing: 0) {
            if onNewWorktree != nil || onNewTerminal != nil {
                // A Button rather than a Menu, so it sits in exactly the same
                // column as the sidebar header's `+`.
                SidebarMenuButton(
                    systemImage: "plus",
                    help: "Add to \(name)",
                    items: [
                        onNewWorktree.map {
                            SidebarMenuItem(title: "New Worktree in \(name)…", action: $0)
                        },
                        // The main checkout is a place people work — a quick
                        // build, a look at main while a worktree is
                        // mid-changes — and it was the one directory this app
                        // could not open a terminal in.
                        onNewTerminal.map {
                            SidebarMenuItem(title: "New Terminal in \(name)", action: $0)
                        },
                    ].compactMap { $0 })
                // Shown on hover, like every other per-row control in a
                // sidebar. A `+` on every project header at all times is a
                // column of plus signs down a list meant to read as quiet
                // section labels.
                .opacity(hovering ? 1 : 0)
            }

            if let onRemove {
                // Its own button rather than a second item on the `+`
                // menu: that menu is for adding things, and a destructive
                // action one row below "New Worktree" is a misclick away
                // from removing a repository instead of branching one.
                SidebarMenuButton(
                    systemImage: "ellipsis",
                    help: "\(name) options",
                    items: [SidebarMenuItem(title: "Remove \(name)…", action: onRemove)])
                .opacity(hovering ? 1 : 0)
            }
        }
    }
}

/// A runner's connection, said as quietly as possible.
///
/// Absent when healthy: a dot that is always there is a dot nobody reads, and
/// the whole point is that you notice it only when something is wrong.
/// Reconnection is neutral and silent; only a runner that has given up is red,
/// and clicking it retries at once rather than waiting out the backoff.
///
/// Reconnection was amber, and that was an honest reading of it before the
/// palette had a rule: amber for the middle state, neither well nor dead. It
/// cannot stay one, because amber now means exactly one thing across all three
/// apps — an agent is waiting on you — and a socket coming back up is nobody
/// waiting. `Status.tint(_:)` already paints `working` and `starting` in
/// `GlancePalette.ink2` — §01's quiet ink, and the same one the mark's own
/// hairline ring uses — for that reason; a connection in progress is the
/// fleet-level version of the
/// same sentence, and iOS says it in the same word. The cost is that this and
/// `.notInstalled` now draw the same mark, separated only by their help text:
/// the shape channel that would have split them is not available at 5pt, where
/// a 1.5pt ring is a soft dot rather than a hollow one.
///
/// A stale daemon is the fourth thing that can be wrong with a runner, and the
/// first one that is not about whether it answers. It rides in this same column
/// rather than in a control of its own, because "does this runner need
/// something from me" is one question and one glance — see `DaemonSkewDot`,
/// which draws it, and `DaemonSkew`, which decides when there is anything to
/// draw. Connection always wins: `daemonSkew` is `.unavailable` for every state
/// but `.connected` (and for the one refused handshake that is really a version),
/// so a runner that is merely reconnecting cannot lose its connection dot to
/// yesterday's version news.
struct HostDot: View {
    let state: HostState
    let onReconnect: () -> Void
    /// Non-nil only when this runner's daemon is one the app has grounds to
    /// offer to replace. Defaulted, so the call sites that have no fleet
    /// behind them — and any future one — read exactly as they did.
    var update: DaemonUpdateTarget?

    var body: some View {
        if let update, update.skew.offersUpdate {
            DaemonSkewDot(target: update)
        } else {
            connection
        }
    }

    @ViewBuilder
    private var connection: some View {
        switch state {
        case .connected:
            EmptyView()
        case .connecting, .reconnecting:
            Circle()
                .fill(Color.secondary)
                .frame(width: 5, height: 5)
                .help("Reconnecting to this runner")
        case .unreachable(let why):
            Button(action: onReconnect) {
                Circle().fill(Color.red).frame(width: 5, height: 5)
            }
            .buttonStyle(.plain)
            .help("\(why) — click to retry now")
        case .notInstalled:
            Button(action: onReconnect) {
                Circle().fill(Color.secondary).frame(width: 5, height: 5)
            }
            .buttonStyle(.plain)
            .help("Far Cooler is not installed on this runner — open Settings > Runners")
        }
    }
}

/// One terminal row.
///
/// A single line, deliberately. Both levels used to be two-line blocks, which
/// gave the list no rhythm: a worktree and the terminal under it were the same
/// shape and the same height, so nothing read as containing anything. A
/// two-line heading over single-line items is the difference between a tree and
/// a run of similar rectangles.
/// A layout's name, or its number when the name IS its number.
private func layoutLabel(_ group: PaneGroup, position: Int) -> String {
    group.name == "\(position)" ? "Move to layout \(position)" : "Move to \u{201c}\(group.name)\u{201d}"
}

struct TerminalRow: View {
    let terminal: Terminal
    let isSelected: Bool
    let onSelect: () -> Void
    /// On screen as part of the current layout.
    var isTiled: Bool = false
    var layouts: [PaneGroup] = []
    /// `nil` means a new layout of its own.
    var onMoveToLayout: (PaneGroup?) -> Void = { _ in }
    var onDropTogether: (String) -> Void = { _ in }
    let onAction: (TerminalAction) -> Void

    @State private var hovering = false
    @State private var targeted = false
    /// See `WorkspaceStyle.navigatorSelection(active:)`.
    ///
    /// `.key` and not `!= .inactive`: with two Far Cooler windows open the
    /// second one is `.active` — the app has focus, this window does not — and
    /// that is precisely the case the dimming exists for.
    @Environment(\.controlActiveState) private var controlActiveState
    private var windowActive: Bool { controlActiveState == .key }

    private var status: Status { terminal.status }

    /// Which of several identical ones this is, or nothing.
    ///
    /// Two shells in a worktree are genuinely alike, so they get `2` and `3` —
    /// but only when there is something to tell apart. Numbering everything was
    /// the old behavior and it labeled a lone `claude` as "Terminal 7", which
    /// answers a question nobody asked.
    var ordinal: Int?
    /// Whether this terminal's runner can be acted on right now. See
    /// `WorktreeSection.usable`, which this mirrors row by row.
    var usable: Bool = true
    /// Use as Orchestrator, or Stop Being Orchestrator, or neither.
    var roleOffer: OrchestratorAdoption.Offer?

    /// Whether the status is worth saying at all, which most of the time it
    /// is not. Asked OUTSIDE the row's clock, because it is a question about
    /// the status and not about the time — a slot that appears and disappears
    /// on a timer would take its spacing with it.
    private var showsMeta: Bool { status.wantsAttention || status == .working }

    /// The status, and how long it has held.
    ///
    /// `now` comes from the row's own clock rather than from `Date()` — see
    /// `Ticking`, and `Terminal.displayDuration(at:)` for what reading the
    /// wall clock inside a view costs.
    private func meta(at now: Date) -> String {
        terminal.displayDuration(at: now).map { "\(status.label) \($0)" } ?? status.label
    }

    /// Whether this row's clock is running. Only these two states have a
    /// duration to show at all, so every other row is drawn once and left
    /// alone.
    private var ticking: Bool { status == .working || status == .blocked }

    /// The gap between the name and what follows it on the status line.
    private static let markerGap: CGFloat = 7

    /// How far the name, and the feed's lines under it, start from the
    /// status glyph: one grid column, the glyph's cell (ov-83).
    ///
    /// Named rather than written twice: the feed's lines below have to begin
    /// in the same column the terminal's name does, and two hand-written
    /// numbers is exactly how a column goes out of alignment.
    private static let textOffset: CGFloat = SidebarGrid.glyphColumn

    /// The row's leading inset inside its highlight: two steps past the
    /// band's.
    ///
    /// A terminal's status glyph sits under its worktree's title, past the
    /// worktree's chevron and branch glyph (ov-83), and never under either.
    /// ov-63 had it take no step, which put a column of dots under a column
    /// of collapsed arrows.
    static let leading: CGFloat = SidebarGrid.rowInset + 2 * SidebarGrid.gutter

    /// Where a terminal's glyph and name start, from the sidebar's edge,
    /// under a worktree drawn at `depth`.
    static func columns(depth: Int) -> (glyph: CGFloat, text: CGFloat) {
        let glyph = SidebarGrid.indent(depth) + SidebarGrid.highlightInset + leading
        return (glyph, glyph + textOffset)
    }

    var body: some View {
        // A column, not a row, since this task: the status line reads across
        // and the feed reads down under it. Everything that positions the row
        // — padding, highlight, drop target — stays on the outside of this, so
        // a row with three steps is one taller row rather than four rows.
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: Self.markerGap) {
                StatusGlyph(status: status)
                    .frame(width: Self.textOffset, alignment: .leading)
                    .gridMark("terminal", .icon)
                    // The cell is the whole step to the name: take back the
                    // stack's spacing, which is for the marks after it.
                    .padding(.trailing, -Self.markerGap)

                Text(terminal.label)
                    .font(.system(size: 12.5))
                    .lineLimit(1)
                    .gridMark("terminal", .text)
                    // The name yields before the status does. Which terminal it
                    // is matters less than what it wants, and the sidebar is
                    // narrow.
                    .layoutPriority(0)

                if let ordinal {
                    Text("\(ordinal)")
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(.tertiary)
                        .layoutPriority(1)
                }

                if showsMeta {
                    Ticking(paused: !ticking) { now in
                        Text(meta(at: now))
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    .layoutPriority(1)
                }

                Spacer(minLength: 0)

                // One small mark for "this one is on screen right now", so the
                // fourth agent running in the background is visibly the odd one
                // out rather than indistinguishable from the three you
                // arranged.
                if isTiled {
                    Image(systemName: "square.split.2x2")
                        .font(.system(size: 9))
                        .foregroundStyle(isSelected ? .primary : .tertiary)
                }

                if status == .lost {
                    Button("Dismiss") { onAction(.dismissLost) }
                        .buttonStyle(.borderless)
                        .font(.system(size: 11))
                        .opacity(usable ? 1 : 0.55)
                        .allowsHitTesting(usable)
                }
            }

            // Where the agent IS: the question it is blocked on, its place in
            // its own task list, or what it is doing right now.
            //
            // The most valuable line on the row, and the reason the blocked
            // question no longer sits inside the header above. Beside the name
            // it was the first thing squeezed out by a narrow sidebar; on its
            // own line it takes the full width and truncates at the tail like
            // everything else here. Which of the three it says was decided on
            // the host — see `Terminal.signalLine`.
            //
            // A step brighter than the transcript under it: the row reads
            // status, then where it is, then what it has been saying.
            if let signal = terminal.signalLine {
                Text(signal)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .gridMark("terminal.signal", .text)
                    .padding(.leading, Self.textOffset)
                    // Take the width offered and no more — see the transcript
                    // below, where the same modifier keeps the widest line from
                    // setting the sidebar's ideal width.
                    .frame(maxWidth: .infinity, alignment: .leading)
            }

            // The last few things the agent SAID, in its own words.
            //
            // Under the row rather than beside it, and that is what keeps the
            // narrow-sidebar promise the signal line makes one line up: a line
            // of its own cannot take width from the terminal's name at all,
            // and each one truncates at the tail when the column is too narrow
            // for it. These arrive already cut to a row's width by the host, so
            // this is the second cut and not the first — no client decides
            // where an ellipsis goes.
            //
            // Not gated on the status: an idle or finished agent keeps its
            // lines, because "what did it do while I was away" is precisely
            // when the summary is worth most. That also means a row never
            // shrinks on going idle or grows on waking up — a sidebar that
            // rearranges itself while it is being read is worse than a long
            // one.
            if !terminal.recentSteps.isEmpty {
                VStack(alignment: .leading, spacing: 1) {
                    // Indexed rather than keyed by the step itself: an agent
                    // that runs the same command twice would otherwise hand
                    // `ForEach` two identical ids and lose a line.
                    ForEach(Array(terminal.recentSteps.enumerated()), id: \.offset) { _, step in
                        Text(step)
                            .font(.system(size: 10.5))
                            // A step further back than the question is: the row
                            // reads status, then detail, then history.
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .gridMark("terminal.step", .text)
                    }
                }
                // Aligned under the terminal's name, not under its status dot,
                // so the steps read as belonging to the row rather than as a
                // second column of their own.
                .padding(.leading, Self.textOffset)
                // Take the width offered and no more. Without this the widest
                // step would set the row's ideal width and the sidebar would
                // report wanting to be wider than the name ever needed.
                .frame(maxWidth: .infinity, alignment: .leading)
            }

            // The agents this one has running, one line each.
            //
            // Named rather than counted here, because a sidebar has the room:
            // the COUNT is already on the signal line above, for the surfaces
            // that do not. The branch mark reads as work happening underneath
            // this row rather than as another thing the row itself said.
            //
            // Gone the moment they finish — unlike the transcript, which is
            // kept. "Two agents are running" stops being true when they stop.
            if !terminal.runningSubagents.isEmpty {
                VStack(alignment: .leading, spacing: 1) {
                    ForEach(Array(terminal.runningSubagents.enumerated()), id: \.offset) { _, agent in
                        Text("⑂ \(agent)")
                            .font(.system(size: 10.5))
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .gridMark("terminal.subagent", .text)
                    }
                }
                // Under the name like the lines above, not a step further:
                // the branch mark says they're underneath.
                .padding(.leading, Self.textOffset)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(.vertical, SidebarGrid.rowVerticalPadding)
        // The highlight sits inside the band exactly as the worktree row's
        // does; the content one step in from it: see `leading`.
        .padding(.leading, Self.leading)
        .padding(.trailing, SidebarGrid.rowInset)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(
                    isSelected
                        ? WorkspaceStyle.navigatorSelection(active: windowActive)
                        : (hovering ? SidebarGrid.hoverFill : .clear))
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .strokeBorder(Color.accentColor, lineWidth: targeted ? 2 : 0))
        )
        .padding(.horizontal, SidebarGrid.highlightInset)
        .contentShape(Rectangle())
        .onTapGesture(perform: onSelect)
        .onHover { hovering = $0 }
        .animation(Motion.snap, value: hovering)
        // A row is a drag source for the panes as well as for other rows, so the
        // same gesture grows a layout and rearranges one.
        //
        // `.onDrag` rather than `.draggable`: the pane a row is dropped on has to
        // know which terminal is coming while the pointer is still moving, so it
        // can show which half it would land in, and reading that back out of an
        // item provider is asynchronous. See `PaneDrag`.
        .onDrag {
            // Dragging arranges panes, which is a mutation like any other —
            // guarded here rather than by hiding the gesture, because the row
            // still has to stay a normal drop TARGET for other terminals'
            // drags even when this runner cannot itself be rearranged.
            guard usable else { return NSItemProvider() }
            MainActor.assumeIsolated { PaneDrag.shared.begin(terminal.id) }
            return NSItemProvider(object: terminal.id as NSString)
        }
        .onDrop(of: [.text], isTargeted: $targeted) { _ in
            guard usable, let dragged = PaneDrag.shared.terminal, dragged != terminal.id else {
                return false
            }
            PaneDrag.shared.end()
            onDropTogether(dragged)
            return true
        }
        .contextMenu {
            switch roleOffer {
            case .use?:
                Button("Use as Orchestrator") { onAction(.useAsOrchestrator) }.disabled(!usable)
                Divider()
            case .stepDown?:
                Button("Stop Being Orchestrator") { onAction(.stopBeingOrchestrator) }.disabled(!usable)
                Divider()
            case nil:
                EmptyView()
            }
            Button("Move to Its Own Layout") { onMoveToLayout(nil) }
                .disabled(!usable)
            if !layouts.isEmpty {
                Divider()
                ForEach(Array(layouts.enumerated()), id: \.element.id) { index, group in
                    Button(layoutLabel(group, position: index + 1)) {
                        onMoveToLayout(group)
                    }
                    .disabled(!usable || group.terminals.contains(terminal.id))
                }
            }
        }
    }
}

/// A worktree's own status, for its heading.
///
/// Worktrees have three states worth showing and no agent of their own, so
/// this stays a small dot: it is context for the heading rather than something
/// to scan, and the terminals underneath carry the detail.
///
/// The three are the exceptions — gone, broken, hidden. `active` was a fourth,
/// painted green, and that was the drift: "three states worth showing" was
/// right and the switch had grown a case for the state that is worth showing
/// least. `derive_worktree` returns `Active` for any worktree with a live
/// terminal, which is nearly all of them nearly all of the time, so the green
/// was on almost every heading — and green in this app means `Status.done`,
/// "the turn ended and nobody has looked yet". A worktree being in use and an
/// agent having finished are close to opposites, and they drew the same mark.
/// That is the phones' bug — an idle `zsh` and a finished agent in one green —
/// which the Mac was credited with not having.
///
/// `Active` is also what `derive_worktree` returns when the runner is
/// unreadable and every terminal derives `Unknown`, deliberately, so the green
/// was vouching for worktrees nobody had heard from. `StatusGlyph` paints that
/// same not-yet-answered case `.secondary` for exactly this reason.
///
/// iOS has nothing to keep in sync here: its worktree heading draws the
/// rolled-up status of the terminals inside plus "worktree gone", and never the
/// worktree's own state. So this is a one-platform change by construction, not
/// one half of a pair.
struct WorktreeDot: View {
    let state: String

    private var color: Color {
        switch StateKind.parse(state) {
        case .error: return .red
        case .hidden: return Color.secondary.opacity(0.4)
        // Red, not a dimmed amber. `StatusGlyph` spends amber on one state —
        // an agent is waiting on you — and a directory that is gone is not
        // waiting for anything. It is the worktree-level `Status.lost`, and
        // `lost` is red. The soft orange was a third reading from before there
        // was a rule: less alarming than `error`, warmer than `hidden`.
        //
        // It shares red with `error` now, which is affordable HERE and would
        // not be in a list: this dot appears once, beside a 24pt title, never
        // next to another one, and its help text names the state.
        case .worktreeMissing: return .red
        // `active` falls here with `ready` and `creating`: a healthy worktree
        // is a healthy worktree, and the difference between one with a live
        // pane and one without is a fact the rows underneath state outright.
        default: return .secondary
        }
    }

    var body: some View {
        Circle().fill(color).frame(width: 8, height: 8).help(state)
    }
}

/// Worktree detail.
///
/// Used to open with a State / Terminals / Worktree key-value table, which read
/// like a database inspector and gave the most screen to a filesystem path
/// nobody needs. The terminals ARE the worktree, so they lead; the path is a
/// footnote you can copy when you want it.
struct WorktreeDetail: View {
    /// An orchestrator running here, drawn in its workspace's conversation
    /// column instead, and how to go there.
    struct Hosted {
        let name: String
        let go: () -> Void
    }

    /// The worktree, without the orchestrators in `hosted`.
    let worktree: Worktree
    var hosted: [Hosted] = []
    let onNewTerminal: () -> Void
    let onHide: () -> Void
    let onUnhide: () -> Void
    let onRemove: () -> Void
    let onOpenTerminal: (Terminal) -> Void
    /// Restart or Dismiss, from a card's context menu (ov-191).
    var onTerminalAction: (TerminalAction, Terminal) -> Void = { _, _ in }
    /// Show Changes, which opens its changes pane on its own when it has no
    /// terminal (ov-78); nil when its runner can't read changes.
    var onShowChanges: (() -> Void)?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                header

                if !hosted.isEmpty { hostedNote }

                if worktree.terminals.isEmpty {
                    empty
                } else {
                    VStack(spacing: 8) {
                        // Creation order, for the reason `WorktreeSection`
                        // gives: cards that rearrange themselves under the
                        // pointer are worse than cards you have to read.
                        ForEach(worktree.terminals) { t in
                            terminalCard(t)
                        }
                    }
                }

                footnote
            }
            .padding(32)  // grid-exempt: a detail page's margin, not a row
            .frame(maxWidth: 720, alignment: .leading)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        // Same title as the tiled and solo views, so switching between them does
        // not change what the window claims you are looking at.
        .navigationTitle(worktree.windowTitle)
        .navigationSubtitle(worktree.windowSubtitle)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 10) {
                    WorktreeDot(state: worktree.state)
                    Text(worktree.task).font(.system(size: 24, weight: .semibold))
                }
                Text(worktree.branch)
                    .font(.system(size: 13, design: .monospaced))
                    .foregroundStyle(.secondary)
            }

            HStack(spacing: 8) {
                Button(action: onNewTerminal) {
                    Label("New Terminal", systemImage: "plus")
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut("t", modifiers: .command)

                if let onShowChanges {
                    Button(action: onShowChanges) {
                        Label("Show Changes", systemImage: "plusminus")
                    }
                }

                // No "Tile all" any more. It gathered every terminal into one
                // arrangement, which was possible only while membership was a list
                // this app maintained; a terminal is a tmux window now, and putting
                // twelve of them in one window means twelve nested splits nobody
                // asked for. Panes come together one drag at a time, where you can
                // see what you are making.

                Spacer()

                // Destructive actions live in a menu rather than sitting as
                // permanent buttons next to the one you press constantly.
                Menu {
                    if worktree.isHidden {
                        Button("Unhide", action: onUnhide)
                    } else {
                        Button("Hide", action: onHide)
                    }
                    // Absent, not disabled, for the main checkout. A
                    // daemon-side refusal is a safety net; the button should
                    // not be there to press.
                    if !worktree.isMainCheckout {
                        Button("Remove Worktree…", role: .destructive, action: onRemove)
                    }
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
            }
        }
    }

    /// Where the orchestrators that run here are drawn, since it isn't here.
    private var hostedNote: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: "person.wave.2")
                .foregroundStyle(.secondary)
            Text(Self.hostedSentence(hosted.map(\.name)))
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            ForEach(Array(hosted.enumerated()), id: \.offset) { _, seat in
                Button(hosted.count == 1 ? "Show Orchestrator" : "Show \(seat.name)", action: seat.go)
            }
        }
    }

    /// "The orchestrator runs here…", for the workspaces whose orchestrators
    /// run in this worktree.
    static func hostedSentence(_ names: [String]) -> String {
        switch names.count {
        case 0: return ""
        case 1: return "The orchestrator runs here. It’s in the Orchestrator column."
        default:
            let list = ListFormatter.localizedString(byJoining: names)
            return "The orchestrators for \(list) run here. Each is in its workspace’s Orchestrator column."
        }
    }

    private var empty: some View {
        VStack(spacing: 8) {
            Image(systemName: "terminal")
                .font(.system(size: 28))
                .foregroundStyle(.tertiary)
            Text("No terminals").font(.callout.weight(.medium))
            Text("A terminal runs one agent, or one shell, inside this worktree.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 40)
        .background(RoundedRectangle(cornerRadius: 10).fill(Color.primary.opacity(0.03)))
    }

    private func terminalCard(_ t: Terminal) -> some View {
        Button {
            onOpenTerminal(t)
        } label: {
            HStack(spacing: 12) {
                StatusGlyph(status: t.status)

                VStack(alignment: .leading, spacing: 2) {
                    Text(t.label).font(.system(size: 14, weight: .medium))
                    Text(t.preset).font(.system(size: 12)).foregroundStyle(.secondary)
                }

                Spacer()

                Ticking(paused: t.status != .working && t.status != .blocked) { now in
                    Text(t.displayDuration(at: now).map { "\(t.status.label) \($0)" } ?? t.status.label)
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                }

                Image(systemName: "chevron.right")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.vertical, 12)
            .padding(.horizontal, 14)  // grid-exempt: a card's inset on the detail page
            .background(
                RoundedRectangle(cornerRadius: 10)
                    .fill(Color.primary.opacity(0.04))
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu { cardMenu(t) }
    }

    /// Open, and for a terminal with no running pane the page's own two
    /// answers (ov-191): a lost card used to do nothing, clicked or not.
    @ViewBuilder
    private func cardMenu(_ t: Terminal) -> some View {
        Button("Open") { onOpenTerminal(t) }
        if let kind = LostPane.Kind(state: t.state) {
            Divider()
            ForEach(LostPane.actions(for: kind), id: \.title) { action in
                Button(action.title) { onTerminalAction(TerminalAction(action), t) }
            }
        }
    }

    /// The path, demoted to where it belongs.
    private var footnote: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "folder")
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
            Text(worktree.path)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.tertiary)
                .textSelection(.enabled)
                .lineLimit(2)
                .truncationMode(.middle)
        }
    }
}

/// The worktrees a project has been told to stop showing.
///
/// A section rather than a filter, because hiding is reversible and something
/// reversible needs a way back that is not the Settings window. Collapsed by
/// default: the whole point of hiding is that these are not in the way.
///
/// The attention dot on the header is what makes hiding safe to allow while an
/// agent runs. The daemon no longer refuses that — a view preference that fails
/// with an error reads as a bug — so this is where "something in here wants you"
/// gets said.
struct HiddenWorktrees: View {
    let project: String
    let worktrees: [Worktree]
    let isExpanded: Bool
    let onToggle: () -> Void
    let onUnhide: (Worktree) -> Void

    private var attention: Int {
        worktrees.flatMap(\.terminals).filter(\.status.wantsAttention).count
    }

    /// The color for that count — the same rule the rows inside use, so
    /// expanding the section cannot change what it was already saying.
    private var attentionStatus: Status? {
        Status.mostUrgent(in: worktrees.flatMap(\.terminals).map(\.status))
    }

    var body: some View {
        SidebarGroupSection(
            title: "Hidden", id: "group.hidden", glyph: "eye.slash", count: worktrees.count,
            attention: attentionStatus,
            attentionHelp: "\(attention) waiting on you, inside a hidden worktree",
            isExpanded: isExpanded, onToggle: onToggle
        ) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(worktrees) { ws in
                    // At D, where a worktree's title is: these are
                    // worktrees, without their chevrons or glyphs.
                    SidebarRow(indent: 3) {
                        HStack(spacing: SidebarGrid.gap) {
                            Text(ws.task)
                                .font(.system(size: 12))
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .gridMark("hidden", .text)
                            Text(ws.branch)
                                .font(.system(size: 10, design: .monospaced))
                                .foregroundStyle(.tertiary)
                                .lineLimit(1)
                            Spacer(minLength: 0)
                            Button("Unhide") { onUnhide(ws) }
                                .buttonStyle(.plain)
                                .font(.system(size: 11))
                                .foregroundStyle(.secondary)
                        }
                        .contentShape(Rectangle())
                    }
                    .frame(minHeight: ColumnGrid.rowHeight - 2 * SidebarGrid.secondaryRowVerticalPadding)
                    .padding(.vertical, SidebarGrid.secondaryRowVerticalPadding)
                }
            }
        }
    }
}

/// A workspace in the sidebar: a place you select, which shows its
/// orchestrator's conversation beside its board (spec §4.5).
///
/// Its chevron, in the leading column (its worktrees' disclosure, ov-78);
/// an unread dot for a finished turn nobody has seen (ruling 10); and at the
/// trailing edge the orchestrator's status glyph, or a dashed circle with
/// none, and the workspace's needs-you count in amber. The task prefix is in the tooltip. It keeps
/// ov-60's menu and is still where a dragged worktree is dropped to move it.
struct WorkspaceRow: View {
    /// What a workspace is drawn as in column B: a stack of tasks, its
    /// board.
    static let glyph = "rectangle.stack"

    let name: String
    /// The workspace's id: what a worktree dropped here is assigned to.
    let workspace: String
    let taskPrefix: String
    /// Its seated orchestrator, or nil for none. `implicit` draws no glyph:
    /// a runner without workspaces has no orchestrators.
    let seat: BoardPane?
    let implicit: Bool
    let count: Int
    let unread: Bool
    let isSelected: Bool
    let onSelect: () -> Void
    /// What the context menu does, or nil for no menu.
    let actions: WorkspaceHeaderActions?
    /// Whether its worktrees are listed under it. The row is their
    /// disclosure (ov-78): only its chevron opens and closes it, and a click
    /// anywhere else selects the workspace, as before.
    var isOpen = false
    var onToggle: () -> Void = {}
    /// New Worktree…, claimed for this workspace, on the row's menu; nil
    /// when the runner can't be acted on.
    var onNewWorktree: (() -> Void)?

    @ObservedObject private var drag = WorktreeDrag.shared
    @State private var hovering = false
    @Environment(\.colorScheme) private var scheme
    @Environment(\.controlActiveState) private var controlActiveState
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var windowActive: Bool { controlActiveState == .key }

    var body: some View {
        HStack(alignment: .center, spacing: 0) {
            // Chevron at column A, glyph at B, name at C (ov-83): a
            // worktree's chevron sits under this glyph, its own glyph under
            // this name.
            DisclosureButton(
                expanded: isOpen, accessibilityLabel: "Worktrees in \(name)", gridRow: "workspace",
                width: SidebarGrid.chevronColumn, action: onToggle)
            // The row, combined, says Expanded or Collapsed and has the
            // toggle as a named action: read twice otherwise (ov-101 review).
            .accessibilityHidden(true)

            Image(systemName: Self.glyph)
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(.secondary)
                .frame(width: SidebarGrid.glyphColumn, height: 16, alignment: .leading)
                .gridMark("workspace", .icon)
                .accessibilityHidden(true)

            Text(name)
                .font(WorkspaceStyle.sidebarPrimary)
                .lineLimit(1)
                .gridMark("workspace", .text)
            if unread {
                Circle()
                    .fill(GlancePalette.amber(scheme))
                    .frame(width: 5, height: 5)
                    .padding(.leading, SidebarGrid.cellGap)
                    .help("The orchestrator finished a turn you haven’t seen")
                    .accessibilityLabel("Unread")
            }
            Spacer(minLength: 6)
            // The orchestrator's status, at the trailing edge beside the
            // count, now the chevron has the leading column.
            Group {
                if implicit {
                    Image(systemName: "square.stack.3d.up")
                        .font(.system(size: 9.5, weight: .medium))
                        .foregroundStyle(.tertiary)
                } else if let seat {
                    StatusGlyph(status: seat.terminal.status)
                } else {
                    Image(systemName: "circle.dashed")
                        .font(.system(size: 9, weight: .medium))
                        .foregroundStyle(.tertiary)
                        .help("No orchestrator")
                }
            }
            .padding(.leading, SidebarGrid.cellGap)
            // A gap between the glyph and the count: "○1" read as one mark
            // (ov-81 P13).
            .padding(.trailing, count > 0 ? SidebarGrid.markGap : 0)
            if count > 0 {
                Text("\(count)")
                    .font(.system(size: 11, weight: .semibold))
                    .monospacedDigit()
                    .foregroundStyle(GlancePalette.amber(scheme))
                    .help(count == 1 ? "1 thing needs you here" : "\(count) things need you here")
                    .accessibilityLabel(count == 1 ? "1 needs you" : "\(count) need you")
            }
        }
        .frame(minHeight: ColumnGrid.rowHeight - 2 * SidebarGrid.rowVerticalPadding)
        .padding(.vertical, SidebarGrid.rowVerticalPadding)
        .padding(.horizontal, SidebarGrid.rowInset)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(
                    drag.workspaceLanding == workspace
                        ? Color.accentColor.opacity(0.2)
                        : isSelected
                            ? WorkspaceStyle.navigatorSelection(active: windowActive)
                            : (hovering ? SidebarGrid.hoverFill : .clear))
        )
        .padding(.horizontal, SidebarGrid.highlightInset)
        .animation(Motion.snap, value: hovering)
        .contentShape(Rectangle())
        .onTapGesture(perform: onSelect)
        .onHover { hovering = $0 }
        .onDrop(of: [.text], delegate: WorkspaceDropTarget(workspace: workspace))
        .contextMenu {
            if let actions { WorkspaceMenuItems(actions: actions) }
            if let onNewWorktree {
                if actions != nil { Divider() }
                Button("New Worktree…", action: onNewWorktree)
            }
        }
        .help(taskPrefix.isEmpty ? name : "\(name) · tasks are \(taskPrefix)-")
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
        .accessibilityAction(named: "Open", onSelect)
        .accessibilityValue(isOpen ? "Expanded" : "Collapsed")
        .accessibilityAction(named: isOpen ? "Hide Worktrees" : "Show Worktrees") {
            BoardMotion.toggle(reduceMotion: reduceMotion, onToggle)
        }
    }
}

/// An open workspace with no worktrees: one dim line (spec §8), under the
/// workspace's name at column C (drawn at depth 2). New Worktree… is on the
/// workspace row's menu, and the repository header's +.
struct NoWorktreesRow: View {
    static let sentence = "No worktrees yet"
    /// What the line's tooltip adds: where they'll come from.
    static let help = "The orchestrator makes them as it dispatches tasks."

    var body: some View {
        SidebarRow {
            Text(Self.sentence)
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
                .lineLimit(1)
                .help(Self.help)
                .gridMark("noWorktrees", .text)
        }
        .frame(minHeight: ColumnGrid.rowHeight - 2 * SidebarGrid.headerVerticalPadding)
        .padding(.vertical, SidebarGrid.headerVerticalPadding)
    }
}

/// The sidebar's first row: everything waiting on you, from every
/// workspace.
struct NeedsYouRow: View {
    let count: Int
    let isSelected: Bool
    let onSelect: () -> Void

    @State private var hovering = false
    @Environment(\.colorScheme) private var scheme
    @Environment(\.controlActiveState) private var controlActiveState

    var body: some View {
        HStack(spacing: 0) {
            Image(systemName: count > 0 ? "tray.full" : "tray")
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(count > 0 ? GlancePalette.amber(scheme) : .secondary)
                .frame(width: SidebarGrid.glyphColumn, height: 16, alignment: .leading)
                .gridMark("needsYou", .icon)
            Text("Needs You").font(WorkspaceStyle.sidebarPrimary)
                .gridMark("needsYou", .text)
            Spacer(minLength: 6)
            if count > 0 {
                Text("\(count)")
                    .font(.system(size: 11, weight: .semibold))
                    .monospacedDigit()
                    .foregroundStyle(GlancePalette.amber(scheme))
            }
        }
        .frame(minHeight: ColumnGrid.rowHeight - 2 * SidebarGrid.rowVerticalPadding)
        .padding(.vertical, SidebarGrid.rowVerticalPadding)
        .padding(.horizontal, SidebarGrid.rowInset)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(
                    isSelected
                        ? WorkspaceStyle.navigatorSelection(active: controlActiveState == .key)
                        : (hovering ? SidebarGrid.hoverFill : .clear)))
        .padding(.horizontal, SidebarGrid.highlightInset)
        .contentShape(Rectangle())
        .onTapGesture(perform: onSelect)
        .onHover { hovering = $0 }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(count == 1 ? "Needs You, 1 item" : "Needs You, \(count) items")
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
    }
}

/// What a workspace header's menu needs to know and do.
struct WorkspaceHeaderActions {
    /// Whether the sidebar draws this workspace a Board row.
    let hasBoard: Bool
    /// Whether an orchestrator is running, which makes Start into Replace.
    let hasOrchestrator: Bool
    let charter: CharterAccess
    let onShowBoard: () -> Void
    /// Start the orchestrator, or with `replace`, ask to replace the one
    /// running.
    let onStart: (_ harness: OrchestratorHarness, _ replace: Bool) -> Void
    let onShowCharter: (URL) -> Void
    /// Whether answering a decision wakes the agent; nil from a runner that
    /// can't, which draws no switch.
    let wakeOnAnswer: Bool?
    let onSetWakeOnAnswer: (Bool) -> Void
}

/// The header's menu, item by item, in `WorkspaceMenu`'s order.
private struct WorkspaceMenuItems: View {
    let actions: WorkspaceHeaderActions

    var body: some View {
        ForEach(
            WorkspaceMenu.items(
                hasBoard: actions.hasBoard, hasOrchestrator: actions.hasOrchestrator,
                wakeOnAnswer: actions.wakeOnAnswer),
            id: \.self
        ) { item in
            switch item {
            case .showBoard:
                Button(item.title, action: actions.onShowBoard)
            case .startOrchestrator:
                Menu(item.title) {
                    ForEach(OrchestratorHarness.allCases) { harness in
                        Button(harness.title) { actions.onStart(harness, false) }
                    }
                }
            case .replaceOrchestrator:
                // An ellipsis on each harness: choosing one asks first,
                // because the orchestrator running now closes.
                Menu(item.title) {
                    ForEach(OrchestratorHarness.allCases) { harness in
                        Button("\(harness.title)…") { actions.onStart(harness, true) }
                    }
                }
            case .showCharter:
                switch actions.charter {
                case .open(let url):
                    Button(item.title) { actions.onShowCharter(url) }
                case .unavailable(let why):
                    // Disabled with the reason rather than left out: the
                    // item is how anybody learns a charter exists.
                    Button(item.title) {}
                        .disabled(true)
                        .help(why)
                }
            case .wakeOnAnswer:
                Divider()
                Toggle(
                    item.title,
                    isOn: Binding(
                        get: { actions.wakeOnAnswer ?? false }, set: actions.onSetWakeOnAnswer))
                    .help("When you answer one of this board’s decisions, type the answer into the agent working that task, or else the orchestrator, once it’s idle.")
            }
        }
    }
}

/// A repository's worktrees that no workspace owns, below its workspaces.
///
/// Collapsed by default, like `HiddenWorktrees` which it is modeled on: a
/// worktree lands here until something claims it, and most of them are
/// claimed as soon as an agent works in one. Unlike a hidden worktree, one
/// in here is live, so expanding draws the ordinary row — `row` — and not an
/// Unhide button.
struct UnclaimedWorktrees<Row: View>: View {
    let worktrees: [Worktree]
    let isExpanded: Bool
    let onToggle: () -> Void
    @ViewBuilder let row: (Worktree) -> Row

    private var attention: Int {
        worktrees.flatMap(\.terminals).filter(\.status.wantsAttention).count
    }

    /// The same rule the rows inside use, so expanding the group cannot
    /// change what it was already saying.
    private var attentionStatus: Status? {
        Status.mostUrgent(in: worktrees.flatMap(\.terminals).map(\.status))
    }

    var body: some View {
        SidebarGroupSection(
            title: "Unclaimed", id: "group.unclaimed", glyph: "questionmark.folder", count: worktrees.count,
            attention: attentionStatus,
            attentionHelp: attention == 1
                ? "1 waiting on you, in a worktree no workspace owns"
                : "\(attention) waiting on you, in worktrees no workspace owns",
            help: "Worktrees no workspace owns yet",
            isExpanded: isExpanded, onToggle: onToggle
        ) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(worktrees) { worktree in row(worktree) }
            }
        }
        .padding(.top, ColumnGrid.rhythm)
    }
}

/// A group of rows below a repository, Hidden and Unclaimed: the one
/// collapsible section (ov-101), in the sidebar's metrics, with a
/// workspace's columns since these sit among the workspaces (ov-83):
/// chevron at A, glyph at B, title at C, and its count trailing, tertiary
/// (the owner: counts go in the header's trailing slot). The most urgent
/// status inside, when something in the group wants you, is the one thing
/// in color, beside the count.
///
/// It was `SidebarDisclosureHeader`, a header of its own with its own
/// chevron, its own animation and the count beside the title.
struct SidebarGroupSection<Content: View>: View {
    let title: String
    let id: String
    /// What the group is drawn as in column B, as a workspace is.
    let glyph: String
    let count: Int
    let attention: Status?
    let attentionHelp: String
    var help: String?
    let isExpanded: Bool
    let onToggle: () -> Void
    @ViewBuilder let content: () -> Content

    var body: some View {
        CollapsibleSection(
            id: id, metrics: .sidebar,
            isExpanded: Binding(get: { isExpanded }, set: { open in if open != isExpanded { onToggle() } }),
            count: count, accessibilityLabel: title,
            label: { _ in
                HStack(spacing: 0) {
                    Image(systemName: glyph)
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(.tertiary)
                        .frame(width: SidebarGrid.glyphColumn, alignment: .leading)
                        .gridMark("group", .icon)
                        .accessibilityHidden(true)
                    SectionTitle(text: title, style: .group, tone: .quiet, gridRow: "group")
                }
                .help(help ?? "")
            },
            accessory: {
                if let attention {
                    StatusGlyph(status: attention, inAppDiameter: 5)
                        .help(attentionHelp)
                        .padding(.trailing, SidebarGrid.cellGap)
                }
            },
            content: content)
    }
}

extension View {
    /// Indent a sidebar row `depth` columns in: what `ContentView.sidebarRow`
    /// does with `SidebarEntry.depth`, and what `GridGeometryTests` does with
    /// the same depth, so the test measures the arrangement the app draws.
    func sidebarDepth(_ depth: Int) -> some View {
        padding(.leading, SidebarGrid.indent(depth))
    }
}

/// The sidebar's title row: "Fleet" at column A, a spinner while any runner
/// is reading, and `trailing` — the add menu — at the trailing edge.
///
/// Just the word. It used to be the one runner being driven, because the
/// sidebar was that runner's worktrees and the pane could not otherwise say
/// whose. Now it is every runner's at once, and each row already names its own
/// runner below, so a header naming one would be naming the wrong thing, or
/// picking a favorite among rows that are not ranked.
struct SidebarTitleRow<Trailing: View>: View {
    let busy: Bool
    @ViewBuilder let trailing: Trailing

    var body: some View {
        SidebarRow {
            HStack(spacing: SidebarGrid.gap) {
                Text("Fleet")
                    .font(.title3.weight(.semibold))
                    .gridMark("title", .text)
                if busy { ProgressView().controlSize(.mini) }
                Spacer()
                trailing
            }
        }
        .padding(.top, 2 * ColumnGrid.rhythm)
        .padding(.bottom, ColumnGrid.rhythm)
    }
}

/// Search, because worktrees are unbounded: the field's edge at column A.
///
/// It matches terminals too, so typing an agent's name finds the worktree
/// containing it — which is how you reach an agent on another runner without
/// going looking for the runner.
struct SidebarSearchRow: View {
    @Binding var query: String
    var focused: FocusState<Bool>.Binding

    var body: some View {
        SidebarRow {
            HStack(spacing: 6) {
                // The system's own field and focus ring, in place of a plain
                // field drawn into a rounded rectangle with a ring of its own.
                TextField("Find a workspace, task, or agent", text: $query)
                    .textFieldStyle(.roundedBorder)
                    .font(.callout)
                    .focused(focused)
                    .gridMark("search", .text)
                    // Esc clears the search, and on an empty one leaves the
                    // field (checklist F3). The window's Esc monitor passes
                    // Esc to a text field, so this is the one place it's
                    // heard.
                    .onExitCommand {
                        let next = SearchEscape.after(query: query)
                        query = next.query
                        if !next.keepsFocus { focused.wrappedValue = false }
                    }
                if !query.isEmpty {
                    Button { query = "" } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 11))
                            .foregroundStyle(.tertiary)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .padding(.bottom, ColumnGrid.rhythm)
    }
}
