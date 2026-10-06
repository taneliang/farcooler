import SwiftUI

// The One tree on an iPhone (ov-300; ov-321's iPhone line, ia.md :223): the
// workspace's Themes segment is the tree's root, and each level is pushed,
//
//   Themes › Theme › Task › Lane › Terminal
//
// where a lane is its worktree. A row with children pushes its level, a level
// opens on its node's own page, and a leaf opens what it points at. Back is
// up. The Worktrees segment's list folds in here: a lane's worktree under its
// card, the rest under Main Checkout and Loose Worktrees, with Hide and Unhide
// on a worktree's swipe and New Worktree… at the root's foot.
//
// Every rule is AgentKit's (`OneTree`, `PhoneTree`); this lays out rows.

extension Connection {
    /// `summary`'s tree, as this connection holds its board, plan, pages and
    /// worktrees, narrowed by `filter`.
    func oneTree(_ summary: WorkspaceSummary, filter: OneTreeFilter) -> OneTree {
        OneTree.build(
            PhoneTree.input(
                summary: summary, board: boards[summary.id], plan: plans.state(summary.id)?.plan ?? .empty,
                worktrees: fleet.worktrees, pages: keepsPages ? pages.pages(summary.id) : [], items: needsYou,
                filter: filter, needsYouCount: workspaceNeedsYou(summary)))
    }

    /// Where `summary`'s board read stands, for a level waiting on it.
    func boardRead(_ summary: WorkspaceSummary) -> PhoneTree.Read {
        if boards[summary.id] != nil { return .read }
        return unreadBoards.contains(summary.id) ? .failed : .pending
    }

    /// Where `summary`'s plan read stands. A runner that keeps none has
    /// nothing to wait for; one that needs an update has answered.
    func planRead(_ summary: WorkspaceSummary) -> PhoneTree.Read {
        guard keepsPlan else { return .notKept }
        switch plans.state(summary.id) {
        case nil, .loading?: return .pending
        case .unavailable?: return .failed
        default: return .read
        }
    }

    /// Read what the tree is built from that hasn't been read yet.
    func readTree(_ summary: WorkspaceSummary) async {
        if boards[summary.id] == nil { _ = await readBoard(summary) }
        if keepsPlan, plans.state(summary.id) == nil { await readPlan(summary) }
        if keepsPages, pages.state(summary.id) == nil { await readPages(summary) }
    }
}

/// Which cards a workspace's tree shows, kept on this device.
enum PhoneTreeFilter {
    static func key(_ place: PhoneWorkspace) -> String { "tree.filter.\(place.runner).\(place.workspace)" }

    static func remembered(_ place: PhoneWorkspace) -> OneTreeFilter {
        UserDefaults.standard.string(forKey: key(place)).flatMap(OneTreeFilter.init(rawValue:)) ?? .open
    }

    static func remember(_ filter: OneTreeFilter, for place: PhoneWorkspace) {
        UserDefaults.standard.set(filter.rawValue, forKey: key(place))
    }
}

