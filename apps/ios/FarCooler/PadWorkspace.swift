import SwiftUI

// The iPad's workspace in columns (ov-348; design §4 of
// .claude/agent/reports/phones-tree/design.md): the Mac's layout.
//
//   Tree │ Plan │ Chat
//
// - Tree: the One tree as an outline, the Mac's sidebar: the same
//   `OneTree.rows`, each row with children opened and closed in place.
// - Plan: the canvas's home, the same sections the phone's Plan sheet draws
//   (`PlanHomeList`). A theme, lane, page or task picked in the tree shows
//   here, and so does the Board, the toolbar's item.
// - Chat: the orchestrator's pane, always shown.
//
// Narrower (an 11-inch iPad in portrait), the plan and the chat stay and the
// tree is a sidebar shown on demand. At compact width the workspace is the
// phone's. The rules are AgentKit's (`PadLayout`, `PadPick`);
// `WorkspaceScreen` lays the columns out, so the chat stays one pane, never
// built again, whichever layout it's in.

extension EnvironmentValues {
    /// Show a route in the iPad's plan column rather than on the stack:
    /// true when the column took it. Nil outside the column.
    @Entry var canvasOpen: ((PhoneRoute) -> Bool)? = nil
}

// MARK: - The tree

/// The tree as an outline: Needs You and Plan, the themes and their cards,
/// lanes and terminals, then the main checkout and the loose worktrees.
struct PadTreeColumn: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    let place: PhoneWorkspace
    /// What the plan column shows, to mark its row.
    let canvas: PadCanvas
    let onPick: (PadPick) -> Void

    @ObservedObject private var reads: PlanReads
    @ObservedObject private var pageReads: PageReads
    @State private var filter: OneTreeFilter
    @State private var expansion: OneTreeExpansion
    @State private var composing = false
    @State private var fromBranch = false
    @State private var moveFailure: ActionFailure?

    init(
        connection: Connection, summary: WorkspaceSummary, place: PhoneWorkspace, canvas: PadCanvas,
        onPick: @escaping (PadPick) -> Void
    ) {
        self.connection = connection
        self.summary = summary
        self.place = place
        self.canvas = canvas
        self.onPick = onPick
        reads = connection.plans
        pageReads = connection.pages
        _filter = State(initialValue: PhoneTreeFilter.remembered(place))
        _expansion = State(initialValue: PadTreeExpansion.remembered(place))
    }

    var body: some View {
        let tree = connection.oneTree(summary, filter: filter)
        List {
            Section { rows(tree.places) }
            Section {
                if connection.boardRead(summary) == .failed {
                    Text("Far Cooler couldn’t read this board.")
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("tree-board-failed")
                    Button(PlanWords.tryAgain) { Task { _ = await connection.readBoard(summary) } }
                } else if connection.boards[summary.id] == nil {
                    ProgressView()
                        .frame(maxWidth: .infinity)
                        .accessibilityLabel("Reading the board")
                } else if tree.tree.isEmpty {
                    Text(filter == .inReview ? "Nothing is in review." : "No cards yet.")
                        .foregroundStyle(.secondary)
                } else {
                    rows(tree.tree)
                }
            } header: {
                header
            }
            if !tree.below.isEmpty {
                Section { rows(tree.below) }
            }
            if connection.daemon?.mayAct ?? true {
                Section {
                    Button("New Worktree…") { composing = true }
                    Button("From a Branch…") { fromBranch = true }
                }
            }
        }
        .listStyle(.sidebar)
        .accessibilityIdentifier("pad-tree")
        .onChange(of: filter) { _, chosen in PhoneTreeFilter.remember(chosen, for: place) }
        .onChange(of: expansion) { _, open in PadTreeExpansion.remember(open, for: place) }
        .task { await connection.readTree(summary) }
        .refreshable {
            _ = await connection.readBoard(summary)
            if connection.keepsPlan { await connection.readPlan(summary) }
        }
        .actionFailureAlert($moveFailure)
        .newWorktreeSheets(composing: $composing, fromBranch: $fromBranch, connection: connection, summary: summary)
    }

    /// The work's header: what it is, and which cards it shows.
    private var header: some View {
        HStack {
            Text(WorkspaceSegment.tree.title)
            Spacer()
            Menu {
                Picker("Show", selection: $filter) {
                    ForEach(OneTreeFilter.allCases, id: \.self) { filter in
                        Text(filter.title).tag(filter)
                    }
                }
            } label: {
                Label("Show: \(filter.title)", systemImage: "line.3.horizontal.decrease.circle")
                    .labelStyle(.iconOnly)
            }
            .accessibilityIdentifier("tree-filter")
        }
    }

    @ViewBuilder
    private func rows(_ nodes: [OneTreeNode]) -> some View {
        ForEach(OneTree.rows(nodes, expansion: expansion)) { row in
            PadTreeRow(
                row: row, connection: connection, place: place,
                chosen: row.node.target != nil && row.node.target == PadPick.target(of: canvas),
                failure: $moveFailure,
                onToggle: { expansion.toggle(row.node) },
                onPick: { pick(row.node) })
        }
    }

    /// A row that goes somewhere goes there; a group with nowhere of its
    /// own opens or closes.
    private func pick(_ node: OneTreeNode) {
        if let target = node.target {
            onPick(PadPick.of(target, in: place))
        } else if node.hasChildren {
            expansion.toggle(node)
        }
    }
}

