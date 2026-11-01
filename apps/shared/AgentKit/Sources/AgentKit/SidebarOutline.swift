import Foundation

// The Mac sidebar as two lists (ov-503, design 3): Themes › Cards (the why)
// and Tickets › Agents and terminals (the how), in place of `OneTree`'s single
// Theme › Task › Ticket › Terminals outline. `ThemeTree` and `TicketTree`
// (PlanSidebarTrees.swift) decide what is in each; this lays them out as the
// same `OneTreeNode`s the navigator, its keyboard and its fold already draw,
// and keeps the join between them: choose a card and its ticket is lit,
// choose a ticket and its card is.
//
// `OneTree` stays for the phones. The Mac's jump bar and ⌘↑ walk this outline
// (ov-512), so the crumbs read what the sidebar shows: Theme › Card, and
// Group › Ticket › Agent or Terminal.

public struct SidebarOutline: Equatable, Sendable {
    /// Needs You and the Plan.
    public var places: [OneTreeNode]
    /// The themes and their cards, then the cards in no theme and the ones
    /// this version can't read.
    public var themes: [OneTreeNode]
    /// The tickets by landing, review, building and waiting, with each
    /// ticket's agents and terminals.
    public var tickets: [OneTreeNode]
    /// The main checkout and the loose worktrees.
    public var below: [OneTreeNode]
    /// The ticket each card is, and the cards each ticket is: the join.
    private var ticketOfCard: [String: String]
    private var cardsOfTicket: [String: [String]]

    /// Every root, top to bottom.
    public var roots: [OneTreeNode] { places + themes + tickets + below }

    public static func build(_ input: OneTreeInput) -> SidebarOutline { SidebarOutline(input) }

    public init(_ input: OneTreeInput) {
        let builder = OneTreeBuilder(input)
        let statuses = Dictionary(input.tasks.map { ($0.id, $0.status) }, uniquingKeysWith: { first, _ in first })
        let themeTree = ThemeTree(plan: input.plan, filter: input.filter, statuses: statuses)
        let ticketTree = TicketTree(plan: input.plan)
        let planLines = Dictionary(input.plan.lines.map { ($0.task, $0) }, uniquingKeysWith: { first, _ in first })

        var ticketOfCard: [String: String] = [:]
        var themes = themeTree.themes.map { Self.themeNode($0, tree: themeTree, builder: builder, into: &ticketOfCard) }
        themes += [builder.noThemeNode().map(Self.cardsOnly), builder.unreadableNode()].compactMap { $0 }

        var cardsOfTicket: [String: [String]] = [:]
        var tickets = ticketTree.groups.map {
            Self.groupNode($0, builder: builder, planLines: planLines, into: &cardsOfTicket)
        }
        if let other = Self.otherWorktrees(builder: builder, input: input) { tickets.append(other) }

        self.places = builder.places()
        self.themes = themes
        self.tickets = tickets
        self.ticketOfCard = ticketOfCard
        self.cardsOfTicket = cardsOfTicket
        let reached = OneTreeBuilder.worktrees(in: themes + tickets)
        self.below = [builder.mainCheckoutNode(), builder.looseNode(reached: reached)].compactMap { $0 }
    }

    // MARK: Themes

