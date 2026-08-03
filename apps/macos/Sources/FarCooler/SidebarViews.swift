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
    let onArchive: () -> Void
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

    @State private var hovering = false

    private var isSelected: Bool { selection == .workspace(workspace.id) }

    /// Open whether or not the user opened it.
    private var showsTerminals: Bool { isExpanded || !workspace.attention.isEmpty }

    private func row(_ terminal: Terminal, ordinal: Int?) -> some View {
        let id = ContentView.Selection.terminal(workspace: workspace.id, terminal: terminal.id)
        return TerminalRow(
            terminal: terminal,
            isSelected: selection == id,
            onSelect: { selection = id },
            isTiled: tiled.contains(terminal.id),
            layouts: layouts,
            onMoveToLayout: { onMoveToLayout(terminal, $0) },
            onDropTogether: { onDropTogether($0, terminal) },
            onAction: { onTerminalAction(terminal, $0) },
            ordinal: ordinal
        )
    }

    /// Numbers for terminals a worktree has more than one of, by command.
    ///
    /// Computed over creation order rather than display order, so a row does not
    /// renumber itself when something starts needing attention and sorts up.
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

    private var ordered: [Terminal] {
        workspace.terminals.sorted { a, b in
            a.status.wantsAttention && !b.status.wantsAttention
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header

            if showsTerminals {
                let numbering = ordinals
                ForEach(ordered) { t in
                    row(t, ordinal: numbering[t.id])
                }

                Button(action: onNewTerminal) {
                    HStack(spacing: Grid.gap) {
                        Image(systemName: "plus")
                            .font(.system(size: 9, weight: .semibold))
                            .frame(width: Grid.marker)
                        Text("New terminal").font(.system(size: 12))
                    }
                    .foregroundStyle(.tertiary)
                    .padding(.vertical, 4)
                    .padding(.leading, Grid.child)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .padding(.bottom, 4)
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
                .foregroundStyle(.tertiary)
                .lineLimit(1)
                .padding(.leading, 7)
                .layoutPriority(-1)

            Spacer(minLength: 6)

            if !showsTerminals {
                // What a closed worktree still has to answer: is anything
                // happening in here?
                Text(workspace.summary)
                    .font(.system(size: 11))
                    .foregroundStyle(workspace.attention.isEmpty ? .tertiary : .secondary)
                    .lineLimit(1)
                    .layoutPriority(1)

                if !workspace.attention.isEmpty {
                    Circle().fill(Color.orange).frame(width: 6, height: 6)
                        .padding(.leading, 5)
                }
            }

            Menu {
                Button("New terminal", action: onNewTerminal)
                if !workspace.isMainCheckout {
                    Divider()
                    Button("Archive", action: onArchive)
                    Button("Remove worktree…", role: .destructive, action: onRemove)
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
            .opacity(hovering ? 1 : 0)
            .padding(.leading, 2)
        }
        .padding(.vertical, 4)
        .padding(.horizontal, Grid.margin)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(isSelected ? Color.primary.opacity(0.08) : (hovering ? Color.primary.opacity(0.04) : .clear))
        )
        .contentShape(Rectangle())
        .onTapGesture { selection = .workspace(workspace.id) }
        .onHover { hovering = $0 }
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

    @State private var hovering = false

    var body: some View {
        HStack(spacing: 6) {
            Text(name.uppercased())
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(.tertiary)
                .tracking(0.6)
            Text("\(count)")
                .font(.system(size: 10))
                .foregroundStyle(.quaternary)
                .monospacedDigit()
            Spacer()

            if onNewWorktree != nil || onNewTerminal != nil {
                Menu {
                    if let onNewWorktree {
                        Button("New worktree in \(name)…", action: onNewWorktree)
                    }
                    if let onNewTerminal {
                        // The main checkout is a place people work — a quick
                        // build, a look at main while a worktree is mid-review —
                        // and it was the one directory this app could not open a
                        // terminal in.
                        Button("New terminal in \(name)", action: onNewTerminal)
                    }
                } label: {
                    Image(systemName: "plus")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(.tertiary)
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                // Shown on hover, like every other per-row control in a
                // sidebar. A `+` on every project header at all times is a
                // column of plus signs down a list that is meant to read as
                // quiet section labels.
                .opacity(hovering ? 1 : 0)
                .padding(.trailing, -Grid.menuChrome)
                .help("Add to \(name)")
            }
        }
        // The title on the rail it shares with every workspace name.
        //
        // The `+` is trickier and the number is measured rather than derived.
        // This row lives inside a `ScrollView`'s content, the sidebar header's
        // `+` does not, and the difference between them is not the 8pt inset
        // that content carries — it is consistently 9pt more than any padding
        // arithmetic here predicts. Three attempts to derive it landed the
        // button visibly left of the one it has to line up with; this is what
        // actually puts them in the same column. Verified by screenshot, which
        // is the only thing that settles it.
        .padding(.leading, Grid.margin + Grid.chevron)
        .padding(.trailing, Grid.edge - 8 - 9)
        .padding(.top, 14)
        .padding(.bottom, 3)
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
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
    /// the old behaviour and it labelled a lone `claude` as "Terminal 7", which
    /// answers a question nobody asked.
    var ordinal: Int?

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
            }
        }
        .padding(.vertical, 4)
        .padding(.leading, Grid.child)
        .padding(.trailing, Grid.margin)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(isSelected ? Color.primary.opacity(0.08) : (hovering ? Color.primary.opacity(0.04) : .clear))
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .strokeBorder(Color.accentColor, lineWidth: targeted ? 2 : 0))
        )
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
            MainActor.assumeIsolated { PaneDrag.shared.begin(terminal.id) }
            return NSItemProvider(object: terminal.id as NSString)
        }
        .onDrop(of: [.text], isTargeted: $targeted) { _ in
            guard let dragged = PaneDrag.shared.terminal, dragged != terminal.id else {
                return false
            }
            PaneDrag.shared.end()
            onDropTogether(dragged)
            return true
        }
        .contextMenu {
            Button("Move to its own layout") { onMoveToLayout(nil) }
            if !layouts.isEmpty {
                Divider()
                ForEach(Array(layouts.enumerated()), id: \.element.id) { index, group in
                    Button(layoutLabel(group, position: index + 1)) {
                        onMoveToLayout(group)
                    }
                    .disabled(group.terminals.contains(terminal.id))
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

    private var colour: Color {
        switch StateKind.parse(state) {
        case .active: return .green
        case .error: return .red
        case .archived: return Color.secondary.opacity(0.4)
        default: return .secondary
        }
    }

    var body: some View {
        Circle().fill(colour).frame(width: 8, height: 8).help(state)
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
    let onArchive: () -> Void
    let onRemove: () -> Void
    let onOpenTerminal: (Terminal) -> Void

    /// Numbers for terminals a worktree has more than one of, by command.
    ///
    /// Computed over creation order rather than display order, so a row does not
    /// renumber itself when something starts needing attention and sorts up.
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

    private var ordered: [Terminal] {
        workspace.terminals.sorted { a, b in
            a.status.wantsAttention && !b.status.wantsAttention
        }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                header

                if workspace.terminals.isEmpty {
                    empty
                } else {
                    VStack(spacing: 8) {
                        ForEach(ordered) { t in
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
                    if workspace.isMainCheckout {
                        // Nothing destructive for the repository's own checkout
                        // — see `Workspace.isMainCheckout`.
                        Text("The repository's own checkout")
                    } else {
                        Button("Archive", action: onArchive)
                        Button("Remove worktree…", role: .destructive, action: onRemove)
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