/// One row of the outline: its disclosure, glyph, key and title, its
/// detail, and the amber dot when it asks, or when it's closed over
/// something that does.
private struct PadTreeRow: View {
    let row: OneTreeRow
    @ObservedObject var connection: Connection
    let place: PhoneWorkspace
    let chosen: Bool
    @Binding var failure: ActionFailure?
    let onToggle: () -> Void
    let onPick: () -> Void

    @Environment(\.colorScheme) private var scheme
    @Environment(\.colorSchemeContrast) private var contrast

    private var node: OneTreeNode { row.node }

    var body: some View {
        HStack(spacing: 4) {
            disclosure
            Button(action: onPick) { label }
                .buttonStyle(.plain)
                .accessibilityAddTraits(chosen ? .isSelected : [])
                .accessibilityIdentifier("pad-tree-row-\(node.key.isEmpty ? node.title : node.key)")
        }
        .padding(.leading, CGFloat(row.depth) * 14)
        .listRowBackground(chosen ? Fill.selection(active: true, contrast: contrast) : nil)
        .worktreeSwipe(node: node, connection: connection, place: place, failure: $failure)
    }

    @ViewBuilder
    private var disclosure: some View {
        if node.hasChildren {
            Button(action: onToggle) {
                Image(systemName: "chevron.right")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .rotationEffect(.degrees(row.expanded ? 90 : 0))
                    .frame(width: 20, height: 28)
                    .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(node.key.isEmpty ? node.title : "\(node.key) \(node.title)")
            .accessibilityValue(row.expanded ? "Expanded" : "Collapsed")
            .accessibilityHint(row.expanded ? "Collapses the row" : "Expands the row")
            .accessibilityIdentifier("pad-tree-disclose-\(node.key.isEmpty ? node.title : node.key)")
        } else {
            Color.clear.frame(width: 20, height: 1).accessibilityHidden(true)
        }
    }

    private var label: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: node.glyph)
                .foregroundStyle(.secondary)
                .frame(width: 20)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    if !node.key.isEmpty {
                        Text(node.key)
                            .font(.subheadline.monospaced())
                            .foregroundStyle(.secondary)
                    }
                    Text(node.title)
                        .foregroundStyle(node.quiet ? .secondary : .primary)
                        .lineLimit(2)
                }
                if !node.also.isEmpty {
                    Text(node.also).font(.caption).foregroundStyle(.secondary)
                }
                if !node.caption.isEmpty {
                    Text(node.caption).font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 4)
            if row.dot {
                Circle()
                    .fill(Tint.attention(scheme))
                    .frame(width: 8, height: 8)
                    .accessibilityLabel(OneTreeWords.needsYou)
                    .accessibilityIdentifier("tree-dot")
            }
            if !node.detail.isEmpty {
                Text(node.detail)
                    .font(.subheadline.monospacedDigit())
                    .foregroundStyle(.secondary)
            }
        }
        .contentShape(.rect)
        .accessibilityElement(children: .combine)
    }
}

