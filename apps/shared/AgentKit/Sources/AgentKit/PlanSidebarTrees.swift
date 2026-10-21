import Foundation

// The two-list sidebar (ov-503, design 3): the *why* and the *how*, apart.
// `ThemeTree` is Themes › Cards; `LaneTree` is Lanes › Agents, grouped as the
// Plan view's sections are, so the two read in one order. They replace
// `OneTree`'s single Theme › Task › Lane › Terminals outline. Terminals are
// the app's own (they are live panes, not plan data) and hang under a lane
// node in the view.
//
// A lane that works two cards still shows the join: `ThemeTree.lane(of:)` and
// `LaneTree.cards(of:)` answer both ways, and a view highlights the other
// side when one is chosen.

public struct ThemeTree: Equatable, Sendable {
    public struct CardNode: Equatable, Identifiable, Sendable {
        public var task: String
        public var key: String
        public var title: String
        public var status: TaskStatus
        public var glyph: PlanCardGlyph { PlanCardGlyph(status) }
        /// The lane working it now, shown as the trailing detail.
        public var lane: String?
        public var laneID: String?
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
        // A train's agent isn't a lane, so it isn't "the lane working" a card.
        // A lane naming a landed train still is: live work never vanishes.
        let gone = Set(plan.laneIsTrain.map(\.lane))
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
                let lane = plan.liveLane(ofTask: ref.task, excluding: gone)
                return CardNode(
                    task: ref.task, key: ref.key, title: known?.title ?? "", status: status, lane: lane?.name, laneID: lane?.id)
            }
            if cards.isEmpty {
                guard theme.cards.isEmpty, filter != .inReview, theme.state == "active" else { return nil }
            }
            return ThemeNode(
                id: theme.id, name: theme.name, done: theme.counts.done, total: PlanWords.total(theme.counts), cards: cards)
        }
    }

    /// The lane working a card, by the card's task id, or nil.
    public func lane(of task: String) -> String? {
        for theme in themes { if let card = theme.cards.first(where: { $0.task == task }) { return card.laneID } }
        return nil
    }

    /// The cards a lane works, by its id, across themes.
    public func cards(of lane: String) -> [CardNode] {
        themes.flatMap(\.cards).filter { $0.laneID == lane }
    }
}

extension PlanModel {
    /// The live lane working a card: one past queued first, else the queued
    /// one ranked highest. Nil for a card whose lanes have all landed.
    func liveLane(ofTask task: String, excluding gone: Set<String> = []) -> PlanLane? {
        let mine = lanes.filter { lane in
            lane.state.isLive && !gone.contains(lane.id) && lane.cards.contains { $0.task == task }
        }
        return mine.first { $0.state != .queued } ?? mine.min { ($0.planRank ?? .max) < ($1.planRank ?? .max) }
    }
}

public struct LaneTree: Equatable, Sendable {
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

    public struct LaneNode: Equatable, Identifiable, Sendable {
        public var id: String
        public var name: String
        public var heading: String
        /// The state beside it: on a train, only what matters ("Needs you",
        /// "Landed · 3f78754f"), and
        /// nothing for a lane that is simply aboard; off one, its state.
        public var state: String
        public var agents: [AgentNode]
        public var cards: [PlanCardRef]
    }

    /// A train inside the Landing group: "Train 86", what it is for, where it
    /// stands, and its lanes (ov-522).
    public struct TrainNode: Equatable, Identifiable, Sendable {
        public var id: String
        /// "Train 86".
        public var label: String
        /// Its title, under the label; nil when it has none.
        public var subtitle: String?
        /// "Checking on CI", the ladder step it stands on.
        public var phase: String
        public var lanes: [LaneNode]
    }

    public enum Kind: Equatable, Sendable {
        case landing, inReview, building, waiting, onLandedTrain
    }

    public struct Group: Equatable, Identifiable, Sendable {
        public var kind: Kind
        public var title: String
        /// A count for a folded group.
        public var detail: String?
        /// Waiting starts folded.
        public var folded: Bool
        public var lanes: [LaneNode]
        /// The trains of the Landing group, each with its lanes inside it;
        /// empty for every other group.
        public var trains: [TrainNode] = []
        public var id: String {
            switch kind {
            case .landing: "landing"
            case .inReview: "inReview"
            case .building: "building"
            case .waiting: "waiting"
            case .onLandedTrain: "onLandedTrain"
            }
        }
    }

