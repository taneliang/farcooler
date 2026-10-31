import Foundation

// The plan column (ov-867, C10f): what the plan side says, as values, so the
// Mac draws it and the phones and the web can follow with the same answer.
// The mental model is `.claude/agent/reports/ov-867/intent.md`:
//
// 1. the facts line: what needs you, what's stuck, the next landing, and how
//    much of the board is done;
// 2. Initiatives (today's themes): the larger goals, each with its goal,
//    progress and next step;
// 3. the pipeline: status, then agent, then tickets. Each agent says what it
//    does now, and its tickets are listed under it. A ticket's initiative is
//    the ticket's, never the agent's;
// 4. the footer counts, which open the ticket lists.
//
// Needs You sits between the facts and Initiatives, but its rows are the
// runner's own list, which the plan doesn't carry, so the views add them.
//
// Built on today's wire (lanes, trains and the agents the runner lists). The
// new wire (ov-872) names each agent's role and what it does now; until then
// the column says the best that today's fields support, and never more.

public struct PlanColumn: Equatable, Sendable {
    // MARK: The facts line

    /// One fact on the facts line, as runs of text: the number is strong.
    public struct Fact: Equatable, Sendable, Identifiable {
        public struct Run: Equatable, Sendable {
            public var text: String
            public var strong: Bool
            public init(_ text: String, strong: Bool = false) {
                self.text = text
                self.strong = strong
            }
        }
        public enum Kind: String, Sendable { case needsYou, stuck, nextLanding, done }
        public var kind: Kind
        public var runs: [Run]
        /// Drawn in amber: it needs the owner (above 0) or is stuck.
        public var attention: Bool
        public var id: String { kind.rawValue }
        /// The fact as one string, for a label or a test.
        public var text: String { runs.map(\.text).joined() }
    }

    // MARK: Initiatives

    public struct Initiative: Equatable, Sendable, Identifiable {
        /// The theme's id: an initiative is today's theme.
        public var id: String
        public var name: String
        /// The outcome the theme was written for; empty when it has none.
        public var goal: String
        public var done: Int
        public var total: Int
        /// What's next for it, or what waits on the owner (`asksOwner`).
        public var next: String?
        /// `next` is the owner's to act on: drawn in amber.
        public var asksOwner: Bool

        /// "20 of 29".
        public var progress: String { "\(done) of \(total)" }
        /// How full its meter is, 0 to 1.
        public var fraction: Double { total > 0 ? min(1, Double(done) / Double(total)) : 0 }
    }

    // MARK: The pipeline

    /// Where an agent's work stands, in the order the pipeline lists them.
    public enum Stage: String, CaseIterable, Sendable {
        case stuck, building, reviewing, landing

        public var title: String {
            switch self {
            case .stuck: "Stuck"
            case .building: "Building"
            case .reviewing: "Reviewing"
            case .landing: "Landing"
            }
        }
    }

    /// What an agent does.
    public enum Role: String, Sendable {
        case builder, reviewer, lander, agent

        public var word: String {
            switch self {
            case .builder: "Builder"
            case .reviewer: "Reviewer"
            case .lander: "Lander"
            case .agent: "Agent"
            }
        }

        /// Its glyph, an SF Symbol.
        public var symbol: String {
            switch self {
            case .builder: "hammer"
            case .reviewer: "eye"
            case .lander: "arrow.down.to.line"
            case .agent: "sparkle"
            }
        }

        init(_ role: PlanFleetRole) {
            // ov-872: the new wire names builder, reviewer and lander; a
            // subagent or a bare pane says nothing of its job today.
            switch role {
            case .build, .fix: self = .builder
            case .review: self = .reviewer
            case .integrator: self = .lander
            case .orchestrator, .subagent, .agent: self = .agent
            }
        }
    }

    public struct Ticket: Equatable, Sendable, Identifiable {
        public var task: String
        public var key: String
        public var title: String
        /// The initiative the ticket is for, if any: the ticket's own.
        public var initiative: String?
        public var initiativeID: String?
        public var stage: Stage
        public var id: String { task }
    }

