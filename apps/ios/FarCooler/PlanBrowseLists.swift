import SwiftUI

// Themes and Lanes as pushed lists (ov-503, design 7): a phone has no
// sidebar, so the two lists of the iPad's (`SidebarOutline`) are two rows at
// the foot of the Plan, each pushing its list. Themes is the why, its cards
// under each theme; Lanes is the how, grouped as the Plan's sections are, each
// lane's agents under it. The nodes are AgentKit's; this draws rows and sends
// a tap to the phone's stack.

struct PlanBrowseList: View {
    enum Kind { case themes, lanes }

    let hook: PlanBoardHook
    let kind: Kind
    @ObservedObject private var reads: PlanReads
    @State private var filter: OneTreeFilter
    /// The folded groups (Waiting, On a Landed Train) opened here.
    @State private var opened: Set<String> = []

    init(hook: PlanBoardHook, kind: Kind) {
        self.hook = hook
        self.kind = kind
        reads = hook.reads
        _filter = State(initialValue: PhoneTreeFilter.remembered(hook.place))
    }

    private var outline: SidebarOutline? { reads.state(hook.summary.id) == nil ? nil : hook.outline?(filter) }

    var body: some View {
        let nodes = outline.map { kind == .themes ? $0.themes : $0.lanes } ?? []
        List {
            if nodes.isEmpty {
                Text(kind == .themes ? "No themes yet." : "No lanes yet.")
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("plan-browse-empty")
            }
            ForEach(nodes) { node in
                Section {
                    if !folded(node) { ForEach(node.children) { row($0, depth: 0) } }
                } header: {
                    header(node)
                }
            }
        }
        .listStyle(.insetGrouped)
        .navigationTitle(kind == .themes ? "Themes" : "Lanes")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            // The filter narrows the cards under Themes only.
            if kind == .themes {
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        Picker("Show", selection: $filter) {
                            ForEach(OneTreeFilter.allCases, id: \.self) { Text($0.title).tag($0) }
                        }
                    } label: {
                        Label("Show: \(filter.title)", systemImage: "line.3.horizontal.decrease.circle")
                    }
                    .accessibilityIdentifier("plan-browse-filter")
                }
            }
        }
        .onChange(of: filter) { _, chosen in PhoneTreeFilter.remember(chosen, for: hook.place) }
        .refreshable { await hook.read() }
        .accessibilityIdentifier(kind == .themes ? "plan-browse-themes-list" : "plan-browse-lanes-list")
    }

    /// A group that starts folded and hasn't been opened.
    private func folded(_ node: OneTreeNode) -> Bool {
        node.kind == .group && !node.expandedByDefault && !opened.contains(node.id)
    }

    // MARK: Headers

    /// A theme or a group: its title and its trailing progress or count. A
    /// theme's header opens its page.
    @ViewBuilder
    private func header(_ node: OneTreeNode) -> some View {
        let label = HStack(spacing: PaneMetrics.step) {
            Text(node.title).lineLimit(2).multilineTextAlignment(.leading)
            Spacer(minLength: 4)
            if !node.detail.isEmpty {
                Text(node.detail).font(.subheadline.monospacedDigit()).foregroundStyle(.secondary)
            }
            if node.target != nil || (node.kind == .group && !node.expandedByDefault) {
                Image(systemName: "chevron.forward")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .rotationEffect(.degrees(node.target == nil && !folded(node) ? 90 : 0))
            }
        }
        .textCase(nil)
        .font(.subheadline.weight(.semibold))
        .foregroundStyle(.secondary)
        // Not a Button: a Button in a section's header takes no tap in an inset list.
        if let target = node.target {
            label
                .contentShape(.rect)
                .onTapGesture { open(target) }
                .accessibilityAddTraits(.isButton)
                .accessibilityIdentifier("plan-browse-\(node.rowName)")
        } else if node.kind == .group && !node.expandedByDefault {
            label
                .contentShape(.rect)
                .onTapGesture { withAnimation { if !opened.insert(node.id).inserted { opened.remove(node.id) } } }
                .accessibilityAddTraits(.isButton)
                .accessibilityValue(folded(node) ? "Collapsed" : "Expanded")
                .accessibilityIdentifier("plan-browse-\(node.rowName)")
        } else {
            label.accessibilityIdentifier("plan-browse-\(node.rowName)")
        }
    }

    // MARK: Rows

    private func row(_ node: OneTreeNode, depth: Int) -> AnyView {
        AnyView(
            Group {
                PlanBrowseRow(node: node, depth: depth, open: node.target.map { target in { open(target) } })
                ForEach(node.children) { row($0, depth: depth + 1) }
            })
    }

    private func open(_ target: OneTreeTarget) {
        switch target {
        case .theme(let id): hook.onOpen(.theme(id))
        case .lane(let id): hook.onOpen(.lane(id))
        case .page(let slot): hook.onOpen(.page(slot))
        default:
            if let route = PhoneTree.route(target, in: hook.place) { hook.onRoute(route) }
        }
    }
}

/// One row of a pushed list: a card, a lane, an agent or a pane. The card's
/// key and title, the lane working it (or a lane's state and cards), and the
/// amber dot when it asks for you.
struct PlanBrowseRow: View {
    let node: OneTreeNode
    let depth: Int
    let open: (() -> Void)?
    @Environment(\.colorScheme) private var scheme
    @ScaledMetric(relativeTo: .body) private var glyphWidth: CGFloat = 22

    var body: some View {
        Group {
            if let open {
                Button(action: open) { content(goes: true) }.buttonStyle(.plain)
            } else {
                content(goes: false)
            }
        }
        .accessibilityIdentifier("plan-browse-row-\(node.rowName)")
    }

    private var isAgent: Bool { node.kind == .subagent || (node.kind == .terminal && depth > 0) }

    private func content(goes: Bool) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: node.glyph)
                .foregroundStyle(.secondary)
                .frame(width: glyphWidth)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    if !node.key.isEmpty {
                        Text(node.key).font(.subheadline.monospaced()).foregroundStyle(.secondary)
                    }
                    Text(node.title)
                        .font(isAgent ? .subheadline : .body)
                        .foregroundStyle(node.quiet ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
                        .lineLimit(2)
                }
                if let second = second {
                    Text(second).font(.footnote).foregroundStyle(.secondary).lineLimit(2)
                }
            }
            Spacer(minLength: 4)
            if node.holdsAsk {
                Circle().fill(Tint.attention(scheme)).frame(width: 8, height: 8)
                    .accessibilityLabel(OneTreeWords.needsYou)
                    .accessibilityIdentifier("plan-browse-dot")
            }
            if isAgent, !node.detail.isEmpty {
                Text(node.detail).font(.footnote.monospacedDigit()).foregroundStyle(.secondary).fixedSize()
            }
            if goes {
                Image(systemName: "chevron.forward")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
        }
        .padding(.leading, depth > 0 ? CGFloat(depth) * 18 : 0)
        .contentShape(.rect)
        .accessibilityElement(children: .combine)
    }

    /// A card says the lane working it; a lane, its state and the cards it
    /// works.
    private var second: String? {
        guard !isAgent else { return nil }
        let words = [node.detail, node.caption].filter { !$0.isEmpty }.joined(separator: " \u{00B7} ")
        return words.isEmpty ? nil : words
    }
}
