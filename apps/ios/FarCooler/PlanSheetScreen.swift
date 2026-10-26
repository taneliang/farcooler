import Combine
import SwiftUI

// The Plan on an iPhone (ov-616): one view of the workspace's work, where the
// Themes and Board segments were two that listed the same themes twice.
//
// - Over a conversation it's a sheet, opened from the Now/Next card, at the
//   medium and large detents. The sheet has its own navigation stack
//   (`PhoneNavigator.plan`): a theme, a card, a lane, a page or a board's
//   history is pushed inside it, Back comes back to the Plan, and Done closes
//   it. A worktree, or another workspace, closes it first.
// - A workspace with no conversation has the Plan as its screen
//   (`WorkspaceScreen`), and pushes on the phone's stack.
//
// The list: the band, the orchestrator and what needs you (in the sheet);
// the status filters; Unread with Mark All as Read; the One tree's work
// (theme › card › lane › terminal); the board's history; the plan's lanes by
// where they stand, its pages, cost and rulings; the main checkout and loose
// worktrees; and New Worktree.

/// The Plan sheet over a workspace.
struct PlanSheetScreen: View {
    @ObservedObject var fleet: FleetStore
    @ObservedObject var hosts: RunnerStore
    @ObservedObject var navigator: PhoneNavigator
    let place: PhoneWorkspace

    /// Medium while the Plan's own list shows; large once something is
    /// pushed, which wants the room.
    @State private var detent: PresentationDetent = .medium

    var body: some View {
        NavigationStack(path: path) {
            root
                // A hard edge under the bar: a section's header scrolled up
                // under "Plan" and Done stops at the bar rather than showing
                // through it, which at accessibility sizes read as one jumble.
                .scrollEdgeEffectStyle(.hard, for: .top)
                .navigationTitle("Plan")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Done") { navigator.plan = nil }
                            .accessibilityIdentifier("plan-sheet-done")
                    }
                }
                .navigationDestination(for: PhoneRoute.self) { route in
                    PhoneScreen(route: route, fleet: fleet, hosts: hosts)
                }
        }
        .accessibilityIdentifier("plan-sheet")
        .presentationDetents([.medium, .large], selection: $detent)
        .presentationDragIndicator(.visible)
        .onAppear { if navigator.plan?.path.isEmpty == false { detent = .large } }
        .onChange(of: navigator.plan?.path.count ?? 0) { _, count in
            if count > 0 { withAnimation(.snappy) { detent = .large } }
        }
    }

    /// The sheet's own stack, kept on the navigator so a link and a relaunch
    /// can stand it up.
    private var path: Binding<[PhoneRoute]> {
        Binding(
            get: { navigator.plan?.path ?? [] },
            set: { screens in if navigator.plan != nil { navigator.plan?.path = screens } })
    }

    @ViewBuilder
    private var root: some View {
        if let connection = fleet.connection(for: place), let summary = connection.workspace(place.workspace) {
            PhonePlanList(connection: connection, summary: summary, place: place, inSheet: true, compactBand: detent == .medium) {
                EmptyView()
            }
        } else {
            RunnerGone()
        }
    }
}