    private static func themeNode(
        _ theme: ThemeTree.ThemeNode, tree: ThemeTree, builder: OneTreeBuilder, into ticketOfCard: inout [String: String]
    ) -> OneTreeNode {
        let id = "theme:\(theme.id)"
        let state = builder.themes.first { $0.id == theme.id }?.state ?? "active"
        var live = false
        var children = theme.cards.map { card -> OneTreeNode in
            if let ticket = card.ticketID { ticketOfCard[card.task] = ticket }
            live = live || (builder.line(of: card.task)?.stage.isPastWaiting ?? false)
            var node = OneTreeNode(
                id: "\(id)/task:\(card.task)", kind: .task, title: card.title, key: card.key,
                detail: card.stage ?? "", glyph: OneTreeGlyph.task(card.status), target: .task(card.task),
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

    /// The old builder's No Theme group without the tickets under its cards:
    /// the tickets have their own list.
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

    // MARK: Tickets

    private static func groupNode(
        _ group: TicketTree.Group, builder: OneTreeBuilder, planLines: [String: PlanTicketLine],
        into cardsOfTicket: inout [String: [String]]
    ) -> OneTreeNode {
        let id = "tickets:\(group.id)"
        // Landing holds its landings, each "Landing N" with its title under it
        // and its tickets inside; every other group holds tickets.
        let landings = group.landings.map { landing -> OneTreeNode in
            var row = OneTreeNode(
                // The phase leads the second line, not the trailing edge: a long one ("Waiting for the Earlier
                // Landing") would crush "Landing 90" in a narrow sidebar, and the title can truncate instead.
                id: "\(id)/landing:\(landing.id)", kind: .group, title: landing.label, detail: "",
                caption: [landing.phase, landing.subtitle].compactMap { $0 }.joined(separator: " \u{00B7} "),
                glyph: OneTreeGlyph.landing,
                children: landing.tickets.map { ticketNode($0, builder: builder, planLines: planLines, into: &cardsOfTicket) })
            row.expandedByDefault = true
            return builder.finish(row)
        }
        var node = OneTreeNode(
            id: id, kind: .group, title: group.title, detail: group.detail ?? "", glyph: glyph(group.kind),
            children: landings + group.tickets.map { ticketNode($0, builder: builder, planLines: planLines, into: &cardsOfTicket) })
        node.expandedByDefault = !group.folded
        node.quiet = group.kind == .onLandedLanding
        return builder.finish(node)
    }

    private static func glyph(_ kind: TicketTree.Kind) -> String {
        switch kind {
        case .landing: OneTreeGlyph.landing
        case .inReview: OneTreeWords.glyph(.reviewing)
        case .building: OneTreeWords.glyph(.building)
        case .waiting: OneTreeWords.glyph(.waiting)
        case .onLandedLanding: OneTreeWords.glyph(.landed)
        }
    }

    private static func ticketNode(
        _ ticket: TicketTree.TicketNode, builder: OneTreeBuilder, planLines: [String: PlanTicketLine],
        into cardsOfTicket: inout [String: [String]]
    ) -> OneTreeNode {
        let id = "ticket:\(ticket.id)"
        let line = planLines[ticket.id]
        let worktree = builder.lineWorktrees[ticket.id]
        cardsOfTicket[ticket.id] = ticket.cards.map(\.task)
        // An agent that runs in one of the ticket's panes opens it; the pane
        // itself isn't listed twice.
        let panes = Set(worktree?.terminals.map(\.id) ?? [])
        let agentPanes = Set(ticket.agents.compactMap(\.terminal).filter(panes.contains))
        var children = worktree.map { builder.terminalNodes($0, under: id) } ?? []
        children.removeAll { node in
            if case .terminal(_, let pane)? = node.target { return agentPanes.contains(pane) }
            return false
        }
        let agents = ticket.agents.map { agent -> OneTreeNode in
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
            id: id, kind: .ticket, title: ticket.heading, key: ticket.key, slug: ticket.key,
            detail: ticket.state, glyph: line.map { OneTreeGlyph.ticket($0.stage) } ?? OneTreeGlyph.worktree,
            target: .ticket(ticket.id), children: children, quiet: !(line?.stage.isLive ?? true))
        node.ticketStage = line?.stage
        node.worktreeID = worktree?.id
        return builder.finish(node)
    }

    /// Worktrees a card is worked in with no ticket recorded (a board with no
    /// plan, a card's own branch): tickets all the same, so their panes stay
    /// reachable.
    private static func otherWorktrees(builder: OneTreeBuilder, input: OneTreeInput) -> OneTreeNode? {
        let id = "tickets:other"
        var seen = builder.lineWorktreeIDs
        var rows: [OneTreeNode] = []
        for task in input.tasks where !task.status.isFinished {
            for worktree in builder.ownWorktrees(of: task) where seen.insert(worktree.id).inserted {
                rows.append(builder.worktreeTicketNode(worktree, for: task, under: id))
            }
        }
        guard !rows.isEmpty else { return nil }
        var node = OneTreeNode(
            id: id, kind: .group, title: OneTreeWords.offTheLine, detail: "\(rows.count)", glyph: OneTreeGlyph.worktree,
            children: rows)
        node.expandedByDefault = true
        return builder.finish(node)
    }

    // MARK: Reading it

    /// The ids of the nodes that go with `target`, to light beside the chosen
    /// one: a card's ticket, a ticket's card. A ticket's pane or worktree is
    /// its ticket. Never the chosen node itself.
    public func related(to target: OneTreeTarget?) -> Set<String> {
        guard let target else { return [] }
        let nodes = (themes + tickets).flatMap(Self.flatten)
        func ids(_ wanted: OneTreeTarget) -> [String] { nodes.filter { $0.target == wanted }.map(\.id) }
        func ofTicket(_ ticket: String) -> Set<String> {
            Set((cardsOfTicket[ticket] ?? []).flatMap { ids(.task($0)) })
        }
        switch target {
        case .task(let task):
            guard let ticket = ticketOfCard[task] else { return [] }
            return Set(ids(.ticket(ticket)))
        case .ticket(let ticket):
            return ofTicket(ticket)
        case .worktree(let worktree), .terminal(let worktree, _):
            let owners = nodes.filter { $0.kind == .ticket && $0.worktreeID == worktree }
            return owners.reduce(into: Set<String>()) { out, node in
                out.insert(node.id)
                if case .ticket(let ticket)? = node.target { out.formUnion(ofTicket(ticket)) }
            }
        default:
            return []
        }
    }

    private static func flatten(_ node: OneTreeNode) -> [OneTreeNode] { [node] + node.children.flatMap(flatten) }

    /// The outline as one tree, for the rules that read a path through it.
    private var flat: OneTree { OneTree(places: places, tree: themes + tickets, below: below) }

    /// The ids of the nodes above `target`'s: opened to reveal it.
    public func ancestors(of target: OneTreeTarget, hint: String? = nil) -> [String] {
        flat.ancestors(of: target, hint: hint)
    }

    /// The nodes beside the one with `id`, itself included: in its own list,
    /// so a ticket group's are the other ticket groups, not the themes above them
    /// (ov-512).
    public func siblings(of id: String) -> [OneTreeNode] {
        for list in [places, themes, tickets, below] {
            let found = OneTree(places: [], tree: list, below: []).siblings(of: id)
            if !found.isEmpty { return found }
        }
        return []
    }

    // MARK: The path (ov-512)

    /// A ticket group's section lives on the Plan's home, so its crumb goes
    /// there. Any other group (No Theme, Main Checkout) goes nowhere.
    private static func goes(_ node: OneTreeNode) -> OneTreeTarget? {
        node.target ?? (node.kind == .group && node.id.hasPrefix("tickets:") ? .plan : nil)
    }

    /// Where choosing `node` as a crumb or a menu row goes, or nil for one
    /// that goes nowhere.
    public static func destination(of node: OneTreeNode) -> OneTreeTarget? { goes(node) }

    /// The nodes the jump bar names for `target`, from the first under the
    /// pinned places down: the sidebar's own rows, so its crumbs and its rows
    /// agree. `hint` picks the row when a target has two (a card under two
    /// themes).
    public func path(to target: OneTreeTarget, hint: String? = nil) -> [OneTreeNode] {
        (flat.path(to: target, hint: hint) ?? []).filter { $0.kind != .place && $0.kind != .doneFold }
    }

    /// The breadcrumb for `target`, one entry for each node of `path`.
    public func crumbs(to target: OneTreeTarget, hint: String? = nil)
        -> [(title: String, target: OneTreeTarget?, glyph: String)]
    {
        path(to: target, hint: hint).map { node in
            (node.key.isEmpty ? node.title : "\(node.key) \(node.title)", Self.goes(node), node.glyph)
        }
    }

    /// Where ⌘↑ goes: the target and the sidebar row it means.
    public struct Up: Equatable, Sendable {
        public var target: OneTreeTarget
        /// The row to light, where the target alone doesn't say: a ticket
        /// group's, whose section is on the Plan's home.
        public var hint: String?
    }

    /// ⌘↑: the node over `target`'s in this outline. A ticket's is its group;
    /// a card's, its theme; an agent's, its ticket. Failing that the nearest
    /// node that goes somewhere, or the Plan at the top.
    public func parent(of target: OneTreeTarget, hint: String? = nil) -> Up? {
        guard let nodes = flat.path(to: target, hint: hint) else { return nil }
        for node in nodes.dropLast().reversed() {
            if let goes = Self.goes(node) { return Up(target: goes, hint: node.id) }
        }
        return target == .plan ? nil : Up(target: .plan, hint: nil)
    }

    /// Every node, depth first.
    public var allNodes: [OneTreeNode] { flat.allNodes }
}

extension OneTreeWords {
    /// Cards in progress or in review that are not on the line, and worktrees
    /// with no ticket.
    public static let offTheLine = "Off the Line"
}