    /// One agent and the tickets it works.
    public struct Agent: Equatable, Sendable, Identifiable {
        public var id: String
        public var role: Role
        /// "Builder 2": its role, numbered when there are several.
        public var name: String
        /// What it does now. ov-872: today's best is what the orchestrator
        /// called it, else the lane's or train's title.
        public var doing: String
        /// `doing` says why it's stuck: drawn in amber.
        public var doingIsTrouble: Bool
        /// How long it has been at it, in ms; nil when nothing says.
        public var elapsed: Int64?
        /// Its pane, to open its chat; nil when the runner names none.
        public var terminal: String?
        /// The lane it works, for its page when there's no pane to open.
        public var lane: String?
        public var tickets: [Ticket]
        /// It is working now: its glyph shows it, and its tickets never do.
        public var working: Bool = false
        /// Tickets in this stage nobody is on, said under the last card.
        public var more: Int = 0

        /// "47 min", or nil.
        public var time: String? { elapsed.map(PlanWords.span) }
    }

    public struct Group: Equatable, Sendable, Identifiable {
        public var stage: Stage
        public var agents: [Agent]
        /// Tickets in this stage with no agent on them.
        public var unattended: [Ticket]
        public var id: String { stage.rawValue }

        /// The tickets in this stage, each once.
        public var count: Int {
            Set(agents.flatMap { $0.tickets.map(\.task) } + unattended.map(\.task)).count
        }

        /// "12 more waiting for review": the tickets with nobody on them.
        public var unattendedWords: String? {
            guard !unattended.isEmpty else { return nil }
            let n = unattended.count
            let lead = agents.isEmpty ? (n == 1 ? "1 ticket" : "\(n) tickets") : "\(n) more"
            switch stage {
            case .stuck: return "\(lead) with no agent on \(n == 1 ? "it" : "them")"
            case .building: return "\(lead) with no builder running"
            case .reviewing: return "\(lead) waiting for review"
            case .landing: return "\(lead) waiting to land"
            }
        }
    }

    // MARK: The footer

    public struct Count: Equatable, Sendable, Identifiable {
        public enum Kind: String, Sendable { case waiting, notStarted, done }
        public var kind: Kind
        public var count: Int
        /// The board's list it opens.
        public var status: TaskStatus
        public var id: String { kind.rawValue }

        /// "10 tickets waiting for a builder", "22 not started", "796 done".
        public var words: String {
            switch kind {
            case .waiting: "\(count == 1 ? "1 ticket" : "\(count) tickets") waiting for a builder"
            case .notStarted: "\(count) not started"
            case .done: "\(count) done"
            }
        }
    }

    public var facts: [Fact]
    /// The initiatives in motion or waiting on the owner, in the board's order.
    public var initiatives: [Initiative]
    /// The rest that are open, said on one quiet line.
    public var quietInitiatives: [Initiative]
    /// The stages with anything in them, in order.
    public var pipeline: [Group]
    public var footer: [Count]
    /// The runner's clock.
    public var now: Int64

    /// How many initiatives are open: the section's count.
    public var initiativeCount: Int { initiatives.count + quietInitiatives.count }

    /// How many agents the pipeline lists.
    public var agentCount: Int { pipeline.reduce(0) { $0 + $1.agents.count } }

    /// "10 agents", "1 agent".
    public var agentWords: String { agentCount == 1 ? "1 agent" : "\(agentCount) agents" }

    /// "Nothing moving: Mac look and feel, 33 of 34; Marketing, 6 of 8".
    public var quietLine: String? {
        guard !quietInitiatives.isEmpty else { return nil }
        return "Nothing moving: " + quietInitiatives.map { "\($0.name), \($0.progress)" }.joined(separator: "; ")
    }

    /// - Parameters:
    ///   - needsYou: the workspace's Needs You count, the theme asks included.
    ///   - statuses: the board's card statuses by task id.
    ///   - titles: the board's card titles by task id, for a card the plan
    ///     doesn't carry.
    public init(
        plan: PlanModel, needsYou: Int = 0, statuses: [String: TaskStatus] = [:], titles: [String: String] = [:]
    ) {
        now = plan.nowMs
        pipeline = Self.pipeline(plan, statuses: statuses, titles: titles)
        let moving = Set(pipeline.flatMap { $0.agents.flatMap(\.tickets) + $0.unattended }.map(\.task))
        (initiatives, quietInitiatives) = Self.initiatives(plan, moving: moving)
        footer = Self.footer(plan, statuses: statuses)
        facts = Self.facts(plan, needsYou: needsYou, pipeline: pipeline)
    }