/// The Plan's list: in the sheet, or as the screen of a workspace with no
/// conversation, under whatever `top` puts first (Start Orchestrator).
struct PhonePlanList<Top: View>: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    let place: PhoneWorkspace
    /// In the sheet over a conversation: the band, the orchestrator and Needs
    /// You lead it. The workspace's own screen has none of them.
    let inSheet: Bool
    /// The band says Now only, for a sheet at its medium detent.
    var compactBand = false
    @ViewBuilder let top: () -> Top

    @ObservedObject private var reads: PlanReads
    @ObservedObject private var pageReads: PageReads
    @State private var narrow: PlanFilter?
    @State private var composing = false
    @State private var fromBranch = false
    @State private var moveFailure: ActionFailure?
    /// The tree it last drew (ov-791): under a pushed screen it keeps that
    /// tree rather than building it again on every board move, a dropped
    /// frame each in the screen on top. On a phone only: in columns what's
    /// beside the top screen is in view.
    @State private var held = HeldTree()
    @State private var moves = 0
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.phoneNavigator) private var navigator

    init(
        connection: Connection, summary: WorkspaceSummary, place: PhoneWorkspace, inSheet: Bool, compactBand: Bool = false,
        @ViewBuilder top: @escaping () -> Top
    ) {
        self.connection = connection
        self.summary = summary
        self.place = place
        self.inSheet = inSheet
        self.compactBand = compactBand
        self.top = top
        reads = connection.plans
        pageReads = connection.pages
        _narrow = State(initialValue: PlanFilter.remembered(runner: place.runner, workspace: place.workspace))
    }

    var body: some View {
        let root = tree
        List {
            top()
            if inSheet {
                PlanHomeHeader(connection: connection, summary: summary, compactBand: compactBand) { navigator?.go([]) }
            }
            Section {
                PlanFilterBar(selection: $narrow)
                    .listRowInsets(EdgeInsets())
                    .listRowBackground(Color.clear)
            }
            .listSectionSpacing(.custom(Spacing.group))
            if narrow == nil, let board = connection.boards[summary.id] {
                BoardUnreadSection(
                    board: board, reads: connection.boardReads[summary.id] ?? .firstLook(now: Date()),
                    readNotes: { [connection] row in await connection.taskRecord(row.id)?.detail.notes },
                    readsAreShared: { [connection, summary] in connection.readsAreShared(workspace: summary.id) },
                    onOpen: { row in navigator?.open(.task(place, task: row.id)) },
                    onMarkAllRead: { [connection, summary] latest in
                        guard let rows = connection.boards[summary.id]?.rows else { return }
                        connection.markAllRead(rows: rows, latest: latest, workspace: summary.id)
                    })
            }
            Section {
                TreeWorkRows(
                    connection: connection, summary: summary, filter: narrow?.base ?? .open, isEmpty: root.work.isEmpty,
                    emptyWords: narrow?.emptyWords
                ) {
                    ForEach(root.work) { node in
                        TreeRow(node: node, connection: connection, place: place, failure: $moveFailure)
                    }
                }
            }
            history
            if connection.keepsPlan, narrow == nil {
                PlanBoardSections(hook: hook)
            }
            if !root.below.isEmpty, narrow == nil {
                Section {
                    ForEach(root.below) { node in
                        TreeRow(node: node, connection: connection, place: place, failure: $moveFailure)
                    }
                }
            }
            if narrow == nil {
                TreeWorktreeActions(connection: connection, composing: $composing, fromBranch: $fromBranch)
            }
        }
        .listStyle(.insetGrouped)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("plan-list")
        .onChange(of: narrow) { _, chosen in
            PlanFilter.remember(chosen, runner: place.runner, workspace: place.workspace)
        }
        .task { await connection.readTree(summary) }
        // Drawn again when the stack moves, so it builds its tree once it's
        // on top again.
        .onReceive(navigator?.objectWillChange.eraseToAnyPublisher() ?? Empty<Void, Never>().eraseToAnyPublisher()) { _ in moves &+= 1 }
        .refreshable {
            _ = await connection.readBoard(summary)
            if connection.keepsPlan { await connection.readPlan(summary) }
            if connection.keepsPages { await connection.readPages(summary) }
        }
        .actionFailureAlert($moveFailure)
        .newWorktreeSheets(composing: $composing, fromBranch: $fromBranch, connection: connection, summary: summary)
        // The tree's and the plan's keys preview their tasks (ov-299).
        .environment(\.taskKeyLinker, connection.taskKeyLinker(navigator))
    }

    /// The tree under its filter: the one last drawn while a screen is over
    /// it, else built from what's read.
    private var tree: (work: [OneTreeNode], below: [OneTreeNode]) {
        _ = moves
        let covered = sizeClass == .compact && navigator?.isOnTop(inSheet ? nil : .workspace(place)) == false
        if covered, let kept = held.root { return kept }
        let made = PhoneTree.root(connection.oneTree(summary, filter: .open, narrow: narrow))
        held.root = made
        return made
    }

    /// "All Done  94 ›" and "All Canceled", the board's History pages
    /// (ov-103): under no filter, and under Done.
    @ViewBuilder
    private var history: some View {
        if narrow == nil || narrow == .done, let board = connection.boards[summary.id] {
            let finished = [TaskStatus.done, .cancelled].map { status in
                (status, board.rows.filter { $0.status == status }.count)
            }.filter { $0.1 > 0 }
            if !finished.isEmpty {
                Section {
                    ForEach(finished, id: \.0) { status, total in
                        Button { navigator?.open(.history(place, status: status.rawValue)) } label: {
                            HStack {
                                Text(BoardDone.historyTitle(status)).foregroundStyle(.primary)
                                Spacer()
                                Text("\(total)")
                                    .monospacedDigit()
                                    .foregroundStyle(.secondary)
                                Image(systemName: "chevron.forward")
                                    .font(.footnote.weight(.semibold))
                                    .foregroundStyle(.tertiary)
                                    .accessibilityHidden(true)
                            }
                            .contentShape(.rect)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("\(BoardDone.historyTitle(status)), \(total)")
                        .accessibilityIdentifier("plan-history-\(status.rawValue)")
                    }
                }
            }
        }
    }

    /// What the Plan's own sections read and where they go: the lanes and
    /// trains, the agents and their spend, pages, cost and rulings, which the
    /// tree doesn't list, and not the themes, which it does, so no card or
    /// theme is listed twice.
    private var hook: PlanBoardHook {
        var hook = connection.planHomeHook(
            summary, place: place, onOpen: { [navigator] page in navigator?.open(.plan(place, page: page)) },
            onRoute: { [navigator] route in navigator?.open(route) },
            // Reverse and Discuss draft into the orchestrator's pane, which is
            // the workspace under the sheet (ov-616): closing the sheet is
            // the way there, as Done is, and no worktree covers it (ov-643).
            // Without a conversation under it (the Plan is the screen), the
            // route still opens.
            onRuling: { [navigator, inSheet] route in
                if inSheet { navigator?.plan = nil } else { navigator?.open(route) }
            })
        // In the sheet the band leads the list, over the orchestrator's line.
        hook.showsBand = !inSheet
        // The tree lists every theme. The Agents section stays: its tokens,
        // cost, role, lane and the day's spend are the plan's, not the card's.
        hook.kinds = Set(PlanSections.Kind.allCases).subtracting([.themes])
        return hook
    }
}