// MARK: - The plan column

/// The canvas: the plan's home, or what was picked, under a header that
/// names it and, away from the plan, goes back to it.
struct PadCanvasColumn<Board: View>: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    let place: PhoneWorkspace
    @Binding var canvas: PadCanvas
    @ViewBuilder let board: () -> Board

    @ObservedObject private var reads: PlanReads
    @ObservedObject private var pageReads: PageReads
    @Environment(\.phoneNavigator) private var navigator

    init(
        connection: Connection, summary: WorkspaceSummary, place: PhoneWorkspace, canvas: Binding<PadCanvas>,
        @ViewBuilder board: @escaping () -> Board
    ) {
        self.connection = connection
        self.summary = summary
        self.place = place
        _canvas = canvas
        self.board = board
        reads = connection.plans
        pageReads = connection.pages
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Group {
                switch canvas {
                case .plan:
                    PlanHomeList(connection: connection, summary: summary, hook: hook) { navigator?.go([]) }
                case .board:
                    board()
                case .page(let page):
                    PlanPageScreen(connection: connection, place: place, page: page, titled: false)
                        .id(page)
                case .task(let task):
                    TaskScreen(connection: connection, place: place, task: task, titled: false)
                        .id(task)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .environment(\.canvasOpen, open)
        .task { await connection.readTree(summary) }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("pad-canvas")
    }

    /// What it shows, named, and the way back to the plan from anything else.
    private var header: some View {
        HStack(spacing: 8) {
            if canvas != .plan {
                Button {
                    canvas = .plan
                } label: {
                    Label(OneTreeWords.plan, systemImage: "chevron.backward")
                        .labelStyle(.iconOnly)
                        .font(.body.weight(.semibold))
                        .frame(minWidth: 28, minHeight: 28)
                        .contentShape(.rect)
                }
                .accessibilityLabel("Back to Plan")
                .accessibilityIdentifier("pad-canvas-plan")
            }
            Text(title)
                .font(.headline)
                .lineLimit(1)
                .accessibilityAddTraits(.isHeader)
                .accessibilityIdentifier("pad-canvas-title")
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 16)
        .frame(minHeight: 44)
    }

    private var title: String {
        switch canvas {
        case .plan: return OneTreeWords.plan
        case .board: return WorkspaceSegment.board.title
        case .page(let page):
            return PlanPageScreen.title(page, plan: reads.state(place.workspace)?.plan, pages: pageReads.pages(place.workspace))
        case .task(let id):
            guard let row = connection.boards[place.workspace]?.rows.first(where: { $0.id == id }) else { return "Task" }
            return "\(row.key) \(row.title)"
        }
    }

    /// A page's own links: another page, or a task, shows here too.
    private func open(_ route: PhoneRoute) -> Bool {
        switch route {
        case .plan(let at, let page) where at == place:
            canvas = .page(page)
            return true
        case .task(let at, let task) where at == place:
            canvas = .task(task)
            return true
        default:
            return false
        }
    }

    /// The plan's sections open their pages here. A ruling's Reverse or
    /// Discuss stays: the chat it went to is beside it.
    private var hook: PlanBoardHook {
        connection.planHomeHook(summary, place: place, onOpen: { canvas = .page($0) }, onRuling: { _ in })
    }
}

// MARK: - The chat column

/// The chat's header: the orchestrator, by its state's mark and word, on the
/// screen's own ground, so the navigation bar above it never sits on the
/// terminal's dark.
struct PadChatHeader: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let state = PhoneTree.orchestrator(OrchestratorSegment.terminal(in: connection, summary: summary))
        HStack(spacing: 8) {
            Image(systemName: state.glyph)
                .foregroundStyle(state.tone == .attention ? Tint.attention(scheme) : state.tone == .failure ? Tint.failure : .secondary)
                .accessibilityHidden(true)
            Text("Orchestrator · \(state.word)")
                .font(.headline)
                .lineLimit(1)
                .accessibilityAddTraits(.isHeader)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 16)
        .frame(minHeight: 44)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("pad-chat-header")
    }
}

// MARK: - The sidebar shown on demand