    // MARK: - Building it

    static func initiatives(_ plan: PlanModel, moving: Set<String>) -> ([Initiative], [Initiative]) {
        var shown: [Initiative] = []
        var quiet: [Initiative] = []
        for theme in plan.shownThemes where theme.state != "done" {
            let total = PlanWords.total(theme.counts)
            if total > 0, theme.counts.done >= total { continue }
            let ask = theme.ownerAsk.trimmingCharacters(in: .whitespacesAndNewlines)
            let next = theme.next.trimmingCharacters(in: .whitespacesAndNewlines)
            let initiative = Initiative(
                id: theme.id, name: theme.name, goal: theme.outcome.trimmingCharacters(in: .whitespacesAndNewlines),
                done: theme.counts.done, total: total,
                next: ask.isEmpty ? (next.isEmpty ? nil : "Next: \(next)") : "Waiting on you: \(ask)",
                asksOwner: !ask.isEmpty)
            let inMotion = theme.cards.contains { moving.contains($0.task) }
            // Paused themes are open but not in motion: they go to the quiet line.
            if (inMotion && theme.state == "active") || !ask.isEmpty { shown.append(initiative) } else { quiet.append(initiative) }
        }
        return (shown, quiet)
    }

    static func footer(_ plan: PlanModel, statuses: [String: TaskStatus]) -> [Count] {
        // Tickets on a queued lane, waiting for a builder, each once.
        var seen = Set<String>()
        let waiting = plan.lanes.filter { $0.state == .queued }.flatMap(\.cards)
            .filter { statuses[$0.task] != .cancelled && seen.insert($0.task).inserted }
        var counts: [Count] = []
        if !waiting.isEmpty { counts.append(Count(kind: .waiting, count: waiting.count, status: .todo)) }
        if let c = plan.boardCounts {
            // Not started, less the ones already counted as waiting.
            let unstarted = waiting.filter { [.backlog, .todo].contains(statuses[$0.task] ?? .todo) }.count
            let notStarted = max(0, c.backlog + c.todo - unstarted)
            if notStarted > 0 { counts.append(Count(kind: .notStarted, count: notStarted, status: .backlog)) }
            if c.done > 0 { counts.append(Count(kind: .done, count: c.done, status: .done)) }
        }
        return counts
    }

    static func facts(_ plan: PlanModel, needsYou: Int, pipeline: [Group]) -> [Fact] {
        var facts: [Fact] = []
        if needsYou > 0 {
            facts.append(Fact(kind: .needsYou, runs: [.init("\(needsYou)", strong: true), .init(" need you")], attention: true))
        }
        if let stuck = pipeline.first(where: { $0.stage == .stuck }) {
            let longest = stuck.agents.compactMap(\.elapsed).max()
            let words = "\(stuck.count) stuck" + (longest.map { ", \(PlanWords.span($0))" } ?? "")
            facts.append(Fact(kind: .stuck, runs: [.init(words)], attention: true))
        }
        if let next = nextLanding(plan) {
            facts.append(
                Fact(
                    kind: .nextLanding,
                    runs: [
                        .init("Next landing: "), .init("\(next.tickets)", strong: true),
                        .init(next.tickets == 1 ? " ticket" : " tickets"),
                        .init(next.since.map { ", \(PlanWords.span(plan.nowMs - $0))" } ?? ""),
                    ].filter { !$0.text.isEmpty }, attention: false))
        }
        if let c = plan.boardCounts, PlanWords.total(c) > 0 {
            facts.append(
                Fact(kind: .done, runs: [.init("\(c.done)", strong: true), .init(" of \(PlanWords.total(c)) done")], attention: false))
        }
        return facts
    }

    /// The live train furthest along: how many tickets it carries, and since
    /// when it has been in its phase.
    static func nextLanding(_ plan: PlanModel) -> (tickets: Int, since: Int64?)? {
        let live = plan.liveTrains.filter { $0.phase(in: plan) != .ciFailed && $0.phase(in: plan) != .stalled }
        guard let train = live.max(by: { $0.phase(in: plan).progress < $1.phase(in: plan).progress }) else { return nil }
        let ids = Set(train.lanes.map(\.lane))
        var seen = Set<String>()
        let tickets = plan.lanes.filter { ids.contains($0.id) && $0.state == .landing }.flatMap(\.cards)
            .filter { seen.insert($0.task).inserted }.count
        let since = train.phaseSince(in: plan)
        return (tickets, since > 0 ? since : nil)
    }
}

