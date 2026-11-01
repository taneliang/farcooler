import Foundation

// The two-list sidebar (ov-503, design 3): the *why* and the *how*, apart.
// `ThemeTree` is Themes › Cards; `TicketTree` is Tickets › Agents, grouped as
// the Plan view's sections are, so the two read in one order. They replace
// `OneTree`'s single Theme › Task › Ticket › Terminals outline. Terminals are
// the app's own (they are live panes, not plan data) and hang under a ticket
// node in the view.
//
// A card on the line is one ticket, so the join is the card's own id:
// `ThemeTree.ticket(of:)` and `TicketTree.cards(of:)` answer both ways, and a
// view highlights the other side when one is chosen.

public struct ThemeTree: Equatable, Sendable {
    public struct CardNode: Equatable, Identifiable, Sendable {
        public var task: String
        public var key: String
        public var title: String
        public var status: TaskStatus
        public var glyph: PlanCardGlyph { PlanCardGlyph(status) }
        /// Its stage on the line while it is live, shown as the trailing
        /// detail.
        public var stage: String?
        /// Its ticket's id (the card's own) while it is on the line.
        public var ticketID: String?
        public var id: String { task }
    }

    public struct ThemeNode: Equatable, Identifiable, Sendable {
        public var id: String
        public var name: String
        public var done: Int
        public var total: Int
        public var cards: [CardNode]
        /// "6/10", the sidebar's short form.
        public var progress: String { "\(done)/\(total)" }
    }

    public var filter: OneTreeFilter
    public var themes: [ThemeNode]

    /// Themes in the plan's order (active, paused, done), each with the cards
    /// `filter` shows. The filter applies to cards, and a theme with none
    /// left isn't listed, except that a theme with no cards at all stays under
    /// Not Done and All, so a new one can be found.
    public init(plan: PlanModel, filter: OneTreeFilter = .open, statuses: [String: TaskStatus] = [:]) {
        self.filter = filter
        themes = plan.shownThemes.compactMap { theme in
            let cards: [CardNode] = theme.cards.compactMap { ref in
                let known = plan.card(ref.task)
                guard let status = statuses[ref.task] ?? known.flatMap({ OneTreeTask.status(word: $0.status) }) else { return nil }
                let shown =
                    switch filter {
                    case .open: status != .done && status != .cancelled
                    case .inReview: status == .inReview
                    case .all: true
                    }
                guard shown else { return nil }
                let line = plan.liveLine(ofTask: ref.task)
                return CardNode(
                    task: ref.task, key: ref.key, title: known?.title ?? "", status: status,
                    stage: line.map { $0.isFixing ? "Fixing" : PlanWords.stage($0.stage) }, ticketID: line?.task)
            }
            if cards.isEmpty {
                guard theme.cards.isEmpty, filter != .inReview, theme.state == "active" else { return nil }
            }
            return ThemeNode(
                id: theme.id, name: theme.name, done: theme.counts.done, total: PlanWords.total(theme.counts), cards: cards)
        }
    }

    /// A card's ticket, by the card's task id, or nil.
    public func ticket(of task: String) -> String? {
        for theme in themes { if let card = theme.cards.first(where: { $0.task == task }) { return card.ticketID } }
        return nil
    }

    /// The cards a ticket is, by its id, across themes.
    public func cards(of ticket: String) -> [CardNode] {
        themes.flatMap(\.cards).filter { $0.ticketID == ticket }
    }
}

extension PlanModel {
    /// The live ticket for a card on the line. Nil for a card whose ticket has
    /// landed or was dropped.
    func liveLine(ofTask task: String) -> PlanTicketLine? {
        lines.first { $0.task == task && $0.stage.isLive }
    }
}

public struct TicketTree: Equatable, Sendable {
    public struct AgentNode: Equatable, Identifiable, Sendable {
        public var id: String
        /// "Builder · Opus".
        public var who: String
        public var working: Bool
        /// "1.2M"; nil when no turn stated any.
        public var tokens: String?
        /// The pane it runs in, when it has one: the row opens it.
        public var terminal: String? = nil
    }

    public struct TicketNode: Equatable, Identifiable, Sendable {
        /// The card's id.
        public var id: String
        public var key: String
        public var heading: String
        /// The stage beside it: on a landing, only what matters ("Needs you",
        /// "Landed · 3f78754f"), and nothing for a ticket that is simply
        /// aboard; off one, its stage.
        public var state: String
        public var agents: [AgentNode]
        public var cards: [PlanCardRef]
    }