/// The tree over the plan, from the leading edge, when two columns leave no
/// room for it as a third: a scrim behind it closes it, as Escape and a pick
/// do, and VoiceOver reads only it while it's up.
struct PadTreeSidebar<Tree: View>: View {
    let width: Double
    let close: () -> Void
    @ViewBuilder let tree: () -> Tree

    var body: some View {
        ZStack(alignment: .leading) {
            Button(action: close) {
                Color.black.opacity(0.2)  // style-exempt: the scrim behind a sidebar shown over content, as the system's own
                    .ignoresSafeArea()
            }
            .buttonStyle(.plain)
            .keyboardShortcut(.cancelAction)
            .accessibilityLabel("Close Tree")
            .accessibilityIdentifier("pad-tree-scrim")
            tree()
                .frame(width: width)
                .background(Surface.contentFill)
                .accessibilityAddTraits(.isModal)
                .accessibilityAction(.escape, close)
        }
        .transition(.move(edge: .leading).combined(with: .opacity))
    }
}

// MARK: - Shared with the phone's tree

extension View {
    /// Hide and Unhide on a lane's or a loose worktree's own checkout: the
    /// Worktrees segment's swipe, kept. Not below a Control grant.
    func worktreeSwipe(
        node: OneTreeNode, connection: Connection, place: PhoneWorkspace, failure: Binding<ActionFailure?>
    ) -> some View {
        swipeActions(edge: .trailing) {
            WorktreeSwipeActions(node: node, connection: connection, place: place, failure: failure)
        }
    }

    /// New Worktree… and From a Branch…, as sheets.
    func newWorktreeSheets(
        composing: Binding<Bool>, fromBranch: Binding<Bool>, connection: Connection, summary: WorkspaceSummary
    ) -> some View {
        sheet(isPresented: composing) {
            TaskComposerView(connection: connection, workspace: summary)
        }
        .sheet(isPresented: fromBranch) {
            NewWorktreeView(
                repositories: connection.repositories.filter {
                    summary.repository == nil || $0.id == summary.repository
                },
                connection: connection
            ) { repository, name, branch, adopt in
                return await connection.createWorktree(
                    repository: repository, name: name, branch: branch, adopt: adopt,
                    workspace: summary.boardWorkspace)
            }
        }
    }
}

/// A row's Hide or Unhide, when it's a worktree's own row.
struct WorktreeSwipeActions: View {
    let node: OneTreeNode
    @ObservedObject var connection: Connection
    let place: PhoneWorkspace
    @Binding var failure: ActionFailure?

    /// The row's worktree, when it's a lane's or a loose one's own checkout,
    /// or the repository's main checkout.
    static func worktree(_ node: OneTreeNode, in connection: Connection) -> Worktree? {
        guard node.kind == .lane || node.kind == .worktree || node.id == "group:main", let id = node.worktreeID
        else { return nil }
        return connection.fleet.worktrees.first { $0.id == id }
    }

    var body: some View {
        // The checkout too, where this workspace owns it, as the Worktrees
        // list offered it (review 14).
        if let worktree = Self.worktree(node, in: connection),
            !worktree.isPrimaryCheckout || worktree.workspace == place.workspace,
            connection.daemon?.mayAct ?? true
        {
            if worktree.isHidden {
                Button("Unhide") { Task { failure = await connection.unhideWorktree(worktree) } }
                    .tint(.blue)
                    .accessibilityIdentifier("unhide-\(worktree.task)")
            } else {
                Button("Hide") { Task { failure = await connection.hideWorktree(worktree) } }
                    .tint(.gray)
                    .accessibilityIdentifier("hide-\(worktree.task)")
            }
        }
    }
}

#if DEBUG
/// `-pad-compact`: the app as a one-third Split View draws it, 375 points
/// wide at compact width, for a test and a capture of the fallback. Only the
/// window is stood in; everything in it is the shipping code.
struct PadCompactWindow: ViewModifier {
    static var isRequested: Bool { CommandLine.arguments.contains("-pad-compact") }

    func body(content: Content) -> some View {
        if Self.isRequested {
            content
                .environment(\.horizontalSizeClass, .compact)
                .frame(width: 375)
                .frame(maxWidth: .infinity, alignment: .leading)
        } else {
            content
        }
    }
}
#endif
