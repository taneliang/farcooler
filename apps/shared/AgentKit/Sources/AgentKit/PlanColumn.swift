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
// Built on the line (ov-872): each agent has a role (builder, reviewer or
// lander) and says what it is doing now, and its tickets are the ones it works.

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

        init(_ role: PlanAgentRole) {
            switch role {
            case .builder: self = .builder
            case .reviewer: self = .reviewer
            case .lander: self = .lander
            case .orchestrator, .unknown: self = .agent
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
        /// When it entered its stage, in ms; 0 when nothing says.
        public var since: Int64 = 0
        public var id: String { task }
    }

    /// One agent and the tickets it works.
    public struct Agent: Equatable, Sendable, Identifiable {
        public var id: String
        public var role: Role
        /// "Builder 2": its role, numbered when there are several.
        public var name: String
        /// What it does now, in its own words (`doing_now`); the problem
        /// when it is stuck.
        public var doing: String
        /// `doing` says why it's stuck: drawn in amber.
        public var doingIsTrouble: Bool
        /// How long it has been at it, in ms; nil when nothing says.
        public var elapsed: Int64?
        /// Its pane, to open its chat; nil when the runner names none.
        public var terminal: String?
        /// The first ticket it works, for its page when there's no pane to
        /// open.
        public var ticket: String?
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

        /// How many unattended tickets a stage with no agent draws before
        /// "Show All": one tall card of them would run past the screen.
        public static let previewLimit = 3

        /// The unattended tickets drawn in place: a stage with no agent shows
        /// the first few; a stage with agents shows none, its "more" line
        /// opening them all.
        public var preview: [Ticket] { agents.isEmpty ? Array(unattended.prefix(Self.previewLimit)) : [] }

        /// The unattended tickets only the list shows.
        public var hidden: Int { unattended.count - preview.count }

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

    /// A ticket waiting for a builder, as the footer's list draws it.
    public struct Waiting: Equatable, Sendable, Identifiable {
        public var task: String
        public var key: String
        public var title: String
        /// When it began waiting, in ms.
        public var since: Int64
        public var id: String { task }
    }

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
    /// The tickets waiting for a builder (the footer's first list), each once,
    /// in the plan's order.
    public var waiting: [Waiting]
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
        waiting = Self.waiting(plan, statuses: statuses)
        footer = Self.footer(plan, waiting: waiting.count, statuses: statuses)
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

    /// Tickets waiting for a builder, each once, a cancelled one left out.
    static func waiting(_ plan: PlanModel, statuses: [String: TaskStatus]) -> [Waiting] {
        var seen = Set<String>()
        return plan.lines.filter { $0.stage == .waiting }
            .filter { statuses[$0.task] != .cancelled && seen.insert($0.task).inserted }
            .map { Waiting(task: $0.task, key: $0.key, title: $0.heading, since: $0.stageSince) }
    }

    static func footer(_ plan: PlanModel, waiting: Int, statuses: [String: TaskStatus]) -> [Count] {
        var counts: [Count] = []
        if waiting > 0 { counts.append(Count(kind: .waiting, count: waiting, status: .todo)) }
        if let c = plan.boardCounts {
            // Not started, less the ones already counted as waiting.
            let unstarted = Self.waiting(plan, statuses: statuses).filter { [.backlog, .todo].contains(statuses[$0.task] ?? .todo) }.count
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
        // A landing carrying no ticket yet is not a landing to name.
        if let next = nextLanding(plan), next.tickets > 0 {
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

    /// The live landing furthest along: how many tickets it carries, and
    /// since when it has been in its phase.
    static func nextLanding(_ plan: PlanModel) -> (tickets: Int, since: Int64?)? {
        let live = plan.liveLandings.filter { $0.phase(in: plan) != .ciFailed && $0.phase(in: plan) != .stalled }
        guard let landing = live.max(by: { $0.phase(in: plan).progress < $1.phase(in: plan).progress }) else { return nil }
        let tickets = Set(plan.lines(on: landing).filter { $0.stage == .landing }.map(\.task)).count
        let since = landing.phaseSince(in: plan)
        return (tickets, since > 0 ? since : nil)
    }
}

// MARK: - The pipeline from the line

extension PlanColumn {
    static func pipeline(_ plan: PlanModel, statuses: [String: TaskStatus], titles: [String: String]) -> [Group] {
        let live = plan.lines.filter { $0.stage.isPastWaiting && statuses[$0.task] != .cancelled }
        let liveByTask = Dictionary(live.map { ($0.task, $0) }, uniquingKeysWith: { first, _ in first })

        func ticket(_ line: PlanTicketLine, stage: Stage) -> Ticket {
            let theme = plan.theme(ofTask: line.task)
            let known = plan.card(line.task)?.title ?? titles[line.task]
            let title: String = {
                if !line.title.isEmpty { return line.title }
                if let known, !known.isEmpty { return known }
                return line.key
            }()
            return Ticket(task: line.task, key: line.key, title: title, initiative: theme?.name, initiativeID: theme?.id, stage: stage, since: line.stageSince)
        }
        /// Where a ticket's own state puts it, whoever works it.
        func stage(of line: PlanTicketLine) -> Stage {
            if line.stale || line.stage == .needsYou { return .stuck }
            switch line.stage {
            case .reviewing: return .reviewing
            case .landing: return .landing
            default: return .building
            }
        }

        var cards: [(stage: Stage, agent: Agent, started: Int64, tasks: Set<String>)] = []
        var attended = Set<String>()
        let agents = plan.agents.filter { $0.role != .orchestrator }
        for agent in agents {
            let role = Role(agent.role)
            let lines = agent.tickets.compactMap { liveByTask[$0.task] }
            // The landing a lander works: its phase says if it is stuck.
            let landing = role == .lander ? lines.compactMap { line in line.landing.flatMap(plan.landing(number:)) }.first : nil
            var s: Stage
            var doing = agent.doingNow
            var trouble = false
            switch role {
            case .lander:
                let phase = landing?.phase(in: plan)
                s = (phase?.needsAttention ?? false) ? .stuck : .landing
                if let phase, phase.needsAttention { (doing, trouble) = phaseWords(phase) }
            case .reviewer: s = .reviewing
            case .builder, .agent: s = .building
            }
            if s != .stuck, let stale = lines.first(where: \.stale) {
                s = .stuck
                if let words = PlanWords.stale(stale, now: plan.nowMs) { (doing, trouble) = (words, true) }
            } else if s != .stuck, lines.contains(where: { $0.stage == .needsYou }) {
                s = .stuck
            }
            if doing.isEmpty { doing = agent.title.isEmpty ? (agent.working ? "Working" : "Idle") : agent.title }
            let mine = lines.map { ticket($0, stage: s) }
            attended.formUnion(mine.map(\.task))
            cards.append(
                (
                    s,
                    Agent(
                        id: agent.id, role: role, name: role.word, doing: doing, doingIsTrouble: trouble,
                        elapsed: agent.startedAt > 0 ? max(0, plan.nowMs - agent.startedAt) : nil, terminal: agent.terminal,
                        ticket: lines.first?.task, tickets: mine, working: agent.working),
                    agent.startedAt, Set(agent.tickets.map(\.task))
                ))
        }

        // Each ticket is on one agent: the lander of its landing first, then
        // the agent whose job matches its stage, then the first to claim it.
        func rank(_ agent: Agent, _ task: String) -> Int {
            if agent.role == .lander { return 0 }
            let wanted: Role? =
                switch liveByTask[task]?.stage {
                case .building: .builder
                case .reviewing: .reviewer
                default: nil
                }
            return wanted == agent.role ? 1 : 2
        }
        var owner: [String: (rank: Int, index: Int)] = [:]
        for (index, card) in cards.enumerated() {
            for t in card.agent.tickets {
                let r = rank(card.agent, t.task)
                if let have = owner[t.task], (have.rank, have.index) <= (r, index) { continue }
                owner[t.task] = (r, index)
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

        // Tickets nobody is on, by the stage their own state gives them.
        var unattended: [Stage: [Ticket]] = [:]
        var seen = attended
        for line in live where seen.insert(line.task).inserted {
            let s = stage(of: line)
            unattended[s, default: []].append(ticket(line, stage: s))
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

    /// What a lander in trouble says: its landing's phase as a sentence.
    static func phaseWords(_ phase: LandingPhase) -> (String, Bool) {
        let sentence = phase.sentence
        let lead = sentence.prefix(1).uppercased() + sentence.dropFirst()
        return (lead, phase.needsAttention)
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