    /// A landing inside the Landing group: "Landing 86", what it is for, where
    /// it stands, and its tickets (ov-522).
    public struct LandingNode: Equatable, Identifiable, Sendable {
        public var id: String
        /// "Landing 86".
        public var label: String
        /// Its title, under the label; nil when it has none.
        public var subtitle: String?
        /// "Checking on CI", the ladder step it stands on.
        public var phase: String
        public var tickets: [TicketNode]
    }

    public enum Kind: Equatable, Sendable {
        case landing, inReview, building, waiting, onLandedLanding
    }

    public struct Group: Equatable, Identifiable, Sendable {
        public var kind: Kind
        public var title: String
        /// A count for a folded group.
        public var detail: String?
        /// Waiting starts folded.
        public var folded: Bool
        public var tickets: [TicketNode]
        /// The landings of the Landing group, each with its tickets inside it;
        /// empty for every other group.
        public var landings: [LandingNode] = []
        public var id: String {
            switch kind {
            case .landing: "landing"
            case .inReview: "inReview"
            case .building: "building"
            case .waiting: "waiting"
            case .onLandedLanding: "onLandedLanding"
            }
        }
    }

    /// Landing (its landings inside it) first, then In Review, Building and
    /// the folded Waiting; a group with no tickets isn't listed.
    public var groups: [Group]

    public init(plan: PlanModel, sections: PlanSections? = nil, locale: Locale = .current) {
        let sections = sections ?? PlanSections(plan: plan)
        func node(_ agent: PlanAgent, _ locale: Locale) -> AgentNode {
            AgentNode(
                id: agent.id, who: agent.who, working: agent.working, tokens: agent.tokens(locale: locale),
                terminal: agent.terminal)
        }
        func agents(_ line: PlanTicketLine) -> [AgentNode] {
            let mine = plan.agents.filter { agent in agent.role != .orchestrator && agent.tickets.contains { $0.task == line.task } }
            return PlanFleet(mine).agents.map { node($0, locale) }
        }
        func ticketNode(_ line: PlanTicketLine, state: String) -> TicketNode {
            TicketNode(id: line.task, key: line.key, heading: line.heading, state: state, agents: agents(line), cards: [line.card])
        }
        func rows(_ rows: [PlanSections.TicketRow]) -> [TicketNode] { rows.map { ticketNode($0.line, state: PlanWords.status($0.line)) } }

        var groups: [Group] = []
        if !sections.landings.isEmpty {
            groups.append(
                Group(
                    kind: .landing, title: PlanSections.Kind.landing.title, detail: nil, folded: false, tickets: [],
                    landings: sections.landings.map { card in
                        LandingNode(
                            id: card.landing.id, label: card.landing.label, subtitle: card.landing.subtitle, phase: card.phase.word,
                            tickets: card.tickets.map { ticketNode($0.line, state: Self.aboard($0.note)) })
                    }))
        }
        let waiting = sections.waiting.onYou + sections.waiting.next + sections.waiting.notRanked
        for (kind, title, list, folded) in [
            (Kind.inReview, "In Review", sections.inReview, false), (.building, "Building", sections.building, false),
            (.waiting, "Waiting", waiting, true), (.onLandedLanding, "Waiting for the Next Landing", sections.onLandedLanding, true),
        ] where !list.isEmpty {
            groups.append(
                Group(kind: kind, title: title, detail: folded ? "\(list.count)" : nil, folded: folded, tickets: rows(list)))
        }
        self.groups = groups
    }

    /// What a ticket aboard says beside its title: its note, but "Landed"
    /// without the commit, which would take the title's room (the landing card
    /// has it).
    static func aboard(_ note: String?) -> String {
        guard let note else { return "" }
        return note.hasPrefix("Landed") ? "Landed" : note
    }

    /// The cards a ticket is, by the ticket's id: what a view highlights when
    /// a ticket is chosen.
    public func cards(of ticket: String) -> [PlanCardRef] {
        groups.lazy.flatMap { $0.tickets + $0.landings.flatMap(\.tickets) }.first { $0.id == ticket }?.cards ?? []
    }
}
