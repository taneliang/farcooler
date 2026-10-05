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
// there, as the Board view (View ▸ Tasks by Status, or ⌘K).

/// What the window hands the navigator to draw the tree.
struct OneTreeSidebar {
    /// The tree under the filter chosen: built once per change of what
    /// it's built from, not once per draw (review H2).
    var tree: (OneTreeFilter) -> OneTree
    /// What the window shows, as the node it is.
    var selected: OneTreeTarget?
    /// The row chosen last, by id: which copy of a lane under two cards.
    var hint: String?
    /// Where this window keeps the tree's choices: the runner and workspace.
    var key: String
    /// A row chosen: the window goes where it points.
    var onChoose: (OneTreeNode) -> Void
    /// The board and its plan are read: before that a lane's worktree
    /// looks loose, and revealing it would open the wrong group.
    /// Asked as the navigator draws, which watches the board and its plan.
    var settled: () -> Bool = { true }
    /// A row's context menu: a card's, a lane's or worktree's, a
    /// terminal's, the checkout's (review M2).
    var menu: (OneTreeNode) -> AnyView = { _ in AnyView(EmptyView()) }
}

/// The tree, drawn: the navigator's content in place of its sections.
struct OneTreeNavigator: View {
    let sidebar: OneTreeSidebar
    /// The navigator's filter field (⌘F).
    let filterText: String
    /// The navigator has the keyboard: a selected row reads in the accent.
    let keyed: Bool
    let onKeyboard: () -> Void
    /// Bumped when the window gives the navigator the keyboard: ⌥⌘2, or Esc
    /// in the filter field (review H1).
    var focusRequest = 0
    /// Return on the row already chosen: into it, as ⌥⌘3.
    var onEnter: () -> Void = {}

    /// Which nodes this window opened and closed, as text, and the filter:
    /// kept with the window's scene, and held here too, since a window
    /// hosted outside a scene (the capture harness's) keeps no scene storage.
    @SceneStorage private var keptExpansion: String
    @SceneStorage private var keptFilter: String
    @State private var heldExpansion: String?
    @State private var heldFilter: String?
    @FocusState private var focused: Bool
    /// The keyboard's row, while it's on one the window doesn't show (a
    /// group, the orchestrator's row): nil follows the selection.
    @State private var cursor: String?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(
        sidebar: OneTreeSidebar, filterText: String, keyed: Bool, onKeyboard: @escaping () -> Void = {},
        focusRequest: Int = 0, onEnter: @escaping () -> Void = {}
    ) {
        self.sidebar = sidebar
        self.filterText = filterText
        self.keyed = keyed
        self.onKeyboard = onKeyboard
        self.focusRequest = focusRequest
        self.onEnter = onEnter
        _keptExpansion = SceneStorage(wrappedValue: "", "oneTree.expansion.\(sidebar.key)")
        _keptFilter = SceneStorage(wrappedValue: OneTreeFilter.open.rawValue, "oneTree.filter.\(sidebar.key)")
    }

    private var expansionText: String {
        get { heldExpansion ?? keptExpansion }
        nonmutating set {
            heldExpansion = newValue
            keptExpansion = newValue
        }
    }

    private var filterRaw: String {
        get { heldFilter ?? keptFilter }
        nonmutating set {
            heldFilter = newValue
            keptFilter = newValue
        }
    }

    private var filter: OneTreeFilter { OneTreeFilter(rawValue: filterRaw) ?? .open }
    private var expansion: OneTreeExpansion { OneTreeExpansion(encoded: expansionText) }
    private var narrowing: Bool { !BoardFilter.isEmpty(filterText) }