// MARK: - The pipeline from today's wire

extension PlanColumn {
    static func pipeline(_ plan: PlanModel, statuses: [String: TaskStatus], titles: [String: String]) -> [Group] {
        let isTrainAgent = Set(plan.laneIsTrain.map(\.lane))
        let lanes = plan.lanes.filter { !isTrainAgent.contains($0.id) && $0.state.isLive && $0.state != .queued }
        let live = plan.liveTrains
        // A landing lane on a live train is the train's lander's.
        var trainOf: [String: PlanTrain] = [:]
        for train in live { for ref in train.lanes { trainOf[ref.lane] = train } }

        func ticket(_ ref: PlanCardRef, lane: PlanLane?, stage: Stage) -> Ticket {
            let theme = plan.theme(ofTask: ref.task)
            let known = plan.card(ref.task)?.title ?? titles[ref.task]
            let title: String = {
                if let known, !known.isEmpty { return known }
                if let lane, lane.cards.count == 1, lane.title?.isEmpty == false { return lane.heading }
                return ref.key
            }()
            return Ticket(task: ref.task, key: ref.key, title: title, initiative: theme?.name, initiativeID: theme?.id, stage: stage)
        }
        func stage(of lane: PlanLane) -> Stage {
            if lane.state == .landing, let train = trainOf[lane.id] {
                let phase = train.phase(in: plan)
                return phase.needsAttention ? .stuck : .landing
            }
            if lane.stale { return .stuck }
            switch lane.state {
            case .review: return .reviewing
            case .landing: return .landing
            default: return .building
            }
        }
        func tickets(_ lane: PlanLane) -> [Ticket] {
            let s = stage(of: lane)
            return lane.cards.filter { statuses[$0.task] != .cancelled }.map { ticket($0, lane: lane, stage: s) }
        }

        var cards: [(stage: Stage, agent: Agent, started: Int64, card: String?)] = []
        var attended = Set<String>()
        var trainsDrawn = Set<String>()
        let agents = plan.agents.filter { $0.role != .orchestrator }
        for agent in agents {
            let lane = agent.lane.flatMap { ref in lanes.first { $0.id == ref.id } }
            let train = agent.train.flatMap { ref in live.first { $0.id == ref.id } }
            var role = Role(agent.role)
            var mine: [Ticket] = []
            var s: Stage
            var doing = agent.label
            var trouble = false
            var laneID: String?
            if let train {
                // The agent landing a train: the lander, over the train's tickets.
                role = .lander
                trainsDrawn.insert(train.id)
                let ids = Set(train.lanes.map(\.lane))
                mine = lanes.filter { ids.contains($0.id) && $0.state == .landing }.flatMap(tickets)
                let phase = train.phase(in: plan)
                s = phase.needsAttention ? .stuck : .landing
                (doing, trouble) = phaseWords(phase, label: agent.label)
            } else if let lane {
                mine = tickets(lane)
                s = stage(of: lane)
                laneID = lane.id
                if s == .stuck, let stale = PlanWords.stale(lane, now: plan.nowMs) {
                    (doing, trouble) = (stale, true)
                } else if doing.isEmpty {
                    doing = lane.heading
                }
            } else {
                if let ref = agent.card { mine = [ticket(ref, lane: nil, stage: role == .reviewer ? .reviewing : .building)] }
                s = role == .reviewer ? .reviewing : role == .lander ? .landing : .building
                if doing.isEmpty { doing = agent.working ? "Working" : "Idle" }
            }
            mine = mine.map { var t = $0; t.stage = s; return t }
            attended.formUnion(mine.map(\.task))
            cards.append(
                (
                    s,
                    Agent(
                        id: agent.id, role: role, name: role.word, doing: doing, doingIsTrouble: trouble,
                        elapsed: agent.startedAt > 0 ? max(0, plan.nowMs - agent.startedAt) : nil, terminal: agent.terminal,
                        lane: laneID, tickets: mine, working: agent.working),
                    agent.startedAt, agent.card?.task
                ))
        }
        // A live train whose integrating agent isn't listed: its own record
        // says who lands it, so it still has a lander.
        for train in live where !trainsDrawn.contains(train.id) {
            guard let record = train.agent, record.isWorking else { continue }
            let ids = Set(train.lanes.map(\.lane))
            let phase = train.phase(in: plan)
            let s: Stage = phase.needsAttention ? .stuck : .landing
            let mine = lanes.filter { ids.contains($0.id) && $0.state == .landing }.flatMap(tickets).map { var t = $0; t.stage = s; return t }
            guard !mine.isEmpty else { continue }
            attended.formUnion(mine.map(\.task))
            let (doing, trouble) = phaseWords(phase, label: "")
            cards.append(
                (
                    s,
                    Agent(
                        id: "train:\(train.id)", role: .lander, name: Role.lander.word, doing: doing, doingIsTrouble: trouble,
                        elapsed: record.startedAt > 0 ? max(0, plan.nowMs - record.startedAt) : nil, terminal: nil, lane: nil,
                        tickets: mine, working: true),
                    record.startedAt, nil
                ))
        }

        // Each ticket is on one agent: the lander of its train first, then the
        // agent whose own card it is, then the first to claim it.
        var owner: [String: (rank: Int, index: Int)] = [:]
        for (index, card) in cards.enumerated() {
            for t in card.agent.tickets {
                let rank = card.agent.role == .lander ? 0 : card.card == t.task ? 1 : 2
                if let have = owner[t.task], (have.rank, have.index) <= (rank, index) { continue }
                owner[t.task] = (rank, index)
            }
        }
        for index in cards.indices {
            cards[index].agent.tickets.removeAll { owner[$0.task]?.index != index }
        }

        // Number each role in the order its agents started: "Builder 1".
        var byRole: [Role: [Int]] = [:]
        for (index, card) in cards.enumerated() { byRole[card.agent.role, default: []].append(index) }
        for (role, indices) in byRole where indices.count > 1 {
            let ordered = indices.sorted { (cards[$0].started, $0) < (cards[$1].started, $1) }
            for (n, index) in ordered.enumerated() { cards[index].agent.name = "\(role.word) \(n + 1)" }
        }

        // Tickets nobody is on, by the stage of their lane.
        var unattended: [Stage: [Ticket]] = [:]
        var seen = attended
        for lane in lanes {
            for t in tickets(lane) where seen.insert(t.task).inserted { unattended[t.stage, default: []].append(t) }
        }

        return Stage.allCases.compactMap { stage in
            var agents = cards.filter { $0.stage == stage }.sorted { (Self.order($0.agent), $0.started) < (Self.order($1.agent), $1.started) }
                .map(\.agent)
            let loose = unattended[stage] ?? []
            // Said under the last card that has tickets, never under an empty one.
            if let last = agents.lastIndex(where: { !$0.tickets.isEmpty }) { agents[last].more = loose.count }
            guard !agents.isEmpty || !loose.isEmpty else { return nil }
            return Group(stage: stage, agents: agents, unattended: loose)
        }
    }

    /// Agents with tickets first, then by name's number (start order).
    private static func order(_ agent: Agent) -> Int { agent.tickets.isEmpty ? 1 : 0 }

    /// What a lander says it does: its train's phase as a sentence, the
    /// orchestrator's label first when it gave one. ov-872: the new wire
    /// says what the lander does now.
    static func phaseWords(_ phase: TrainPhase, label: String) -> (String, Bool) {
        if !label.isEmpty, !phase.needsAttention { return (label, false) }
        let sentence = phase.sentence
        return (sentence.prefix(1).uppercased() + sentence.dropFirst(), phase.needsAttention)
    }
}

extension PlanColumn {
    /// What a Needs You row puts in the orchestrator's input when clicked:
    /// a reference to it, for the owner to finish (unsent).
    public static func quote(key: String?, title: String) -> String {
        if let key, !key.isEmpty { return AskAboutTask.draft(key: key, title: title) }
        return "About \u{201C}\(AskAboutTask.oneLine(title))\u{201D}: "
    }
}
