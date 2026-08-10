import SwiftUI
import UniformTypeIdentifiers

/// The sidebar's layout grid.
///
/// Written down because it was not before, and the result was every measurement
/// chosen locally: a workspace title at one indent, its terminals at another,
/// and the "new terminal" affordance at a third that sat to the LEFT of the
/// children it belonged to. Three columns pretending to be a hierarchy.
///
/// Everything below positions against these and nothing invents its own.
enum Grid {
    /// Every row's own horizontal padding, inside its highlight.
    ///
    /// Small on purpose. It used to be wide enough that a selected row's
    /// highlight began well to the LEFT of its parent's disclosure chevron, so
    /// children appeared to hang outside the workspace containing them. That is
    /// what made the list feel off balance: the indent said one thing and the
    /// highlight said the opposite.
    /// The sidebar's outer edge.
    ///
    /// The search field, the header above it and every `+` in the column share
    /// this, because they are the sidebar's chrome — the list's own titles sit
    /// deeper, on `rail`. Three hand-tuned constants used to do this job and
    /// they had drifted to three different edges.
    static let edge: CGFloat = 14

    /// What `.menuStyle(.borderlessButton)` insets its own label by.
    ///
    /// Invisible, unavoidable, and only partly correctable. A trailing `+`
    /// lands on `edge` once this is cancelled. A LEADING label does not: the
    /// control clamps how far a negative pad may move it, so the machine picker
    /// still sits about 9pt inside the search field below it. Closing that last
    /// gap means not using `Menu` for the picker at all — a plain `Button`
    /// presenting an `NSMenu` — which is a deliberate change, not a padding
    /// tweak, and is why this constant stops where it does.
    static let menuChromeLeading: CGFloat = 9
    static let menuChrome: CGFloat = 5

    static let margin: CGFloat = 6

    /// The disclosure chevron's column — a gutter, like every outline view.
    static let chevron: CGFloat = 18

    /// The rail everything titled aligns to: workspace names, and the section
    /// header above them.
    static var rail: CGFloat { margin + chevron }

    /// One indent step, from the rail to a child's TEXT.
    ///
    /// The step used to be applied to the child's leading edge and then the
    /// marker and its gap were added on top, so a terminal's name sat 32pt right
    /// of its worktree's — two indents for one level of nesting, which is what
    /// made the hierarchy look wrong. The marker now lives INSIDE the step, the
    /// way a disclosure triangle lives inside its own.
    static let indent: CGFloat = 16
    /// A child row's leading edge: the same rail its parent's title sits on.
    static var child: CGFloat { rail }

    /// The marker column, reserved whether or not a row has anything to show,
    /// so titles align down the list.
    static let marker: CGFloat = 8
    static let gap: CGFloat = 8
    static var childText: CGFloat { child + marker + gap }

    /// Space between one workspace and the next.
    static let group: CGFloat = 12