    var body: some View {
        let tree = sidebar.tree(filter)
        let groups = shown(tree)
        let rows = groups.flatMap { $0 }
        ScrollViewReader { scroller in
            ScrollView {
                // Lazy: a long board draws the rows in sight (review H2).
                LazyVStack(alignment: .leading, spacing: 0) {
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
            // Again when the path to it appears: the window can open on a
            // terminal before the plan that puts its lane under a card is read.
            .onChange(of: RevealKey(target: sidebar.settled() ? sidebar.selected : nil, path: sidebar.selected.map { tree.ancestors(of: $0, hint: sidebar.hint) } ?? []), initial: true) { _, key in
                reveal(key.target, in: tree)
                if let id = selectedRow(rows)?.id { withAnimation(nil) { scroller.scrollTo(id) } }
            }
        }
        .focusable()
        .focused($focused)
        .focusEffectDisabled()
        .onChange(of: focused) { _, now in if now { onKeyboard() } }
        .onChange(of: focusRequest) { _, _ in focused = true }
        // The window's choice moved: the keyboard follows it.
        .onChange(of: sidebar.selected) { _, _ in cursor = nil }
        // The default open theme, taken once the board and plan are read,
        // and kept for this window (review M1).
        .onChange(of: sidebar.settled(), initial: true) { _, settled in
            guard settled, !narrowing, !expansion.isSeeded else { return }
            var next = expansion
            next.seed(from: tree.roots)
            expansionText = next.encoded
        }
        .onKeyPress(.downArrow) { apply(OneTreeKeys.step(rows, from: cursorRow(rows), by: 1), rows, arriving: true) }
        .onKeyPress(.upArrow) { apply(OneTreeKeys.step(rows, from: cursorRow(rows), by: -1), rows, arriving: true) }
        .onKeyPress(.rightArrow) { apply(OneTreeKeys.right(rows, cursor: cursorRow(rows)), rows) }
        .onKeyPress(.leftArrow) { apply(OneTreeKeys.left(rows, cursor: cursorRow(rows)), rows) }
        .onKeyPress(.return) { apply(OneTreeKeys.enter(rows, cursor: cursorRow(rows)), rows) }
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
                row: row, selected: isSelected(row.node) || (cursor == row.id && focused), keyed: keyed,
                onToggle: { toggle(row.node) },
                onChoose: {
                    cursor = row.node.target == nil ? row.id : nil
                    choose(row.node)
                },
                menu: { sidebar.menu(row.node) })
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

    /// Status, as a filter on the tree: Not Done, In Review, All.
    private var filterPicker: some View {
        Picker("Show", selection: Binding(get: { filter }, set: { filterRaw = $0.rawValue })) {
            ForEach(OneTreeFilter.allCases, id: \.self) { Text($0.title).tag($0).help($0.help) }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .controlSize(.small)
        .padding(.bottom, NavigatorRhythm.air)
        .accessibilityLabel("Show")
        .help(OneTreeFilter.allHelp)
        .identified("one-tree-filter")
    }

    // MARK: Selection

    /// Whether `node` is what the window shows. Every copy of a lane under
    /// two cards is lit, so the two read as one lane.
    private func isSelected(_ node: OneTreeNode) -> Bool {
        node.stands(for: sidebar.selected)
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
        if rows.contains(where: { $0.node.stands(for: target) }) { return }
        let ancestors = tree.ancestors(of: target, hint: sidebar.hint)
        guard !ancestors.isEmpty else { return }
        var next = expansion
        next.open(ancestors)
        expansionText = next.encoded
    }

    // MARK: Keys

    /// The keyboard's row: the cursor, else the row the window shows.
    private func cursorRow(_ rows: [OneTreeRow]) -> String? {
        if let cursor, rows.contains(where: { $0.id == cursor }) { return cursor }
        return selectedRow(rows)?.id
    }

    /// Carry out what a key does (`OneTreeKeys`). Arriving on a row by ↑
    /// or ↓ goes where it points, as a list's selection does, unless that
    /// would take the keyboard away (the orchestrator) or there's nowhere.
    private func apply(_ move: OneTreeKeys.Move, _ rows: [OneTreeRow], arriving: Bool = false) -> KeyPress.Result {
        let node = { (id: String) in rows.first { $0.id == id }?.node }
        switch move {
        case .none:
            return .ignored
        case .cursor(let id):
            cursor = id
            if arriving, let found = node(id), OneTreeKeys.choosesOnArrival(found) {
                sidebar.onChoose(found)
            }
        case .toggle(let id):
            if let found = node(id) { toggle(found) }
        case .choose(let id):
            guard let found = node(id) else { return .ignored }
            if isSelected(found) && found.target != .orchestrator { onEnter() } else { sidebar.onChoose(found) }
        }
        return .handled
    }
}

/// What a reveal waits on: the selection, and the path the tree has to it.
private struct RevealKey: Equatable {
    var target: OneTreeTarget?
    var path: [String]
}

/// One row of the tree: its disclosure, its glyph, its words, and its dot.
struct OneTreeRowView: View {
    let row: OneTreeRow
    let selected: Bool
    let keyed: Bool
    let onToggle: () -> Void
    let onChoose: () -> Void
    var menu: () -> AnyView = { AnyView(EmptyView()) }
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
        .help(ifAny: node.kind == .page ? OneTreeWords.pageHelp : nil)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibility)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityAction(named: row.expanded ? "Collapse" : "Expand", onToggle)
        // Its open state and its level, which the drawing says (review M6).
        .accessibilityValue(node.hasChildren ? (row.expanded ? "Expanded" : "Collapsed") : "")
        .accessibilityHint("Level \(row.depth + 1)")
        .contextMenu { menu() }
        .identified("tree-\(node.id)")
    }

    @ViewBuilder private var disclosure: some View {
        if node.hasChildren {
            // The app's one disclosure (ov-101), on the shared motion.
            DisclosureButton(
                expanded: row.expanded, accessibilityLabel: node.title, width: Self.disclosure, action: onToggle)
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
