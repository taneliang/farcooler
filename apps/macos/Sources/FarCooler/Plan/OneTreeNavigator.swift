import AgentKit
import SwiftUI

// The navigator as one tree (ov-321, Concept 1 of
// .claude/agent/reports/ov-321/ia.md): three pinned places, then Theme › Task
// › Lane › Terminals, then under a divider the main checkout and the loose
// worktrees. Every rule (what's under what, the "also" marker, the roll-up of
// a Needs You dot, which theme opens first) is AgentKit's `OneTree`; this lays
// out its rows.
//
// It takes the place of the sections a planned board drew (Themes, Pages,
// Tasks by status, Terminals, Worktrees). The task list by status is still
// there, as the Board view (View ▸ Board, or ⌘K).

/// What the window hands the navigator to draw the tree.
struct OneTreeSidebar {
    /// What the tree is built from, under the filter chosen.
    var input: (OneTreeFilter) -> OneTreeInput
    /// What the window shows, as the node it is.
    var selected: OneTreeTarget?
    /// The row chosen last, by id: which copy of a lane under two cards.
    var hint: String?
    /// Where this window keeps the tree's choices: the runner and workspace.
    var key: String
    /// A row chosen: the window goes where it points.
    var onChoose: (OneTreeNode) -> Void
}

/// The tree, drawn: the navigator's content in place of its sections.
struct OneTreeNavigator: View {
    let sidebar: OneTreeSidebar
    /// The navigator's filter field (⌘F).
    let filterText: String
    /// The navigator has the keyboard: a selected row reads in the accent.
    let keyed: Bool
    let onKeyboard: () -> Void

    /// Which nodes this window opened and closed, as text.
    @SceneStorage private var expansionText: String
    @SceneStorage private var filterRaw: String
    @FocusState private var focused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(sidebar: OneTreeSidebar, filterText: String, keyed: Bool, onKeyboard: @escaping () -> Void = {}) {
        self.sidebar = sidebar
        self.filterText = filterText
        self.keyed = keyed
        self.onKeyboard = onKeyboard
        _expansionText = SceneStorage(wrappedValue: "", "oneTree.expansion.\(sidebar.key)")
        _filterRaw = SceneStorage(wrappedValue: OneTreeFilter.open.rawValue, "oneTree.filter.\(sidebar.key)")
    }

    private var filter: OneTreeFilter { OneTreeFilter(rawValue: filterRaw) ?? .open }
    private var expansion: OneTreeExpansion { OneTreeExpansion(encoded: expansionText) }
    private var narrowing: Bool { !BoardFilter.isEmpty(filterText) }

