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
struct WorkspaceSection: View {
    let workspace: Workspace
    let isExpanded: Bool
    @Binding var selection: ContentView.Selection?
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
    /// Whether this workspace's runner can be acted on right now.
    ///
    /// Reads stay live even when this is `false` — the row is still
    /// selectable and its terminals still show whatever was last read from
    /// it — but the affordances that would MUTATE something on a runner
    /// already known to be unreachable are dimmed and inert rather than
    /// left to fail silently or hang on a dead connection.
    var usable: Bool = true

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
    /// See `WorkspaceStyle.navigatorSelection(active:)`.
    ///
    /// `.key` and not `!= .inactive`: with two Far Cooler windows open the
    /// second one is `.active` — the app has focus, this window does not — and
    /// that is precisely the case the dimming exists for.
    @Environment(\.controlActiveState) private var controlActiveState
    private var windowActive: Bool { controlActiveState == .key }

    private var isSelected: Bool {
        selection == .workspace(host: workspace.host ?? "", id: workspace.id)
    }

    /// Open whether or not the user opened it.
    private var showsTerminals: Bool { isExpanded || !workspace.attention.isEmpty }

    private func row(_ terminal: Terminal, ordinal: Int?) -> some View {
        let id = ContentView.Selection.terminal(
            host: workspace.host ?? "", workspace: workspace.id, terminal: terminal.id)
        return TerminalRow(
            terminal: terminal,
            isSelected: selection == id,
            onSelect: { selection = id },
            isTiled: tiled.contains(terminal.id),
            layouts: layouts,
            onMoveToLayout: { onMoveToLayout(terminal, $0) },
            onDropTogether: { onDropTogether($0, terminal) },
            onAction: { onTerminalAction(terminal, $0) },
            ordinal: ordinal,
            usable: usable
        )
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header

            if showsTerminals {
                let numbering = workspace.ordinals()
                // Creation order, always. Sorting whatever needs you to the top
                // put the sidebar in motion at the exact moment you were
                // reaching for it: an agent three rows down finishes, every row
                // under it slides, and the click you had already committed to
                // lands on something else. Attention is a mark on a row — the
                // glyph, the count in the status bar, ⌘⇧A — and a mark you can
                // find in a stable list beats one that comes to you by moving
                // the list.
                ForEach(workspace.terminals) { t in
                    row(t, ordinal: numbering[t.id])
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
            Button(action: onToggle) {
                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(.tertiary)
                    .rotationEffect(.degrees(showsTerminals ? 90 : 0))
                    .frame(width: SidebarGrid.gutter, height: 16, alignment: .leading)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            VStack(alignment: .leading, spacing: 1) {
                Text(workspace.task)
                    .font(WorkspaceStyle.sidebarPrimary)
                    .lineLimit(1)

                HStack(spacing: 5) {
                    Text(workspace.isMainCheckout ? "Primary checkout" : workspace.branch)
                        .font(WorkspaceStyle.sidebarMetadata)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)

                    if workspace.worktreeMissing {
                        Text("worktree gone")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(.orange)
                            .help(
                                "This worktree’s directory is gone. If you moved it with git worktree "
                                    + "move, the directory it moved to is a separate workspace now — this "
                                    + "row keeps the terminals and agent transcripts from before the move.")
                    }
                }
            }
            .layoutPriority(1)

            Spacer(minLength: 6)

            if !showsTerminals, let waiting = workspace.attentionStatus {
                // Attention is the only collapsed status worth permanent room.
                // Terminal counts and "no terminals" repeat what expanding the
                // row already says, while this dot is the reason to expand it.
                //
                // Its color comes from the same rule the expanded row's glyph
                // uses. Hardcoded orange here meant a worktree whose one
                // pending item was a failed agent showed orange collapsed and
                // red expanded — the same terminal, two colors, decided by
                // whether a disclosure triangle happened to be open.
                Circle()
                    .fill(waiting.tint)
                    .frame(width: 6, height: 6)
                    .accessibilityLabel(
                        "\(workspace.attention.count) waiting on you in \(workspace.task)")
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
                                + "in \(workspace.task), including work that isn’t committed yet"
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
                    .help("New terminal in \(workspace.task)")
                    .opacity(hovering ? (usable ? 1 : 0.45) : 0)
                    .allowsHitTesting(hovering && usable)

                    Menu {
                        // First, because it is what you do with a worktree you can see
                        // in a list but are not currently looking at — which is the
                        // only thing this menu can do that the title bar cannot.
                        EditorMenuItems(
                            workspace: workspace, onError: onEditorError,
                            showsSettingsItem: false)
                        Divider()
                        Button("New terminal", action: onNewTerminal)
                        Divider()
                        if workspace.isHidden {
                            Button("Unhide", action: onUnhide)
                        } else {
                            Button("Hide", action: onHide)
                        }
                        // Absent, not disabled, for the main checkout. A daemon-side
                        // refusal is a safety net; the button should not be there to
                        // press.
                        if workspace.worktreeMissing && !workspace.isMainCheckout {
                            Button("Dismiss", action: onRemove)
                        } else if !workspace.isMainCheckout {
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
        .padding(.vertical, SidebarGrid.rowVerticalPadding)
        .padding(.horizontal, SidebarGrid.edge - SidebarGrid.highlightInset)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(
                    isSelected
                        ? WorkspaceStyle.navigatorSelection(active: windowActive)
                        : (hovering ? Color.primary.opacity(0.045) : .clear))
        )
        .padding(.horizontal, SidebarGrid.highlightInset)
        .animation(Motion.snap, value: hovering)
        .contentShape(Rectangle())
        .onTapGesture { selection = .workspace(host: workspace.host ?? "", id: workspace.id) }
        .onHover { hovering = $0 }
        // On the ROW, not on the counts, and that is the whole of it.
        //
        // The counts fade to zero the instant a pointer arrives — the buttons
        // above them take that space — so a `.help` on the counts could only
        // ever describe a number that had already gone invisible, and it would
        // be unreachable by construction rather than by bad luck. The row is
        // the one thing under the pointer at the moment a tooltip could fire,
        // so it carries the explanation and says the two numbers again, which
        // is also how the reader gets them back after hovering hid them.
        .help(ifAny: changesHelp)
    }

    /// What the trailing `+N -M` counts, in words, with the numbers repeated.
    ///
    /// The sidebar's number and the Changes pane's Branch header answer
    /// different questions and are not being made to agree: this one is "is
    /// this worth opening at all", so it spans everything in the workspace,
    /// while Branch is "what lands when this merges" and counts commits only.
    /// See `ChangesStore.files` for where the two diverge on the wire.
    ///
    /// Nil rather than an empty string when there is nothing to say: an empty
    /// tooltip is not the same thing as no tooltip, and every clean workspace in
    /// the fleet would be tracking one.
    private var changesHelp: String? {
        guard let changes, changes.hasDiff else { return nil }
        let counts = DiffCounts.pair(
            insertions: changes.insertions, deletions: changes.deletions)
        // Said only in the gray state, which is the one that needs explaining.
        // A worktree nobody has marked read is every worktree until somebody
        // starts marking them, and a sentence on every row in the fleet saying
        // so would be the tooltip telling you the default.
        let read =
            changes.changedSinceReviewed
            ? ""
            : " You’ve marked this reviewed, and nothing has changed since."
        return "\(counts) in this "
            + "workspace — committed work, uncommitted edits, and untracked files, together. "
            + "The Changes pane splits them: Branch is what’s committed, Uncommitted is what isn’t."
            + read
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

    var body: some View {
        // One band, one indent level for the title's gutter, and a `+` that is
        // a Button rather than a Menu — so it sits in exactly the same column
        // as the sidebar header's, which no amount of padding on a `Menu` could
        // achieve.
        SidebarRow {
            HStack(spacing: 0) {
                // One semantic gutter, never an icon followed by a disclosure
                // column. At rest it says what the row is; on hover a
                // collapsible row becomes its control. A host with nothing to
                // disclose keeps the machine icon rather than advertising a
                // caret that cannot do anything.
                if onToggleCollapse != nil {
                    ZStack(alignment: .leading) {
                        Image(systemName: "folder")
                            .font(.system(size: 10, weight: .medium))
                            .opacity(hovering ? 0 : 1)

                        Image(systemName: "chevron.right")
                            .font(.system(size: 9, weight: .semibold))
                            .rotationEffect(.degrees(isCollapsed ? 0 : 90))
                            .opacity(hovering ? 1 : 0)
                    }
                    .foregroundStyle(.tertiary)
                    .frame(width: SidebarGrid.gutter, alignment: .leading)
                } else {
                    Image(systemName: "desktopcomputer")
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(.tertiary)
                        .frame(width: SidebarGrid.gutter, alignment: .leading)
                }

                HStack(spacing: 6) {
                    Text(name)
                        .font(WorkspaceStyle.sectionTitle)
                        .foregroundStyle(.secondary)
                    if showHost {
                        Text(host.isEmpty ? "this Mac" : host)
                            .font(.system(size: 10.5))
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                    }
                    HostDot(state: hostState, onReconnect: onReconnect, update: daemonUpdate)
                    Spacer()

                    if onNewWorktree != nil || onNewTerminal != nil {
                        SidebarMenuButton(
                            systemImage: "plus",
                            help: "Add to \(name)",
                            items: [
                                onNewWorktree.map {
                                    SidebarMenuItem(title: "New Workspace in \(name)…", action: $0)
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
                        // action one row below "New Workspace" is a misclick away
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
        .padding(.top, SidebarGrid.projectTopPadding)
        .padding(.bottom, SidebarGrid.projectBottomPadding)
        .contentShape(Rectangle())
        // The whole row toggles, not just the chevron: a section label is a big
        // easy target and a 9-point glyph is not. The `+` and `…` are real
        // Buttons, which take their own clicks ahead of this.
        .onTapGesture { onToggleCollapse?() }
        .onHover { hovering = $0 }
        // The one hover in the sidebar that changes SHAPE and not just alpha:
        // the gutter swaps a 10pt folder for a 9pt chevron, and the `+` and `…`
        // arrive at the same instant. Cut hard, that is three things appearing
        // out of nothing under a pointer that only grazed the row. `Motion.snap`
        // is the app's own "instant, but not a cut" — 0.22s — and it was
        // already in four other files while this file, the one with the most
        // hover states in the app, had no animation modifier at all.
        .animation(Motion.snap, value: hovering)
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
/// waiting. `Status.tint` already paints `working` and `starting` `.secondary`
/// for that reason; a connection in progress is the fleet-level version of the
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
            .help("Far Cooler is not installed on this runner — open Settings ▸ Runners")
        }
    }
}

/// One terminal row.
///
/// A single line, deliberately. Both levels used to be two-line blocks, which
/// gave the list no rhythm: a workspace and the terminal under it were the same
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
    /// `WorkspaceSection.usable`, which this mirrors row by row.
    var usable: Bool = true

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

    /// The gap between the status column and the text beside it.
    ///
    /// Named rather than written twice: the feed's lines below have to begin
    /// in the same column the terminal's name does, and two hand-written 7s is
    /// exactly how a column goes out of alignment — see `SidebarGrid`'s own
    /// note on the four insets that used to be chosen locally.
    private static let markerGap: CGFloat = 7

    var body: some View {
        // A column, not a row, since this task: the status line reads across
        // and the feed reads down under it. Everything that positions the row
        // — padding, highlight, drop target — stays on the outside of this, so
        // a row with three steps is one taller row rather than four rows.
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: Self.markerGap) {
                StatusGlyph(status: status)

                Text(terminal.label)
                    .font(.system(size: 12.5))
                    .lineLimit(1)
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
                    .padding(.leading, StatusGlyph.inline + Self.markerGap)
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
                    }
                }
                // Aligned under the terminal's name, not under its status dot,
                // so the steps read as belonging to the row rather than as a
                // second column of their own.
                .padding(.leading, StatusGlyph.inline + Self.markerGap)
                // Take the width offered and no more. Without this the widest
                // step would set the row's ideal width and the sidebar would
                // report wanting to be wider than the name ever needed.
                .frame(maxWidth: .infinity, alignment: .leading)
            }

            // The agents this one has running, one line each.
            //
            // Named rather than counted here, because a sidebar has the room:
            // the COUNT is already on the signal line above, for the surfaces
            // that do not. Indented one step further than the transcript, so
            // the branch mark reads as work happening underneath this row
            // rather than as another thing the row itself said.
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
                    }
                }
                .padding(.leading, StatusGlyph.inline + Self.markerGap * 2)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(.vertical, SidebarGrid.rowVerticalPadding)
        // A terminal is one level in from its worktree; the highlight sits
        // inside the band exactly as the worktree row's does.
        .padding(.leading, SidebarGrid.edge - SidebarGrid.highlightInset + SidebarGrid.gutter)
        .padding(.trailing, SidebarGrid.edge - SidebarGrid.highlightInset)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(
                    isSelected
                        ? WorkspaceStyle.navigatorSelection(active: windowActive)
                        : (hovering ? Color.primary.opacity(0.045) : .clear))
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
            Button("Move to its own layout") { onMoveToLayout(nil) }
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

/// A workspace's own status, for its heading.
///
/// Workspaces have three states worth showing and no agent of their own, so
/// this stays a small dot: it is context for the heading rather than something
/// to scan, and the terminals underneath carry the detail.
struct WorkspaceDot: View {
    let state: String

    private var color: Color {
        switch StateKind.parse(state) {
        case .active: return .green
        case .error: return .red
        case .hidden: return Color.secondary.opacity(0.4)
        // Red, not a dimmed amber. `StatusGlyph` spends amber on one state —
        // an agent is waiting on you — and a directory that is gone is not
        // waiting for anything. It is the workspace-level `Status.lost`, and
        // `lost` is red. The soft orange was a third reading from before there
        // was a rule: less alarming than `error`, warmer than `hidden`.
        //
        // It shares red with `error` now, which is affordable HERE and would
        // not be in a list: this dot appears once, beside a 24pt title, never
        // next to another one, and its help text names the state.
        case .worktreeMissing: return .red
        default: return .secondary
        }
    }

    var body: some View {
        Circle().fill(color).frame(width: 8, height: 8).help(state)
    }
}

/// Workspace detail.
///
/// Used to open with a State / Terminals / Worktree key-value table, which read
/// like a database inspector and gave the most screen to a filesystem path
/// nobody needs. The terminals ARE the workspace, so they lead; the path is a
/// footnote you can copy when you want it.
struct WorkspaceDetail: View {
    let workspace: Workspace
    let onNewTerminal: () -> Void
    let onHide: () -> Void
    let onUnhide: () -> Void
    let onRemove: () -> Void
    let onOpenTerminal: (Terminal) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                header

                if workspace.terminals.isEmpty {
                    empty
                } else {
                    VStack(spacing: 8) {
                        // Creation order, for the reason `WorkspaceSection`
                        // gives: cards that rearrange themselves under the
                        // pointer are worse than cards you have to read.
                        ForEach(workspace.terminals) { t in
                            terminalCard(t)
                        }
                    }
                }

                footnote
            }
            .padding(32)
            .frame(maxWidth: 720, alignment: .leading)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        // Same title as the tiled and solo views, so switching between them does
        // not change what the window claims you are looking at.
        .navigationTitle(workspace.windowTitle)
        .navigationSubtitle(workspace.windowSubtitle)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 10) {
                    WorkspaceDot(state: workspace.state)
                    Text(workspace.task).font(.system(size: 24, weight: .semibold))
                }
                Text(workspace.branch)
                    .font(.system(size: 13, design: .monospaced))
                    .foregroundStyle(.secondary)
            }

            HStack(spacing: 8) {
                Button(action: onNewTerminal) {
                    Label("New terminal", systemImage: "plus")
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut("t", modifiers: .command)

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
                    if workspace.isHidden {
                        Button("Unhide", action: onUnhide)
                    } else {
                        Button("Hide", action: onHide)
                    }
                    // Absent, not disabled, for the main checkout. A
                    // daemon-side refusal is a safety net; the button should
                    // not be there to press.
                    if !workspace.isMainCheckout {
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

    private var empty: some View {
        VStack(spacing: 8) {
            Image(systemName: "terminal")
                .font(.system(size: 28))
                .foregroundStyle(.tertiary)
            Text("No terminals").font(.callout.weight(.medium))
            Text("A terminal runs one agent, or one shell, inside this workspace.")
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
            .padding(.horizontal, 14)
            .background(
                RoundedRectangle(cornerRadius: 10)
                    .fill(Color.primary.opacity(0.04))
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    /// The path, demoted to where it belongs.
    private var footnote: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "folder")
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
            Text(workspace.worktree)
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
    let worktrees: [Workspace]
    let isExpanded: Bool
    let onToggle: () -> Void
    let onUnhide: (Workspace) -> Void

    private var attention: Int {
        worktrees.flatMap(\.terminals).filter(\.status.wantsAttention).count
    }

    /// The color for that count — the same rule the rows inside use, so
    /// expanding the section cannot change what it was already saying.
    private var attentionStatus: Status? {
        Status.mostUrgent(in: worktrees.flatMap(\.terminals).map(\.status))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SidebarRow(indent: 0) {
                Button(action: onToggle) {
                    HStack(spacing: SidebarGrid.gap) {
                        Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
                            .font(.system(size: 9, weight: .semibold))
                            .foregroundStyle(.tertiary)
                            .frame(width: SidebarGrid.gutter - SidebarGrid.gap, alignment: .leading)
                        Text("Hidden")
                            .font(.system(size: 11, weight: .medium))
                            .foregroundStyle(.secondary)
                        Text("\(worktrees.count)")
                            .font(.system(size: 11))
                            .foregroundStyle(.tertiary)
                        if let waiting = attentionStatus {
                            Circle()
                                .fill(waiting.tint)
                                .frame(width: 5, height: 5)
                                .help("\(attention) waiting on you, inside a hidden workspace")
                        }
                        Spacer(minLength: 0)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            .padding(.vertical, 3)

            if isExpanded {
                ForEach(worktrees) { ws in
                    SidebarRow(indent: 1) {
                        HStack(spacing: SidebarGrid.gap) {
                            Text(ws.task)
                                .font(.system(size: 12))
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
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
                    .padding(.vertical, 2)
                }
            }
        }
    }
}