/// The tree's root: the Themes segment.
struct TreeRootList: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    let place: PhoneWorkspace

    @ObservedObject private var reads: PlanReads
    @ObservedObject private var pageReads: PageReads
    @State private var filter: OneTreeFilter
    @State private var composing = false
    @State private var fromBranch = false
    @State private var moveFailure: ActionFailure?

    init(connection: Connection, summary: WorkspaceSummary, place: PhoneWorkspace) {
        self.connection = connection
        self.summary = summary
        self.place = place
        reads = connection.plans
        pageReads = connection.pages
        _filter = State(initialValue: PhoneTreeFilter.remembered(place))
    }

    var body: some View {
        let root = PhoneTree.root(connection.oneTree(summary, filter: filter))
        List {
            if connection.boardRead(summary) == .failed {
                Section {
                    Text("Far Cooler couldn’t read this board.")
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("tree-board-failed")
                    Button(PlanWords.tryAgain) { Task { _ = await connection.readBoard(summary) } }
                        .accessibilityIdentifier("tree-board-retry")
                }
            } else if connection.boards[summary.id] == nil {
                Section {
                    ProgressView()
                        .frame(maxWidth: .infinity)
                        .accessibilityLabel("Reading the board")
                        .accessibilityIdentifier("tree-loading")
                }
            } else if root.work.isEmpty {
                Section {
                    Text(filter == .inReview ? "Nothing is in review." : "No cards yet. The orchestrator files them as it plans the work.")
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("tree-empty")
                }
            } else {
                Section {
                    ForEach(root.work) { node in
                        TreeRow(node: node, connection: connection, place: place, failure: $moveFailure)
                    }
                }
            }
            if !root.below.isEmpty {
                Section {
                    ForEach(root.below) { node in
                        TreeRow(node: node, connection: connection, place: place, failure: $moveFailure)
                    }
                }
            }
            if connection.daemon?.mayAct ?? true {
                Section {
                    Button("New Worktree…") { composing = true }
                        .accessibilityIdentifier("new-worktree")
                    Button("From a Branch…") { fromBranch = true }
                }
            }
        }
        .listStyle(.insetGrouped)
        .accessibilityIdentifier("tree-root")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Picker("Show", selection: $filter) {
                        ForEach(OneTreeFilter.allCases, id: \.self) { filter in
                            Text(filter.title).tag(filter)
                        }
                    }
                } label: {
                    Label("Show: \(filter.title)", systemImage: "line.3.horizontal.decrease.circle")
                }
                .accessibilityIdentifier("tree-filter")
            }
        }
        .onChange(of: filter) { _, chosen in PhoneTreeFilter.remember(chosen, for: place) }
        .task { await connection.readTree(summary) }
        .refreshable {
            _ = await connection.readBoard(summary)
            if connection.keepsPlan { await connection.readPlan(summary) }
        }
        .actionFailureAlert($moveFailure)
        .newWorktreeSheets(composing: $composing, fromBranch: $fromBranch, connection: connection, summary: summary)
    }
}

/// One level of the tree, pushed: a node's own page first, then its children.
struct TreeLevelScreen: View {
    @ObservedObject var connection: Connection
    let place: PhoneWorkspace
    let nodeID: String

    @ObservedObject private var reads: PlanReads
    @State private var moveFailure: ActionFailure?
    @Environment(\.phoneNavigator) private var navigator

    init(connection: Connection, place: PhoneWorkspace, nodeID: String) {
        self.connection = connection
        self.place = place
        self.nodeID = nodeID
        reads = connection.plans
    }