    var body: some View {
        let tree = OneTree.build(sidebar.input(filter))
        let groups = shown(tree)
        let rows = groups.flatMap { $0 }
        ScrollViewReader { scroller in
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    section(groups[0])
                    rule
                    filterPicker
                    section(groups[1])
                    if !groups[2].isEmpty {
                        rule
                        section(groups[2])
                    }
                }
                .padding(.horizontal, NavigatorGrid.edge)
                .padding(.top, NavigatorRhythm.band)
                .padding(.bottom, NavigatorRhythm.section)
                // Rows come, go and move on the shared spring; a row whose
                // words change washes. What arrives by opening a node is
                // told apart from what's new by the whole tree's signature.
                .listChanges(
                    rows.map { Self.change($0.node) }, arrivals: tree.allNodes.map(Self.change))
            }
            .scrollBounceBehavior(.basedOnSize)
            .onChange(of: sidebar.selected, initial: true) { _, target in
                reveal(target, in: tree)
                if let id = selectedRow(rows)?.id { withAnimation(nil) { scroller.scrollTo(id) } }
            }
        }
        .focusable()
        .focused($focused)
        .focusEffectDisabled()
        .onChange(of: focused) { _, now in if now { onKeyboard() } }
        .onKeyPress(.downArrow) { step(1, rows) }
        .onKeyPress(.upArrow) { step(-1, rows) }
        .onKeyPress(.rightArrow) { open(true, rows) }
        .onKeyPress(.leftArrow) { open(false, rows) }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("one-tree")
    }

    /// The three groups as drawn: places, tree, below; narrowed and all
    /// open while the filter field holds something.
    private func shown(_ tree: OneTree) -> [[OneTreeRow]] {
        [tree.places, tree.tree, tree.below].map { nodes in
            guard narrowing else { return OneTree.rows(nodes, expansion: expansion) }
            let kept = OneTree.narrowed(nodes, to: filterText)
            return OneTree.rows(kept, expansion: OneTree.allOpen(kept))
        }
    }

    private static func change(_ node: OneTreeNode) -> ListChangeRow {
        ListChangeRow(
            id: node.id, signature: [node.title, node.detail, node.also, "\(node.holdsAsk)"].joined(separator: "\u{1}"))
    }

    private func section(_ rows: [OneTreeRow]) -> some View {
        ForEach(rows) { row in
            OneTreeRowView(
                row: row, selected: isSelected(row.node), keyed: keyed,
                onToggle: { toggle(row.node) },
                onChoose: { choose(row.node) })
            .changeWashed(row.id)
            .id(row.id)
        }
    }

    /// Between the places and the tree, and over the checkout: the edge a
    /// group ends at, as the navigator's rules between panes are.
    private var rule: some View {
        Divider()  // style-exempt: the navigator's rule between groups, as NavigatorSplit's (ov-243)
            .padding(.vertical, NavigatorRhythm.rule)
    }

    /// Status, as a filter on the tree: Open, In Review, All.
    private var filterPicker: some View {
        Picker("Show", selection: Binding(get: { filter }, set: { filterRaw = $0.rawValue })) {
            ForEach(OneTreeFilter.allCases, id: \.self) { Text($0.title).tag($0) }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .controlSize(.small)
        .padding(.bottom, NavigatorRhythm.air)
        .accessibilityLabel("Show")
        .identified("one-tree-filter")
    }

    // MARK: Selection

    /// Whether `node` is what the window shows. Every copy of a lane under
    /// two cards is lit, so the two read as one lane.
    private func isSelected(_ node: OneTreeNode) -> Bool {
        guard let target = node.target, target == sidebar.selected else { return false }
        return true
    }

    private func selectedRow(_ rows: [OneTreeRow]) -> OneTreeRow? {
        rows.first { $0.id == sidebar.hint && isSelected($0.node) } ?? rows.first { isSelected($0.node) }
    }

    private func choose(_ node: OneTreeNode) {
        focused = true
        if node.target == nil {
            toggle(node)
        } else {
            sidebar.onChoose(node)
        }
    }

    private func toggle(_ node: OneTreeNode) {
        guard !narrowing else { return }
        var next = expansion
        next.toggle(node)
        withAnimation(BoardMotion.list(reduceMotion: reduceMotion)) { expansionText = next.encoded }
    }

    /// Open the nodes over `target`, so what's selected is in sight (as
    /// Xcode's Reveal in Project Navigator), unless one copy of it already is.
    private func reveal(_ target: OneTreeTarget?, in tree: OneTree) {
        guard let target, !narrowing else { return }
        let rows = OneTree.rows(tree.roots, expansion: expansion)
        if rows.contains(where: { $0.node.target == target }) { return }
        let ancestors = tree.ancestors(of: target, hint: sidebar.hint)
        guard !ancestors.isEmpty else { return }
        var next = expansion
        next.open(ancestors)
        expansionText = next.encoded
    }

    // MARK: Keys

    /// ↑ or ↓: the row above or below the one selected, chosen.
    private func step(_ by: Int, _ rows: [OneTreeRow]) -> KeyPress.Result {
        let walkable = rows.filter { $0.node.target != nil }
        guard !walkable.isEmpty else { return .ignored }
        let at = selectedRow(walkable).flatMap { row in walkable.firstIndex { $0.id == row.id } }
        let next = at.map { min(max($0 + by, 0), walkable.count - 1) } ?? (by > 0 ? 0 : walkable.count - 1)
        if next != at { sidebar.onChoose(walkable[next].node) }
        return .handled
    }

    /// → opens the selected row, ← closes it, as an outline's do.
    private func open(_ opening: Bool, _ rows: [OneTreeRow]) -> KeyPress.Result {
        guard let row = selectedRow(rows), row.node.hasChildren, row.expanded != opening else { return .ignored }
        toggle(row.node)
        return .handled
    }
}