    /// Landing (its trains inside it) first, then In Review, Building and the
    /// folded Waiting; a group with no lanes isn't listed.
    public var groups: [Group]

    public init(plan: PlanModel, sections: PlanSections? = nil, locale: Locale = .current) {
        let sections = sections ?? PlanSections(plan: plan)
        func node(_ agent: PlanFleetAgent, _ locale: Locale) -> AgentNode {
            AgentNode(
                id: agent.id, who: agent.who, working: agent.working, tokens: agent.tokens(locale: locale),
                terminal: agent.terminal)
        }
        func agents(_ lane: PlanLane) -> [AgentNode] {
            let fleet = plan.agents.filter { $0.lane?.id == lane.id }
            if !fleet.isEmpty {
                return PlanFleet(fleet).agents.map { node($0, locale) }
            }
            // A runner that totals the day's agents (`agents_spend`, ov-518)
            // lists the ones that exist: none of this lane's does, whatever its
            // records say. A lane in review between the builder and the
            // reviewer has no agent rows.
            if plan.agentsSpend != nil { return [] }
            // An older runner lists none that exist: the agents the
            // orchestrator recorded on it and has not ended.
            return lane.agents.enumerated().filter { $0.element.endedAt == nil }.map { index, a in
                AgentNode(
                    id: "\(lane.id):\(index)", who: PlanWords.agentWho(a), working: a.endedAt == nil, tokens: nil)
            }
        }
        func laneNode(_ lane: PlanLane, state: String) -> LaneNode {
            LaneNode(id: lane.id, name: lane.name, heading: lane.heading, state: state, agents: agents(lane), cards: lane.cards)
        }
        func rows(_ rows: [PlanSections.LaneRow]) -> [LaneNode] { rows.map { laneNode($0.lane, state: PlanWords.status($0.lane)) } }

        var groups: [Group] = []
        if !sections.trains.isEmpty {
            groups.append(
                Group(
                    kind: .landing, title: PlanSections.Kind.landing.title, detail: nil, folded: false, lanes: [],
                    trains: sections.trains.map { card in
                        TrainNode(
                            id: card.train.id, label: card.train.label, subtitle: card.train.subtitle, phase: card.phase.word,
                            lanes: card.lanes.map { laneNode($0.lane, state: Self.aboard($0.note)) })
                    }))
        }
        let waiting = sections.waiting.onYou + sections.waiting.next + sections.waiting.notRanked
        for (kind, title, list, folded) in [
            (Kind.inReview, "In Review", sections.inReview, false), (.building, "Building", sections.building, false),
            (.waiting, "Waiting", waiting, true), (.onLandedTrain, "Waiting for the Next Landing", sections.onLandedTrain, true),
        ] where !list.isEmpty {
            groups.append(
                Group(kind: kind, title: title, detail: folded ? "\(list.count)" : nil, folded: folded, lanes: rows(list)))
        }
        self.groups = groups
    }

    /// What a lane aboard says beside its title: its note, but "Landed" without
    /// the commit, which would take the title's room (the train card has it).
    static func aboard(_ note: String?) -> String {
        guard let note else { return "" }
        return note.hasPrefix("Landed") ? "Landed" : note
    }

    /// The cards a lane works, by the lane's id: what a view highlights when
    /// a lane is chosen.
    public func cards(of lane: String) -> [PlanCardRef] {
        groups.lazy.flatMap { $0.lanes + $0.trains.flatMap(\.lanes) }.first { $0.id == lane }?.cards ?? []
    }
}

extension PlanWords {
    /// A lane's typed agent record as a row says it: "Builder · Opus".
    public static func agentWho(_ a: PlanAgent) -> String {
        let role =
            switch a.role {
            case "review": "Reviewer"
            case "fix": "Fixer"
            default: "Builder"
            }
        let model = model(a.model)
        return model.isEmpty ? role : "\(role) · \(model)"
    }
}