    /// The gap between two trailing cells.
    static let cell: CGFloat = 6
}

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
            // Built the same way the row builds it, thousands separators and
            // all, so the measurement is of the string that gets drawn.
            let text =
                "+\(row.insertions.formatted()) -\(row.deletions.formatted())"
            let w = (text as NSString).size(withAttributes: [.font: font]).width
            widest = max(widest, w)
        }
        // Nothing in the fleet has a diff: no column, no cost.
        return widest == 0 ? 0 : ceil(widest) + 2
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
    /// Whether this workspace's machine can be acted on right now.
    ///
    /// Reads stay live even when this is `false` — the row is still
    /// selectable and its terminals still show whatever was last read from
    /// it — but the affordances that would MUTATE something on a machine
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

    /// Numbers for terminals a worktree has more than one of, by command.
    private var ordinals: [String: Int] {
        var counts: [String: Int] = [:]
        for terminal in workspace.terminals {
            counts[terminal.label, default: 0] += 1
        }
        var seen: [String: Int] = [:]
        var out: [String: Int] = [:]
        for terminal in workspace.terminals where counts[terminal.label, default: 0] > 1 {
            let next = (seen[terminal.label] ?? 0) + 1
            seen[terminal.label] = next
            out[terminal.id] = next
        }
        return out
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header

            if showsTerminals {
                let numbering = ordinals
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

                Button(action: onNewTerminal) {
                    SidebarRow(indent: 1) {
                        HStack(spacing: SidebarGrid.gap) {
                            Image(systemName: "plus")
                                .font(.system(size: 9, weight: .semibold))
                                .frame(width: SidebarGrid.marker)
                            Text("New terminal").font(.system(size: 12))
                        }
                        .foregroundStyle(.secondary)
                        .padding(.vertical, SidebarGrid.rowVerticalPadding)
                        .contentShape(Rectangle())
                    }
                }
                .buttonStyle(.plain)
                .padding(.bottom, 4)
                // Making a terminal is a mutation same as any other, and one
                // that would simply hang against a machine already known to
                // be gone rather than fail fast the way `act(on:)` fails
                // everything else.
                .opacity(usable ? 1 : 0.55)
                .allowsHitTesting(usable)
            }
        }
    }

    /// One line. Name, branch, and — only when closed — what is inside.
    private var header: some View {
        HStack(spacing: 0) {
            Button(action: onToggle) {
                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(.tertiary)
                    .rotationEffect(.degrees(showsTerminals ? 90 : 0))
                    .frame(width: Grid.chevron, height: 16, alignment: .leading)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            Text(workspace.task)
                .font(.system(size: 13, weight: .medium))
                .lineLimit(1)

            Text(workspace.branch)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .padding(.leading, 7)
                .layoutPriority(-1)

            if workspace.worktreeMissing {
                // Said plainly, because every terminal in it is dead and
                // the reason is not something the user can work out from
                // a color. The row survives at all because it holds
                // terminals worth keeping — an empty one is deleted by
                // the daemon without asking.
                Text("worktree gone")
                    .font(.system(size: 10))
                    .foregroundStyle(.orange)
                    .padding(.leading, 7)
            }

            Spacer(minLength: 6)

            if !showsTerminals && !workspace.attention.isEmpty {
                // Attention is the only collapsed status worth permanent room.
                // Terminal counts and "no terminals" repeat what expanding the
                // row already says, while this dot is the reason to expand it.
                Circle()
                    .fill(Color.orange)
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
            if let changes, changes.hasDiff {
                // One attributed Text means one baseline. Two sibling Text
                // views can still land on adjacent device pixels after the
                // stack reconciles their independently rounded font metrics.
                changeCountsText(changes)
                .font(.system(size: 10, design: .monospaced))
                .lineLimit(1)
                .fixedSize()
                .frame(width: countsWidth, alignment: .trailing)
                .padding(.leading, Grid.cell)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(
                    "\(changes.insertions) insertions, \(changes.deletions) deletions")
            }

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
                    // A worktree whose directory is already gone needs no
                    // confirmation: `removal_needs_confirmation` (Task 8)
                    // answers false once the path is not a directory, so this
                    // routes to the same removal path as everything else.
                    // Excluded for the main checkout the same way "Remove
                    // Worktree…" below is: `remove_worktree` refuses it
                    // outright, so the button should not be offered at all.
                    Button("Dismiss", action: onRemove)
                } else if !workspace.isMainCheckout {
                    Button("Remove Worktree…", role: .destructive, action: onRemove)
                }
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 11, weight: .semibold))
                    .frame(width: 18, height: 16)
                    .contentShape(Rectangle())
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            // Hover already gates visibility; an unusable machine's menu
            // stays reachable by hover but reads as dimmed and does nothing
            // — every item in it is a mutation (hide, remove, a new
            // terminal, opening an editor on the worktree), and every one
            // of those is exactly what `usable` says this machine cannot
            // take right now.
            .opacity(hovering ? (usable ? 1 : 0.55) : 0)
            .allowsHitTesting(usable)
            .padding(.leading, 2)
        }
        .padding(.vertical, SidebarGrid.rowVerticalPadding)
        .padding(.horizontal, SidebarGrid.edge - SidebarGrid.highlightInset)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(isSelected ? Color.primary.opacity(0.08) : (hovering ? Color.primary.opacity(0.04) : .clear))
        )
        .padding(.horizontal, SidebarGrid.highlightInset)
        .contentShape(Rectangle())
        .onTapGesture { selection = .workspace(host: workspace.host ?? "", id: workspace.id) }
        .onHover { hovering = $0 }
    }

    private func changeCountsText(_ changes: InboxRow) -> Text {
        var additions = AttributedString("+\(changes.insertions.formatted())")
        additions.foregroundColor = .green
        var deletions = AttributedString(" -\(changes.deletions.formatted())")
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
    /// host's placeholder header — it names a machine, not a repository,
    /// and there is nothing there to remove.
    var onRemove: (() -> Void)?
    /// Which machine this project's worktrees are on.
    var host: String = ""
    var hostState: HostState = .connected
    /// Whether to name the machine at all — noise on a fleet of one.
    var showHost: Bool = false
    var onReconnect: () -> Void = {}
    /// Whether this project's worktrees are hidden right now.
    var isCollapsed: Bool = false
    /// `nil` for a silent host's placeholder header, which names a machine and
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
                // The chevron goes in `SidebarGrid.gutter` — the column this
                // header already padded by and left empty — so collapsing costs
                // no horizontal space and lands in the same column as every
                // other disclosure in the sidebar. A `Spacer` of the same width
                // holds the column when there is nothing to collapse, so a
                // silent host's header does not sit one indent to the left of
                // every real one.
                if onToggleCollapse != nil {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .semibold))
                        .rotationEffect(.degrees(isCollapsed ? 0 : 90))
                        .foregroundStyle(.tertiary)
                        .frame(width: SidebarGrid.gutter, alignment: .leading)
                } else {
                    Spacer().frame(width: SidebarGrid.gutter)
                }

                HStack(spacing: 6) {
                    Text(name.uppercased())
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .tracking(0.6)
                    Text("\(count)")
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                        .monospacedDigit()
                    if showHost {
                        Text(host.isEmpty ? "this Mac" : host)
                            .font(.system(size: 10))
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                    }
                    HostDot(state: hostState, onReconnect: onReconnect)
                    Spacer()

                    if onNewWorktree != nil || onNewTerminal != nil {
                        SidebarMenuButton(
                            systemImage: "plus",
                            help: "Add to \(name)",
                            items: [
                                onNewWorktree.map {
                                    SidebarMenuItem(title: "New worktree in \(name)…", action: $0)
                                },
                                // The main checkout is a place people work — a quick
                                // build, a look at main while a worktree is
                                // mid-changes — and it was the one directory this app
                                // could not open a terminal in.
                                onNewTerminal.map {
                                    SidebarMenuItem(title: "New terminal in \(name)", action: $0)
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
                        // action one row below "New worktree" is a misclick away
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
    }
}

/// A machine's connection, said as quietly as possible.
///
/// Absent when healthy: a dot that is always there is a dot nobody reads, and
/// the whole point is that you notice it only when something is wrong.
/// Reconnection is amber and silent; only a machine that has given up is red,
/// and clicking it retries at once rather than waiting out the backoff.
struct HostDot: View {
    let state: HostState
    let onReconnect: () -> Void

    var body: some View {
        switch state {
        case .connected:
            EmptyView()
        case .connecting, .reconnecting:
            Circle()
                .fill(Color.orange)
                .frame(width: 5, height: 5)
                .help("Reconnecting to this machine")
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
            .help("Far Cooler is not installed on this machine — open Settings ▸ Machines")
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

    private var status: Status { terminal.status }

    /// Which of several identical ones this is, or nothing.
    ///
    /// Two shells in a worktree are genuinely alike, so they get `2` and `3` —
    /// but only when there is something to tell apart. Numbering everything was
    /// the old behavior and it labeled a lone `claude` as "Terminal 7", which
    /// answers a question nobody asked.
    var ordinal: Int?
    /// Whether this terminal's machine can be acted on right now. See
    /// `WorkspaceSection.usable`, which this mirrors row by row.
    var usable: Bool = true

    /// The status, and only when it is not the boring case.
    private var meta: String? {
        guard status.wantsAttention || status == .working else { return nil }
        return terminal.statusDuration.map { "\(status.label) \($0)" } ?? status.label
    }

    var body: some View {
        HStack(spacing: Grid.gap) {
            StatusGlyph(status: status, size: Grid.marker)

            Text(terminal.label)
                .font(.system(size: 13))
                .lineLimit(1)
                // The name yields before the status does. Which terminal it is
                // matters less than what it wants, and the sidebar is narrow.
                .layoutPriority(0)

            if let ordinal {
                Text("\(ordinal)")
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.tertiary)
                    .layoutPriority(1)
            }

            if let meta {
                Text(meta)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .layoutPriority(1)
            }

            Spacer(minLength: 0)

            // One small mark for "this one is on screen right now", so the
            // fourth agent running in the background is visibly the odd one out
            // rather than indistinguishable from the three you arranged.
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
        .padding(.vertical, SidebarGrid.rowVerticalPadding)
        // A terminal is one level in from its worktree; the highlight sits
        // inside the band exactly as the worktree row's does.
        .padding(.leading, SidebarGrid.edge - SidebarGrid.highlightInset + SidebarGrid.gutter)
        .padding(.trailing, SidebarGrid.edge - SidebarGrid.highlightInset)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(isSelected ? Color.primary.opacity(0.08) : (hovering ? Color.primary.opacity(0.04) : .clear))
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .strokeBorder(Color.accentColor, lineWidth: targeted ? 2 : 0))
        )
        .padding(.horizontal, SidebarGrid.highlightInset)
        .contentShape(Rectangle())
        .onTapGesture(perform: onSelect)
        .onHover { hovering = $0 }
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
            // drags even when this machine cannot itself be rearranged.
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
        case .worktreeMissing: return Color.orange.opacity(0.7)
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

    /// Numbers for terminals a worktree has more than one of, by command.
    private var ordinals: [String: Int] {
        var counts: [String: Int] = [:]
        for terminal in workspace.terminals {
            counts[terminal.label, default: 0] += 1
        }
        var seen: [String: Int] = [:]
        var out: [String: Int] = [:]
        for terminal in workspace.terminals where counts[terminal.label, default: 0] > 1 {
            let next = (seen[terminal.label] ?? 0) + 1
            seen[terminal.label] = next
            out[terminal.id] = next
        }
        return out
    }

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
                StatusGlyph(status: t.status, size: 10)

                VStack(alignment: .leading, spacing: 2) {
                    Text(t.label).font(.system(size: 14, weight: .medium))
                    Text(t.preset).font(.system(size: 12)).foregroundStyle(.secondary)
                }

                Spacer()

                Text(t.statusDuration.map { "\(t.status.label) \($0)" } ?? t.status.label)
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)

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
                        if attention > 0 {
                            Circle()
                                .fill(Color.orange)
                                .frame(width: 5, height: 5)
                                .help("\(attention) waiting on you, inside a hidden worktree")
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