    var body: some View {
        let summary = connection.workspace(place.workspace)
        let node = summary.flatMap(node)
        Group {
            // A workspace missing while its runner isn't answering is a
            // reconnect, not a level gone (review R2-5).
            switch summary.map({ PhoneTree.level(found: node != nil, board: connection.boardRead($0), plan: connection.planRead($0)) })
                ?? (connection.isAnswering ? .gone : .loading)
            {
            case .node:
                if let node { level(node) }
            case .loading:
                // A level restored on a relaunch arrives before the board and
                // the plan it's built from (review 1): it waits, and says so.
                ProgressView("Reading the plan…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .accessibilityIdentifier("tree-level-loading")
            case .failed:
                ContentUnavailableView {
                    Label(PlanWords.couldntRead, systemImage: "exclamationmark.triangle")
                } actions: {
                    Button(PlanWords.tryAgain) {
                        guard let summary else { return }
                        Task {
                            _ = await connection.readBoard(summary)
                            if connection.keepsPlan { await connection.readPlan(summary) }
                        }
                    }
                    .accessibilityIdentifier("tree-level-retry")
                }
                .accessibilityIdentifier("tree-level-failed")
            case .gone:
                ContentUnavailableView {
                    Label("No Longer Here", systemImage: "questionmark.folder")
                } description: {
                    Text("It finished or moved since this opened.")
                }
                .accessibilityIdentifier("tree-gone")
            }
        }
        .navigationTitle(node.map { $0.key.isEmpty ? $0.title : "\($0.key) \($0.title)" } ?? WorkspaceSegment.tree.title)
        .navigationBarTitleDisplayMode(.inline)
        // Read whatever it's built from, found or not: a restored level is
        // what asks for the plan when nothing else on the stack has.
        .task(id: summary?.id) {
            if let summary { await connection.readTree(summary) }
        }
        // Its cards' keys preview their tasks, as the board's do.
        .environment(\.taskKeyLinker, connection.taskKeyLinker(navigator))
    }

    /// The node, under the filter the root shows, else under All: a
    /// finished card opened before the filter changed is still itself.
    private func node(_ summary: WorkspaceSummary) -> OneTreeNode? {
        PhoneTree.node(nodeID, in: connection.oneTree(summary, filter: PhoneTreeFilter.remembered(place)))
            ?? PhoneTree.node(nodeID, in: connection.oneTree(summary, filter: .all))
    }

    private func level(_ node: OneTreeNode) -> some View {
        List {
            if let own = PhoneTree.ownRow(node), let target = node.target, let route = PhoneTree.route(target, in: place) {
                Section {
                    Button { navigator?.open(route) } label: {
                        HStack {
                            Label(own, systemImage: node.glyph)
                                .foregroundStyle(.tint)
                            Spacer()
                            Image(systemName: "chevron.forward")
                                .font(.footnote.weight(.semibold))
                                .foregroundStyle(.tertiary)
                        }
                        .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("tree-own")
                } footer: {
                    if !node.also.isEmpty { Text(node.also) }
                }
            }
            Section {
                ForEach(node.children) { child in
                    TreeRow(node: child, connection: connection, place: place, failure: $moveFailure)
                }
            } footer: {
                if !node.caption.isEmpty { Text(node.caption) }
            }
        }
        .listStyle(.insetGrouped)
        .actionFailureAlert($moveFailure)
        .accessibilityIdentifier("tree-level")
    }
}

/// One row of the tree: its glyph, key and title, its detail, the amber dot
/// when it or anything behind it asks for you, and where a tap goes.
struct TreeRow: View {
    let node: OneTreeNode
    @ObservedObject var connection: Connection
    let place: PhoneWorkspace
    @Binding var failure: ActionFailure?

    @Environment(\.phoneNavigator) private var navigator
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let tap = PhoneTree.tap(node)
        Group {
            if tap == .none {
                content(goes: false)
            } else {
                Button { perform(tap) } label: { content(goes: true) }
                    .buttonStyle(.plain)
            }
        }
        .accessibilityIdentifier(identifier)
        .worktreeSwipe(node: node, connection: connection, place: place, failure: $failure)
    }

    /// A worktree's row is named as the Worktrees segment's was, so a
    /// worktree is found by its name wherever it hangs; every other row by
    /// its key, else its title.
    private var identifier: String {
        if node.kind == .lane || node.kind == .worktree, case .worktree? = node.target {
            return "worktree-row-\(node.title)"
        }
        return "tree-row-\(node.key.isEmpty ? node.title : node.key)"
    }

    /// The row's worktree, when it's a lane's or a loose one's own checkout,
    /// or the repository's main checkout.
    private var worktree: Worktree? { WorktreeSwipeActions.worktree(node, in: connection) }

    private func perform(_ tap: PhoneTree.Tap) {
        switch tap {
        case .push(let id): navigator?.open(.tree(place, node: id))
        case .open(let target):
            if let route = PhoneTree.route(target, in: place) { navigator?.open(route) }
        case .none: break
        }
    }

    private func content(goes: Bool) -> some View {
        // On a phone every row with children is closed behind its push, so
        // the dot rolls up from anything under it.
        let dot = node.showsDot(expanded: false)
        return HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: node.glyph)
                .foregroundStyle(.secondary)
                .frame(width: 22)
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
                // Its branch, as the Worktrees list's row said it (review 13).
                if let branch = worktree?.branch, !branch.isEmpty {
                    Text(branch)
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                // Put away, but still its card's: said, where Unhide is.
                if worktree?.isHidden == true {
                    Text(OneTreeWords.hidden).font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 4)
            if dot {
                Circle()
                    .fill(Tint.attention(scheme))
                    .frame(width: 8, height: 8)
                    .accessibilityLabel(OneTreeWords.needsYou)
                    .accessibilityIdentifier("tree-dot")
            }
            // What changed in it and nobody has looked at, as the list said it.
            if let worktree, let inbox = connection.inbox[worktree.id], inbox.hasDiff {
                Text("+\(inbox.insertions) −\(inbox.deletions)")
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("tree-row-diff")
            }
            if !node.detail.isEmpty {
                Text(node.detail)
                    .font(.subheadline.monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            if goes {
                Image(systemName: "chevron.forward")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
        }
        .contentShape(.rect)
        .accessibilityElement(children: .combine)
    }
}
