import Foundation

// The Mac sidebar as two lists (ov-503, design 3): Themes › Cards (the why)
// and Lanes › Agents and terminals (the how), in place of `OneTree`'s single
// Theme › Task › Lane › Terminals outline. `ThemeTree` and `LaneTree`
// (PlanSidebarTrees.swift) decide what is in each; this lays them out as the
// same `OneTreeNode`s the navigator, its keyboard and its fold already draw,
// and keeps the join between them: choose a card and the lane working it is
// lit, choose a lane and its cards are.
//
// `OneTree` stays for the phones and for the jump bar's crumbs and ⌘↑, which
// still read the path Theme › Task › Lane › Terminal.

public struct SidebarOutline: Equatable, Sendable {
    /// Needs You and the Plan.
    public var places: [OneTreeNode]
    /// The themes and their cards, then the cards in no theme and the ones
    /// this version can't read.
    public var themes: [OneTreeNode]
    /// The lanes by train, review, building and waiting, with each lane's
    /// agents and terminals.
    public var lanes: [OneTreeNode]
    /// The main checkout and the loose worktrees.
    public var below: [OneTreeNode]
    /// The lane working each card, and the cards each lane works: the join.
    private var laneOfCard: [String: String]
    private var cardsOfLane: [String: [String]]

    /// Every root, top to bottom.
    public var roots: [OneTreeNode] { places + themes + lanes + below }

    public static func build(_ input: OneTreeInput) -> SidebarOutline { SidebarOutline(input) }

    public init(_ input: OneTreeInput) {
        let builder = OneTreeBuilder(input)
        let statuses = Dictionary(input.tasks.map { ($0.id, $0.status) }, uniquingKeysWith: { first, _ in first })
        let themeTree = ThemeTree(plan: input.plan, filter: input.filter, statuses: statuses)
        let laneTree = LaneTree(plan: input.plan)
        let planLanes = Dictionary(input.plan.lanes.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })

        var laneOfCard: [String: String] = [:]
        var themes = themeTree.themes.map { Self.themeNode($0, tree: themeTree, builder: builder, into: &laneOfCard) }
        themes += [builder.noThemeNode().map(Self.cardsOnly), builder.unreadableNode()].compactMap { $0 }

        var cardsOfLane: [String: [String]] = [:]
        var lanes = laneTree.groups.map {
            Self.groupNode($0, builder: builder, planLanes: planLanes, into: &cardsOfLane)
        }
        if let other = Self.otherWorktrees(builder: builder, input: input) { lanes.append(other) }