/// One row of the tree: its disclosure, its glyph, its words, and its dot.
struct OneTreeRowView: View {
    let row: OneTreeRow
    let selected: Bool
    let keyed: Bool
    let onToggle: () -> Void
    let onChoose: () -> Void
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false

    /// How far each level sits in from the one over it.
    static let indent: CGFloat = 12
    /// The disclosure's cell, before the glyph.
    static let disclosure: CGFloat = 12

    private var node: OneTreeNode { row.node }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 0) {
            Color.clear.frame(width: CGFloat(row.depth) * Self.indent, height: 1)
            disclosure
            Image(systemName: node.glyph)
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(glyphStyle)
                .accessibilityHidden(true)
                .glyphColumn()
            VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                words
                if !node.caption.isEmpty {
                    Text(node.caption)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(SidebarInk.secondary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: SidebarGrid.gap)
            trailing
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .navigatorRow(selected: selected, keyed: keyed, leading: 0)
        .background {
            if hovering && !selected { RoundedRectangle.control.fill(Fill.hover).boxOutset() }
        }
        .contentShape(Rectangle())
        .onTapGesture(perform: onChoose)
        .onHover { hovering = $0 }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibility)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityAction(named: row.expanded ? "Collapse" : "Expand", onToggle)
        .identified("tree-\(node.id)")
    }

    @ViewBuilder private var disclosure: some View {
        if node.hasChildren {
            Button(action: onToggle) {
                Image(systemName: "chevron.right")
                    .font(.system(size: WorkspaceStyle.PaneText.minimum, weight: .semibold))
                    .foregroundStyle(SidebarInk.secondary)
                    .rotationEffect(.degrees(row.expanded ? 90 : 0))
                    .frame(width: Self.disclosure)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityHidden(true)
        } else {
            Color.clear.frame(width: Self.disclosure, height: 1)
        }
    }

    private var words: some View {
        HStack(alignment: .firstTextBaseline, spacing: SidebarGrid.gap / 2) {
            if !node.key.isEmpty {
                Text(node.key)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary, design: .monospaced))
                    .foregroundStyle(SidebarInk.secondary)
                    .fixedSize()
            }
            Text(node.title)
                .font(.system(size: WorkspaceStyle.PaneText.body, weight: node.kind == .theme ? .medium : .regular))
                .foregroundStyle(node.quiet ? AnyShapeStyle(SidebarInk.secondary) : AnyShapeStyle(.primary))
                .lineLimit(1)
                .truncationMode(.tail)
                .layoutPriority(1)
            if !node.also.isEmpty {
                Text(node.also)
                    .font(.system(size: WorkspaceStyle.PaneText.minimum))
                    .foregroundStyle(SidebarInk.secondary)
                    .lineLimit(1)
                    .fixedSize()
            }
        }
    }

    @ViewBuilder private var trailing: some View {
        HStack(alignment: .firstTextBaseline, spacing: SidebarGrid.gap / 2) {
            if row.dot {
                Circle()
                    .fill(Tint.attention(scheme))
                    .frame(width: 6, height: 6)
                    .alignmentGuide(.firstTextBaseline) { $0[.bottom] }
                    .accessibilityHidden(true)
                    .identified("tree-dot-\(node.id)")
            }
            if !node.detail.isEmpty {
                Text(node.detail)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary).monospacedDigit())
                    .foregroundStyle(detailStyle)
                    .lineLimit(1)
                    .fixedSize()
            }
        }
    }

    /// Color only for what needs attention: the Needs You count above zero.
    private var detailStyle: AnyShapeStyle {
        node.target == .needsYou ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(SidebarInk.secondary)
    }

    private var glyphStyle: AnyShapeStyle {
        if node.target == .needsYou && !node.detail.isEmpty { return AnyShapeStyle(Tint.attention(scheme)) }
        return AnyShapeStyle(SidebarInk.secondary)
    }

    private var accessibility: String {
        var parts = [node.key.isEmpty ? node.title : "\(node.key) \(node.title)"]
        if !node.detail.isEmpty { parts.append(node.detail) }
        if !node.also.isEmpty { parts.append(node.also) }
        if !node.caption.isEmpty { parts.append(node.caption) }
        if row.dot { parts.append(OneTreeWords.needsYou) }
        return parts.joined(separator: ", ")
    }
}
