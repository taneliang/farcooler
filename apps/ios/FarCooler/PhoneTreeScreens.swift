import Combine
import SwiftUI

// The One tree on an iPhone (ov-300; ov-321's iPhone line, ia.md :223): the
// Plan's list is the tree's root (`PhonePlanList`, ov-616), and each level is
// pushed, inside the Plan sheet,
//
//   Plan › Theme › Task › Lane › Terminal
//
// where a lane is its worktree. A row with children pushes its level, a level
// opens on its node's own page, and a leaf opens what it points at. Back is
// up. The Worktrees segment's list folds in here: a lane's worktree under its
// card, the rest under Main Checkout and Loose Worktrees, with Hide and Unhide
// on a worktree's swipe and New Worktree… at the root's foot.
//
// Every rule is AgentKit's (`OneTree`, `PhoneTree`); this lays out rows.

extension Connection {
    /// What `summary`'s tree is built from, as this connection holds its
    /// board, plan, pages and worktrees, narrowed by `filter`.
    func treeInput(_ summary: WorkspaceSummary, filter: OneTreeFilter, narrow: PlanFilter? = nil) -> OneTreeInput {
        PhoneTree.input(
            summary: summary, board: boards[summary.id], plan: plans.state(summary.id)?.plan ?? .empty,
            worktrees: fleet.worktrees, pages: keepsPages ? pages.pages(summary.id) : [], items: needsYou,
            filter: filter, narrow: narrow, needsYouCount: workspaceNeedsYou(summary))
    }

    /// `summary`'s One tree: the phone's pushed levels, narrowed by the Plan
    /// sheet's filter when one is chosen (ov-616).
    func oneTree(_ summary: WorkspaceSummary, filter: OneTreeFilter, narrow: PlanFilter? = nil) -> OneTree {
        OneTree.build(treeInput(summary, filter: filter, narrow: narrow))
    }

    /// `summary`'s sidebar as two lists, Themes and Lanes: the iPad's outline
    /// (ov-503).
    func sidebarOutline(_ summary: WorkspaceSummary, filter: OneTreeFilter) -> SidebarOutline {
        SidebarOutline.build(treeInput(summary, filter: filter))
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
        case nil, .loading?, .reconnecting?: return .pending
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

/// The tree's work, or why there's none: the board couldn't be read, is
/// being read, or has no card under the filter. The phone's root and the
/// iPad's outline both draw it, so they say the same thing.
struct TreeWorkRows<Rows: View>: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    let filter: OneTreeFilter
    /// Whether the filter leaves no work to draw.
    let isEmpty: Bool
    /// What it says with nothing under the Plan's filter (ov-616); nil for
    /// the tree's own words.
    var emptyWords: String? = nil
    @ViewBuilder let rows: () -> Rows

    var body: some View {
        if connection.boardRead(summary) == .failed {
            Text("Far Cooler couldn’t read this board.")
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("tree-board-failed")
            Button(PlanWords.tryAgain) { Task { _ = await connection.readBoard(summary) } }
                .accessibilityIdentifier("tree-board-retry")
        } else if connection.boards[summary.id] == nil {
            ProgressView()
                .frame(maxWidth: .infinity)
                .accessibilityLabel("Reading the board")
                .accessibilityIdentifier("tree-loading")
        } else if isEmpty {
            Text(emptyWords ?? (filter == .inReview ? "Nothing is in review." : "No cards yet. The orchestrator files them as it plans the work."))
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("tree-empty")
        } else {
            rows()
        }
    }
}

/// New Worktree… and From a Branch…, at the tree's foot. Not below a
/// Control grant.
struct TreeWorktreeActions: View {
    @ObservedObject var connection: Connection
    @Binding var composing: Bool
    @Binding var fromBranch: Bool

    var body: some View {
        if connection.daemon?.mayAct ?? true {
            Section {
                Button("New Worktree…") { composing = true }
                    .accessibilityIdentifier("new-worktree")
                Button("From a Branch…") { fromBranch = true }
            }
        }
    }
}

/// One level of the tree, pushed: a node's own page first, then its children.
struct TreeLevelScreen: View {
    @ObservedObject var connection: Connection
    let place: PhoneWorkspace
    let nodeID: String

    @ObservedObject private var reads: PlanReads
    @State private var moveFailure: ActionFailure?
    /// Under a pushed screen, on a phone, it keeps the node it last drew
    /// rather than building the tree again on every board move (ov-791).
    @State private var held = HeldNode()
    @State private var moves = 0
    @Environment(\.horizontalSizeClass) private var sizeClass
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
        // Drawn again when the stack moves, so it builds its node once it's
        // on top again.
        .onReceive(navigator?.objectWillChange.eraseToAnyPublisher() ?? Empty<Void, Never>().eraseToAnyPublisher()) { _ in moves &+= 1 }
    }

    /// The node, under the filter the root shows, else under All: a
    /// finished card opened before the filter changed is still itself.
    /// Covered, the one last drawn.
    private func node(_ summary: WorkspaceSummary) -> OneTreeNode? {
        _ = moves
        let covered = sizeClass == .compact && navigator?.isOnTop(.tree(place, node: nodeID)) == false
        if covered, let kept = held.node { return kept }
        let made =
            PhoneTree.node(nodeID, in: connection.oneTree(summary, filter: PhoneTreeFilter.remembered(place)))
            ?? PhoneTree.node(nodeID, in: connection.oneTree(summary, filter: .all))
        held.node = made
        return made
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
    @Environment(\.dynamicTypeSize) private var typeSize
    /// The glyph's column, which grows with the glyph so it never reaches the title (ov-424).
    @ScaledMetric(relativeTo: .body) private var glyphWidth: CGFloat = 22

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
            return "worktree-row-\(node.rowName)"
        }
        return "tree-row-\(node.rowName)"
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
                .frameProbe("tree-row-icon")  // the glyph's own size, which a fixed column does not contain
                .frame(width: glyphWidth)
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
                        .frameProbe("tree-row-title")
                }
                if !node.also.isEmpty {
                    Text(node.also).font(.caption).foregroundStyle(.secondary)
                }
                if !node.caption.isEmpty {
                    Text(node.caption).font(.caption).foregroundStyle(.secondary)
                }
                // Under the title at the accessibility sizes, where beside it
                // the count took the title's room and broke it ("Invoic-/es").
                if !node.detail.isEmpty, typeSize.isAccessibilitySize {
                    Text(node.detail)
                        .font(.subheadline.monospacedDigit())
                        .foregroundStyle(.secondary)
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
            if !node.detail.isEmpty, !typeSize.isAccessibilitySize {
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
        .probesShown()
    }
}

/// `TreeLevelScreen`'s last node, kept by reference: written as it's drawn,
/// read while it's covered, and never a reason to draw again.
final class HeldNode {
    var node: OneTreeNode?
}