        self.places = builder.places()
        self.themes = themes
        self.lanes = lanes
        self.laneOfCard = laneOfCard
        self.cardsOfLane = cardsOfLane
        let reached = OneTreeBuilder.worktrees(in: themes + lanes)
        self.below = [builder.mainCheckoutNode(), builder.looseNode(reached: reached)].compactMap { $0 }
    }

    // MARK: Themes

    private static func themeNode(
        _ theme: ThemeTree.ThemeNode, tree: ThemeTree, builder: OneTreeBuilder, into laneOfCard: inout [String: String]
    ) -> OneTreeNode {
        let id = "theme:\(theme.id)"
        let state = builder.themes.first { $0.id == theme.id }?.state ?? "active"
        var live = false
        var children = theme.cards.map { card -> OneTreeNode in
            if let lane = card.laneID { laneOfCard[card.task] = lane }
            live = live || builder.lanes(of: card.task).contains { $0.state.isLive && $0.state != .queued }
            var node = OneTreeNode(
                id: "\(id)/task:\(card.task)", kind: .task, title: card.title, key: card.key,
                detail: card.lane ?? "", glyph: OneTreeGlyph.task(card.status), target: .task(card.task),
                quiet: card.status.isFinished)
            node.asks = builder.input.asks.tasks.contains(card.task)
            return builder.finish(node)
        }
        children += builder.input.pages.filter { $0.themeID == theme.id }.map { builder.pageNode($0, under: id) }
        var node = OneTreeNode(
            id: id, kind: .theme, title: theme.name, detail: theme.progress, glyph: OneTreeGlyph.theme(state: state),
            target: .theme(theme.id), children: children)
        node.asks = builder.input.asks.themes.contains(theme.id)
        node = builder.finish(node)
        node.expandedByDefault = live || node.holdsAsk
        return node
    }

    /// The old builder's No Theme group without the lanes under its cards:
    /// the lanes have their own list.
    private static func cardsOnly(_ node: OneTreeNode) -> OneTreeNode {
        var node = node
        node.children = node.children.map { child in
            var child = child
            if child.kind == .task { child.children = [] } else { child = cardsOnly(child) }
            return child
        }
        node.holdsAsk = node.asks || node.children.contains(where: \.holdsAsk)
        return node
    }

    // MARK: Lanes

    private static func groupNode(
        _ group: LaneTree.Group, builder: OneTreeBuilder, planLanes: [String: PlanLane],
        into cardsOfLane: inout [String: [String]]
    ) -> OneTreeNode {
        let id = "lanes:\(group.id)"
        var node = OneTreeNode(
            id: id, kind: .group, title: group.title, detail: group.detail ?? "", glyph: glyph(group.kind),
            children: group.lanes.map { laneNode($0, builder: builder, planLanes: planLanes, into: &cardsOfLane) })
        node.expandedByDefault = !group.folded
        node.quiet = group.kind == .onLandedTrain
        return builder.finish(node)
    }

    private static func glyph(_ kind: LaneTree.Kind) -> String {
        switch kind {
        case .train: OneTreeGlyph.train
        case .inReview: OneTreeWords.glyph(.review)
        case .building: OneTreeWords.glyph(.building)
        case .waiting: OneTreeWords.glyph(.queued)
        case .onLandedTrain: OneTreeWords.glyph(.landed)
        }
    }

    private static func laneNode(
        _ lane: LaneTree.LaneNode, builder: OneTreeBuilder, planLanes: [String: PlanLane],
        into cardsOfLane: inout [String: [String]]
    ) -> OneTreeNode {
        let id = "lane:\(lane.id)"
        let planLane = planLanes[lane.id]
        let worktree = builder.laneWorktrees[lane.id]
        cardsOfLane[lane.id] = lane.cards.map(\.task)
        // An agent that runs in one of the lane's panes opens it; the pane
        // itself isn't listed twice.
        let panes = Set(worktree?.terminals.map(\.id) ?? [])
        let agentPanes = Set(lane.agents.compactMap(\.terminal).filter(panes.contains))
        var children = worktree.map { builder.terminalNodes($0, under: id) } ?? []
        children.removeAll { node in
            if case .terminal(_, let pane)? = node.target { return agentPanes.contains(pane) }
            return false
        }
        let agents = lane.agents.map { agent -> OneTreeNode in
            let node = "\(id)/agent:\(agent.id)"
            if let worktree, let pane = agent.terminal, agentPanes.contains(pane) {
                var row = OneTreeNode(
                    id: node, kind: .terminal, title: agent.who, detail: agent.tokens ?? "",
                    glyph: OneTreeGlyph.agent(working: agent.working), target: .terminal(worktree: worktree.id, terminal: pane),
                    quiet: !agent.working)
                row.worktreeID = worktree.id
                row.asks = builder.input.asks.terminals.contains(pane)
                row.holdsAsk = row.asks
                return row
            }
            return OneTreeNode(
                id: node, kind: .subagent, title: agent.who, detail: agent.tokens ?? "",
                glyph: OneTreeGlyph.agent(working: agent.working), target: .orchestrator, quiet: !agent.working)
        }
        children = agents + children
        var node = OneTreeNode(
            id: id, kind: .lane, title: planLane.map(OneTreeBuilder.laneTitle) ?? lane.heading, slug: lane.name,
            detail: lane.state, caption: joined(lane.cards), glyph: planLane.map { OneTreeGlyph.lane($0.state) } ?? OneTreeGlyph.worktree,
            target: .lane(lane.id), children: children, quiet: !(planLane?.state.isLive ?? true))
        node.laneState = planLane?.state
        node.worktreeID = worktree?.id
        return builder.finish(node)
    }

    /// The cards a lane combines, said once the lane works more than one:
    /// "ov-487 ov-303", "ov-487 +2", a second line under the title. One card needs no words, since its title
    /// is the lane's.
    static func joined(_ cards: [PlanCardRef]) -> String {
        let keys = cards.map(\.key).filter { !$0.isEmpty }
        guard keys.count > 1 else { return "" }
        return keys.count == 2 ? keys.joined(separator: " ") : "\(keys[0]) +\(keys.count - 1)"
    }

    /// Worktrees a card is worked in with no plan lane recorded (a board with
    /// no plan, a card's own branch): lanes all the same, so their panes stay
    /// reachable.
    private static func otherWorktrees(builder: OneTreeBuilder, input: OneTreeInput) -> OneTreeNode? {
        let id = "lanes:other"
        var seen = builder.laneWorktreeIDs
        var rows: [OneTreeNode] = []
        for task in input.tasks where !task.status.isFinished {
            for worktree in builder.ownWorktrees(of: task) where seen.insert(worktree.id).inserted {
                rows.append(builder.worktreeLaneNode(worktree, for: task, under: id))
            }
        }
        guard !rows.isEmpty else { return nil }
        var node = OneTreeNode(
            id: id, kind: .group, title: OneTreeWords.withoutLane, detail: "\(rows.count)", glyph: OneTreeGlyph.worktree,
            children: rows)
        node.expandedByDefault = true
        return builder.finish(node)
    }

    // MARK: Reading it

    /// The ids of the nodes that go with `target`, to light beside the chosen
    /// one: a card's lane, a lane's cards. A lane's pane or worktree is its
    /// lane. Never the chosen node itself.
    public func related(to target: OneTreeTarget?) -> Set<String> {
        guard let target else { return [] }
        let nodes = (themes + lanes).flatMap(Self.flatten)
        func ids(_ wanted: OneTreeTarget) -> [String] { nodes.filter { $0.target == wanted }.map(\.id) }
        func ofLane(_ lane: String) -> Set<String> {
            Set((cardsOfLane[lane] ?? []).flatMap { ids(.task($0)) })
        }
        switch target {
        case .task(let task):
            guard let lane = laneOfCard[task] else { return [] }
            return Set(ids(.lane(lane)))
        case .lane(let lane):
            return ofLane(lane)
        case .worktree(let worktree), .terminal(let worktree, _):
            let owners = nodes.filter { $0.kind == .lane && $0.worktreeID == worktree }
            return owners.reduce(into: Set<String>()) { out, node in
                out.insert(node.id)
                if case .lane(let lane)? = node.target { out.formUnion(ofLane(lane)) }
            }
        default:
            return []
        }
    }

    private static func flatten(_ node: OneTreeNode) -> [OneTreeNode] { [node] + node.children.flatMap(flatten) }

    /// The outline as one tree, for the rules that read a path through it.
    private var flat: OneTree { OneTree(places: places, tree: themes + lanes, below: below) }

    /// The ids of the nodes above `target`'s: opened to reveal it.
    public func ancestors(of target: OneTreeTarget, hint: String? = nil) -> [String] {
        flat.ancestors(of: target, hint: hint)
    }

    /// The nodes beside the one with `id`, itself included.
    public func siblings(of id: String) -> [OneTreeNode] { flat.siblings(of: id) }

    /// Every node, depth first.
    public var allNodes: [OneTreeNode] { flat.allNodes }
}

extension OneTreeWords {
    /// Worktrees a card is worked in with no plan lane.
    public static let withoutLane = "Without a Lane"
}