/// Needs You · In Progress · In Review · Done: one chosen, or none for every
/// card not done. A chosen one is chosen again to clear it, as a filter in
/// Mail is.
struct PlanFilterBar: View {
    @Binding var selection: PlanFilter?

    var body: some View {
        // Wrapped onto a second line where four don't fit (a narrow phone, a
        // large type size) rather than cut at the list's edge.
        ChipFlow(spacing: Spacing.group) {
            ForEach(PlanFilter.allCases, id: \.self) { filter in
                chip(filter)
            }
        }
        .padding(.vertical, Spacing.tight)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("plan-filters")
    }

    private func chip(_ filter: PlanFilter) -> some View {
        let chosen = selection == filter
        return Button {
            withAnimation(.snappy) { selection = chosen ? nil : filter }
        } label: {
            Text(filter.title)
                .font(.subheadline.weight(chosen ? .semibold : .regular))
                .lineLimit(1)
                .fixedSize()
                .padding(.horizontal, Spacing.group)
                .padding(.vertical, Spacing.tight)
                .foregroundStyle(chosen ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary))
                .background(chosen ? Fill.selection(active: true) : Fill.inset(), in: Capsule())
                .contentShape(.capsule)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(chosen ? .isSelected : [])
        .accessibilityHint(chosen ? "Shows every card not done" : "")
        .accessibilityIdentifier("plan-filter-\(filter.rawValue)")
    }
}

/// Chips in rows: as many as fit on each line, left to right.
struct ChipFlow: Layout {
    var spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = rows(subviews, width: proposal.width ?? .infinity)
        let height = rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(rows.count - 1, 0))
        let width = rows.map(\.width).max() ?? 0
        return CGSize(width: proposal.width ?? width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in rows(subviews, width: bounds.width) {
            var x = bounds.minX
            for index in row.items {
                let size = subviews[index].sizeThatFits(.unspecified)
                subviews[index].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
                x += size.width + spacing
            }
            y += row.height + spacing
        }
    }

    private func rows(_ subviews: Subviews, width: CGFloat) -> [(items: [Int], width: CGFloat, height: CGFloat)] {
        var out: [(items: [Int], width: CGFloat, height: CGFloat)] = []
        for index in subviews.indices {
            let size = subviews[index].sizeThatFits(.unspecified)
            if let last = out.indices.last, out[last].width + spacing + size.width <= width {
                out[last].items.append(index)
                out[last].width += spacing + size.width
                out[last].height = max(out[last].height, size.height)
            } else {
                out.append(([index], size.width, size.height))
            }
        }
        return out
    }
}

/// `PhonePlanList`'s last tree, kept by reference: written as it's drawn,
/// read while it's covered, and never a reason to draw again.
final class HeldTree {
    var root: (work: [OneTreeNode], below: [OneTreeNode])?
}
