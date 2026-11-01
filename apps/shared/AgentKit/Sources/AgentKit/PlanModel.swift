import Foundation

// The plan layer (ov-268, ov-872), as a client reads it: themes (why a group of
// cards exists), the line (one ticket per card in the plan: its stage, who
// worked it, its branch and pull request), agents (who is working now, and
// what each is doing) and landings (finished tickets going into the product
// together). EXPERIMENTAL, behind the runner's `board_plan` capability, and
// removable: nothing on a task points here, and nothing outside the Plan view
// and the one line on a task page reads it.
//
// The Mac reads `farcooler plan --json` (`crates/cli/src/plan.rs`,
// `plan_json`), and a theme's or ticket's timeline from `plan theme show` and
// `plan ticket show`, which add `events`. Every rule the Plan view draws is
// here, so `swift test --package-path apps/shared/AgentKit` reaches it; the
// views only lay it out. The wire is `test/fixtures/plan.json`.

/// A card a theme, a landing or a ticket's stack names, by id and key.
public struct PlanCardRef: Decodable, Equatable, Hashable, Sendable {
    public var task: String
    public var key: String

    public init(task: String, key: String) {
        self.task = task
        self.key = key
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        task = try c.decode(String.self, forKey: .task)
        key = try c.decodeIfPresent(String.self, forKey: .key) ?? ""
    }

    private enum CodingKeys: String, CodingKey { case task, key }
}

/// How many of a theme's cards are in each of the board's statuses.
public struct PlanCounts: Decodable, Equatable, Sendable {
    public var backlog = 0
    public var todo = 0
    public var needsDecision = 0
    public var inProgress = 0
    public var inReview = 0
    public var done = 0
    public var cancelled = 0

    public init() {}
}

public struct PlanTheme: Decodable, Equatable, Identifiable, Sendable {
    public var id: String
    public var short: String
    public var name: String
    /// One sentence: the world when it's done.
    public var outcome: String
    /// Where it stands, rewritten at checkpoints.
    public var story: String
    /// When `story` was last written, in ms; 0 if never.
    public var storyAt: Int64
    public var next: String
    /// What needs the owner; empty when nothing does.
    public var ownerAsk: String
    /// `active`, `paused`, `done` or `dropped`.
    public var state: String
    public var ordinal: Int64
    public var cards: [PlanCardRef]
    public var counts: PlanCounts
    /// What its tickets spent (ov-306). Nil from a runner before it.
    public var spend: PlanSpend?
    /// Its token budget (ov-307), counted as `spend` is; nil when it has none
    /// or from a runner without `board_cost`.
    public var budgetTokens: UInt64?
    /// Its tokens on each of the last seven UTC days, oldest first and today
    /// last (ov-307); nil from a runner without `board_cost`.
    public var trendTokens: [UInt64]?
    /// When it last moved, by the runner's reckoning (its story, tickets,
    /// rulings and cards, ov-331); nil from a runner before it, and a client
    /// then works it out from what it has.
    public var lastMovedAt: Int64?
}

// `TicketStage` is in `PlanGlance.swift`, which the watch and the widgets
// compile without this file (ov-310).

/// Who worked a ticket and as what, on the line (ov-872).
public enum WorkRole: String, Decodable, Sendable, CaseIterable {
    case build, fix, review, land, unknown

    public init(from decoder: Decoder) throws {
        self = WorkRole(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
    }
}

/// One agent's turn at a ticket, oldest first on the ticket.
public struct PlanWork: Decodable, Equatable, Sendable {
    /// The agent's id, as `PlanAgent.id` has it.
    public var agent: String
    public var role: WorkRole
    public var startedAt: Int64
    public var endedAt: Int64?
    /// What the agent said when it ended: `finished`, or its verdict.
    public var verdict: String
    /// Recorded from the agent's description, not by the agent itself.
    public var linkedByDescription: Bool

    public var isOpen: Bool { endedAt == nil }
}

/// What an agent, a ticket or a landing spent. An agent the runner has read no turn of is
/// `unmeasuredAgents`, which reads "Not reported", never zero.
public struct PlanSpend: Decodable, Equatable, Sendable {
    public var inputTokens: UInt64 = 0
    public var outputTokens: UInt64 = 0
    public var cacheReadTokens: UInt64 = 0
    public var cacheWriteTokens: UInt64 = 0
    /// Millionths of a dollar; nil when no model's price is known.
    public var costMicros: Int64?
    public var runs = 0
    public var unmeasuredAgents = 0
    /// Agents also recorded on another ticket, whose spend is split evenly
    /// across their tickets: the figures hold this ticket's part.
    public var sharedAgents = 0

    public init() {}

    /// Each figure is zero when absent, so a field the runner stops sending
    /// reads as nothing spent rather than as a plan that can't be read.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        inputTokens = try c.decodeIfPresent(UInt64.self, forKey: .inputTokens) ?? 0
        outputTokens = try c.decodeIfPresent(UInt64.self, forKey: .outputTokens) ?? 0
        cacheReadTokens = try c.decodeIfPresent(UInt64.self, forKey: .cacheReadTokens) ?? 0
        cacheWriteTokens = try c.decodeIfPresent(UInt64.self, forKey: .cacheWriteTokens) ?? 0
        costMicros = try c.decodeIfPresent(Int64.self, forKey: .costMicros)
        runs = try c.decodeIfPresent(Int.self, forKey: .runs) ?? 0
        unmeasuredAgents = try c.decodeIfPresent(Int.self, forKey: .unmeasuredAgents) ?? 0
        sharedAgents = try c.decodeIfPresent(Int.self, forKey: .sharedAgents) ?? 0
    }

    private enum CodingKeys: String, CodingKey {
        case inputTokens, outputTokens, cacheReadTokens, cacheWriteTokens, costMicros, runs, unmeasuredAgents
        case sharedAgents
    }

    public var totalTokens: UInt64 { inputTokens + outputTokens + cacheReadTokens + cacheWriteTokens }
}

/// One ticket on the line (ov-872): a card in the plan, with where it stands.
public struct PlanTicketLine: Decodable, Equatable, Identifiable, Sendable {
    /// The card's id.
    public var task: String
    public var key: String
    /// The card's title.
    public var title: String
    /// The card's status in the board's words ("In Review").
    public var status: String
    public var stage: TicketStage
    /// One line: why it's in the plan, or why it's where it is now.
    public var reason: String
    /// 1 is next up; nil outside the plan.
    public var planRank: Int?
    public var worktreePath: String
    public var branch: String
    /// The ticket this one's branch is stacked on; it lands first.
    public var stackParent: PlanCardRef?
    public var sameBranch: String?
    public var prNumber: Int?
    public var landedSha: String?
    public var stageSince: Int64
    /// Taken off a landing by hand.
    public var heldOff: Bool
    /// Sat in one live stage for over an hour, by the runner's clock.
    public var stale: Bool
    public var fixRounds: Int
    /// The number of the landing it is on, if any.
    public var landing: Int?
    /// Everyone who worked it, oldest first.
    public var work: [PlanWork]
    public var spend: PlanSpend
    /// Its token budget (ov-307); nil when it has none.
    public var budgetTokens: UInt64?
    /// Where its pull request stands (ov-316); nil when the runner had
    /// nothing to say. Read it through `shownStage`.
    public var prStage: PlanPrStage?

    public var id: String { task }

    /// What every surface shows first: the card's title, else its key.
    public var heading: String { title.isEmpty ? key : title }

    /// The ticket as a card reference.
    public var card: PlanCardRef { PlanCardRef(task: task, key: key) }

    /// A builder has it again, for a fix round after a review or a failed
    /// landing: the work open on it is a fix.
    public var isFixing: Bool { stage == .building && work.last(where: \.isOpen)?.role == .fix }

    /// The work that is still open, newest first.
    public var openWork: [PlanWork] { work.filter(\.isOpen).reversed() }
}

/// A card's row, as the plan read it: what a theme page lists for a card
/// the board hasn't read. `status` is the CLI's word for it, for showing.
public struct PlanCard: Decodable, Equatable, Sendable {
    public var task: String
    public var key: String
    public var title: String
    public var status: String
}

/// A card `plan --json` flags in `off_the_line` or `landed_not_closed`: its key
/// and status, with no title (the CLI sends none).
public struct PlanFlaggedCard: Decodable, Equatable, Sendable {
    public var task: String
    public var key: String
    public var status: String

    public init(task: String, key: String, status: String) {
        self.task = task
        self.key = key
        self.status = status
    }
}

/// One read of a board's plan: every Plan surface draws from it.
public struct PlanModel: Decodable, Equatable, Sendable {
    /// The runner's clock when it answered.
    public var nowMs: Int64
    public var themes: [PlanTheme]
    /// The line (ov-872): the live tickets, and the ones that landed in the
    /// last week. The plan is the waiting ones with a rank, first is next up.
    public var lines: [PlanTicketLine]
    public var cards: [PlanCard]
    /// Decided for you (ov-304): standing rulings newest first, then the
    /// settled ones, in the runner's order. Empty from a runner without
    /// `board_rulings`, and absent from an older CLI's answer.
    public var rulings: [PlanRuling]
    /// Landings (ov-309, ov-872): those not landed or dropped, oldest first,
    /// then the settled ones.
    public var landings: [PlanLanding]
    /// What the runner last read of CI for each subject the board names: its
    /// landings' pushed SHAs and its pages' CI references (ov-306).
    public var ci: [PlanCIRead]
    /// How many of the board's cards are in each status (ov-306), what a
    /// page's card counts draw; nil from a runner before it.
    public var boardCounts: PlanCounts?
    /// The week's tokens and the harness and model comparison (ov-307); nil
    /// from a runner without `board_cost`.
    public var cost: PlanCostRead?
    /// "Worth a look" (ov-331): open cards in review or progress that are not
    /// on the line, and open cards whose tickets have landed. Empty from an
    /// older runner.
    public var offTheLine: [PlanFlaggedCard]
    public var landedNotClosed: [PlanFlaggedCard]
    /// Every agent that still exists, the orchestrator included (ov-503,
    /// ov-518, ov-872).
    public var agents: [PlanAgent]
    /// What every agent of the last 24 hours spent, those no longer in
    /// `agents` included; nil from a runner before ov-518.
    public var agentsSpend: PlanSpend?

    public init(
        nowMs: Int64 = 0, themes: [PlanTheme] = [], lines: [PlanTicketLine] = [],
        cards: [PlanCard] = [], rulings: [PlanRuling] = [], landings: [PlanLanding] = [], ci: [PlanCIRead] = [],
        boardCounts: PlanCounts? = nil, cost: PlanCostRead? = nil, offTheLine: [PlanFlaggedCard] = [],
        landedNotClosed: [PlanFlaggedCard] = [], agents: [PlanAgent] = [], agentsSpend: PlanSpend? = nil
    ) {
        self.nowMs = nowMs
        self.themes = themes
        self.lines = lines
        self.cards = cards
        self.rulings = rulings
        self.landings = landings
        self.ci = ci
        self.boardCounts = boardCounts
        self.cost = cost
        self.offTheLine = offTheLine
        self.landedNotClosed = landedNotClosed
        self.agents = agents
        self.agentsSpend = agentsSpend
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        nowMs = try c.decode(Int64.self, forKey: .nowMs)
        themes = try c.decode([PlanTheme].self, forKey: .themes)
        lines = try c.decode([PlanTicketLine].self, forKey: .lines)
        cards = try c.decode([PlanCard].self, forKey: .cards)
        rulings = try c.decodeIfPresent([PlanRuling].self, forKey: .rulings) ?? []
        landings = try c.decodeIfPresent([PlanLanding].self, forKey: .landings) ?? []
        ci = try c.decodeIfPresent([PlanCIRead].self, forKey: .ci) ?? []
        boardCounts = try c.decodeIfPresent(PlanCounts.self, forKey: .boardCounts)
        cost = try c.decodeIfPresent(PlanCostRead.self, forKey: .cost)
        offTheLine = try c.decodeIfPresent([PlanFlaggedCard].self, forKey: .offTheLine) ?? []
        landedNotClosed = try c.decodeIfPresent([PlanFlaggedCard].self, forKey: .landedNotClosed) ?? []
        agents = try c.decodeIfPresent([PlanAgent].self, forKey: .agents) ?? []
        agentsSpend = try c.decodeIfPresent(PlanSpend.self, forKey: .agentsSpend)
    }

    private enum CodingKeys: String, CodingKey {
        case nowMs, themes, lines, cards, rulings, landings, ci, boardCounts, cost, offTheLine, landedNotClosed, agents, agentsSpend
    }

    public static let empty = PlanModel()

    public static func decode(_ data: Data) throws -> PlanModel {
        try PlanJSON.decoder.decode(PlanModel.self, from: data)
    }

    /// Nothing planned: no theme and no ticket on the line. Rulings don't count: they
    /// never switch a board into its plan layout (review 1005a F1).
    public var isEmpty: Bool { themes.isEmpty && lines.isEmpty }

    /// Nothing at all to show: nothing planned and no ruling, so the Plan
    /// view's "Nothing is planned" notice stands alone.
    public var showsNothing: Bool { isEmpty && rulings.isEmpty }
}

/// One entry in a theme's or ticket's record, oldest first.
public struct PlanEvent: Decodable, Equatable, Sendable {
    public struct Extra: Decodable, Equatable, Sendable {
        /// A `story` event's new story; its `body` is the one it replaced.
        public var to: String?
        public var from: String?
    }

    public var at: Int64
    public var actor: String
    /// `story`, `state`, `plan`, `cards` or `agent`.
    public var kind: String
    public var body: String
    public var extra: Extra?
}

/// `plan theme show --json` and `plan ticket show --json`: the subject, with
/// its record. Only the record is read; the subject is the plan's.
public struct PlanRecord: Decodable, Equatable, Sendable {
    public var events: [PlanEvent]

    public static func decode(_ data: Data) throws -> PlanRecord {
        try PlanJSON.decoder.decode(PlanRecord.self, from: data)
    }
}

enum PlanJSON {
    static var decoder: JSONDecoder {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }
}

// MARK: - What the overview derives

extension PlanModel {
    /// Next Up: the waiting tickets the plan ranks, first is next.
    public var nextUp: [PlanTicketLine] {
        lines.filter { $0.stage == .waiting && $0.planRank != nil }
            .sorted { ($0.planRank ?? .max) < ($1.planRank ?? .max) }
    }

    /// Now: the tickets being built, reviewed, waiting on the owner or
    /// landed, in the order the line lists them.
    public var working: [PlanTicketLine] {
        lines.filter { $0.stage.isPastWaiting }
    }

    /// Waiting tickets the plan doesn't rank: written, but not yet ordered.
    public var unranked: [PlanTicketLine] {
        lines.filter { $0.stage == .waiting && $0.planRank == nil }
    }

    /// The tickets that landed on the runner's day, in `calendar`'s time zone,
    /// most recent first.
    public func landedToday(calendar: Calendar = .current) -> [PlanTicketLine] {
        let today = Date(timeIntervalSince1970: Double(nowMs) / 1000)
        return lines
            .filter {
                $0.stage == .landed
                    && calendar.isDate(Date(timeIntervalSince1970: Double($0.stageSince) / 1000), inSameDayAs: today)
            }
            .sorted { $0.stageSince > $1.stageSince }
    }

    /// The themes as the overview lists them: active, then paused, then
    /// done, each by its order on the board.
    public var shownThemes: [PlanTheme] {
        let rank = ["active": 0, "paused": 1, "done": 2]
        return themes.filter { $0.state != "dropped" }.sorted {
            let (a, b) = (rank[$0.state] ?? 3, rank[$1.state] ?? 3)
            return a != b ? a < b : $0.ordinal < $1.ordinal
        }
    }

    /// The theme a ticket serves: the one its card is in, the earlier on the
    /// board when two list it. Nil when none does.
    public func theme(of line: PlanTicketLine) -> PlanTheme? { theme(ofTask: line.task) }

    /// The tickets on the line for any of a theme's cards: live ones as
    /// they're listed, then waiting in plan order, then the finished.
    public func lines(in theme: PlanTheme) -> [PlanTicketLine] {
        let keys = Set(theme.cards.map(\.task))
        let mine = lines.filter { keys.contains($0.task) }
        func group(_ line: PlanTicketLine) -> Int {
            switch line.stage {
            case .waiting: 1
            case .landed, .dropped: 2
            default: 0
            }
        }
        return mine.enumerated().sorted { l, r in
            let (a, b) = (group(l.element), group(r.element))
            if a != b { return a < b }
            if a == 1 { return (l.element.planRank ?? .max, l.offset) < (r.element.planRank ?? .max, r.offset) }
            if a == 2 { return l.element.stageSince > r.element.stageSince }
            return l.offset < r.offset
        }.map(\.element)
    }

    /// The theme a card is in, if any.
    public func theme(ofTask task: String) -> PlanTheme? {
        themes.first { $0.state != "dropped" && $0.cards.contains { $0.task == task } }
    }

    /// The ticket a card is: its line, unless it was dropped.
    public func line(ofTask task: String) -> PlanTicketLine? {
        lines.first { $0.task == task && $0.stage != .dropped }
    }

    /// The ticket with this key, as a person types it.
    public func line(key: String) -> PlanTicketLine? {
        lines.first { $0.key.lowercased() == key.lowercased() }
    }

    /// The line under a task page's title (6.4), or nil when the card is on
    /// no line and in no theme.
    public func taskLine(_ task: String) -> PlanTaskLine? {
        let line = line(ofTask: task)
        let theme = theme(ofTask: task)
        guard line != nil || theme != nil else { return nil }
        let stage = line.flatMap { $0.stage.isLive ? $0.shownStage : nil }
        return PlanTaskLine(line: line, theme: theme, stage: stage)
    }

    /// Whether a live ticket is waiting on the owner: the runner says so, or
    /// its card needs a decision, by the board's own statuses. With
    /// staleness, the only thing that draws a ticket in color.
    public func waitsOnOwner(_ line: PlanTicketLine, statuses: [String: TaskStatus]) -> Bool {
        line.stage.isLive && (line.stage == .needsYou || statuses[line.task] == .needsDecision)
    }

    /// A card's row, from the plan's own read: for a card the board hasn't.
    public func card(_ task: String) -> PlanCard? { cards.first { $0.task == task } }

    /// The agent with this id.
    public func agent(_ id: String) -> PlanAgent? { agents.first { $0.id == id } }
}

/// "In Review · Visual language": where a card stands in the plan.
public struct PlanTaskLine: Equatable, Sendable {
    public var line: PlanTicketLine?
    public var theme: PlanTheme?
    /// The card's own pull request stage, while its ticket is live (ov-316).
    public var stage: PlanPrStage?

    /// The ticket's half: "Building", "Waiting · 2nd", "Landed · ac840108"; nil
    /// without a ticket on the line.
    public var stageWords: String? {
        guard let line else { return nil }
        return PlanWords.status(line, stage: false)
    }

    /// The whole line, as VoiceOver reads it.
    public var text: String { [stageWords, stage?.fullWords, theme?.name].compactMap { $0 }.joined(separator: " · ") }
}

// MARK: - Words

/// What the Plan view says. Title case for a state, as the board's statuses
/// are; no parenthesized counts.
public enum PlanWords {
    public static let nothingPlanned = "Nothing is planned on this board yet."
    public static let nothingPlannedDetail =
        "The orchestrator writes the plan: themes say why a group of cards exists, and agents work the tickets that make them up."
    /// Under it with no orchestrator, where the plan comes from: the job, then
    /// the first step.
    public static let nothingPlannedStart =
        "The orchestrator plans the work and starts agents on it. Start it, then tell it what you want built."
    /// With one running: the next step is a message to it.
    public static let nothingPlannedRunning =
        "Tell the orchestrator what you want built, and it writes the plan here. "
        + "Themes say why a group of cards exists, and agents work the tickets that make them up."
    /// Under "Nothing is waiting.": what moves work in, with an orchestrator, and
    /// without one.
    public static let nothingQueuedNext = "Tell the orchestrator which theme to move forward."
    public static let nothingQueuedStart = "Start the orchestrator, then tell it which theme to move forward."
    public static let needsUpdate = "This runner needs an update to keep a plan."
    public static let couldntRead = "Far Cooler couldn’t read this board’s plan."
    /// A read that got no answer because the runner is restarting or the link
    /// is down (ov-631): it isn't a broken plan, and it reads again by itself.
    public static let reconnecting = "Reconnecting to the runner…"
    /// The runner has no Far Cooler on it (a missing CLI): said at once, since
    /// waiting would not install it.
    public static let notInstalled = "Far Cooler isn’t installed on this runner."
    /// Over a plan already read, once the runner can't be reached: the plan on
    /// screen may be old, and Try Again asks again (ov-662).
    public static let staleUnreachable = "Couldn’t reach the runner. Showing the last plan."
    public static let staleNotInstalled = "Far Cooler isn’t installed on this runner. Showing the last plan."
    public static let notReported = TaskUsageFormat.notReported

    /// A ticket's stage alone.
    public static func stage(_ stage: TicketStage) -> String { stage.word }

    /// A ticket's stage with what it needs said beside it: "Fixing · round
    /// 1", "Landing · in Landing 8", "Waiting · 2nd", "Landed · ac840108".
    public static func status(_ line: PlanTicketLine) -> String { status(line, stage: true) }

    /// `status`, with or without the pull request's stage (ov-316): a row that
    /// draws the stage on a line of its own leaves it out here.
    public static func status(_ line: PlanTicketLine, stage: Bool) -> String {
        var parts = [line.isFixing ? "Fixing" : line.stage.word]
        // A pull request's stage says more than "In Review" or "Landing", so it
        // takes the stage's place; one the runner could not read is added, for
        // it says less than the stage does (ov-316).
        if let shown = line.shownStage, let words = shown.fullWords {
            if !stage {
                // A row that draws the stage on its own line keeps what the
                // stage leaves to the state ("Approved" beside "Checks Failing").
                if let kept = shown.stateWords { parts[0] = kept }
            } else if shown.kind == .unknown {
                parts.append(words)
            } else {
                parts[0] = words
            }
        }
        if line.isFixing, line.fixRounds > 0 { parts.append("round \(line.fixRounds)") }
        if line.stage == .waiting, let rank = line.planRank { parts.append(ordinal(rank)) }
        if line.stage == .landed, let sha = line.landedSha, !sha.isEmpty { parts.append(String(sha.prefix(8))) }
        if line.stage.isLive, let landing = line.landing { parts.append("in Landing \(landing)") }
        return parts.joined(separator: " · ")
    }

    /// "1st", "2nd", "3rd", "4th", … "11th", "12th", "13th", "21st".
    public static func ordinal(_ n: Int) -> String {
        let tens = n % 100
        if (11...13).contains(tens) { return "\(n)th" }
        switch n % 10 {
        case 1: return "\(n)st"
        case 2: return "\(n)nd"
        case 3: return "\(n)rd"
        default: return "\(n)th"
        }
    }

    /// "1 card", "5 cards".
    public static func cards(_ n: Int) -> String { n == 1 ? "1 card" : "\(n) cards" }

    /// A model as people say it: "opus" and "claude-opus-5-5" are "Opus".
    public static func model(_ raw: String) -> String {
        let lower = raw.lowercased()
        for family in ["opus", "sonnet", "haiku", "fable"] where lower.contains(family) {
            return family.prefix(1).uppercased() + family.dropFirst()
        }
        return raw
    }

    /// "4 of 18 done". Canceled cards count on neither side: they were
    /// neither left to do nor done.
    public static func progress(_ counts: PlanCounts) -> String {
        "\(counts.done) of \(total(counts)) done"
    }

    /// The cards a theme's progress counts: all but the canceled.
    public static func total(_ c: PlanCounts) -> Int {
        c.backlog + c.todo + c.needsDecision + c.inProgress + c.inReview + c.done
    }

    /// The theme's bar, left to right: done, in review, in progress, then
    /// what hasn't started (waiting on a decision, to do, backlog). Empty
    /// parts are left out.
    public static func segments(_ c: PlanCounts) -> [PlanSegment] {
        [
            PlanSegment(kind: .done, count: c.done),
            PlanSegment(kind: .inReview, count: c.inReview),
            PlanSegment(kind: .inProgress, count: c.inProgress),
            PlanSegment(kind: .notStarted, count: c.needsDecision + c.todo + c.backlog),
        ].filter { $0.count > 0 }
    }

    /// "3 done · 1 in progress · 6 backlog": a theme's cards by status, for
    /// its page, in the board's own words.
    public static func breakdown(_ c: PlanCounts) -> String {
        [
            (c.done, "done"), (c.inReview, "in review"), (c.inProgress, "in progress"),
            (c.needsDecision, "need a decision"), (c.todo, "to do"), (c.backlog, "backlog"),
        ].filter { $0.0 > 0 }.map { "\($0.0) \($0.1)" }.joined(separator: " · ")
    }

    /// "470K tokens", with "about $31 estimated" when a price is known and
    /// "2 agents not reported" for agents nobody has measured; "Not
    /// reported" with no tokens at all.
    public static func spend(_ s: PlanSpend, locale: Locale = .current) -> String {
        guard s.totalTokens > 0 else { return notReported }
        var parts = ["\(TaskUsageFormat.tokens(s.totalTokens, locale: locale)) tokens"]
        if let micros = s.costMicros, micros > 0 {
            parts.append("about \(TaskUsageFormat.dollars(micros, locale: locale)) estimated")
        }
        if s.unmeasuredAgents > 0 {
            parts.append(s.unmeasuredAgents == 1 ? "1 agent not reported" : "\(s.unmeasuredAgents) agents not reported")
        }
        if s.sharedAgents > 0 {
            parts.append(
                s.sharedAgents == 1
                    ? "1 agent’s spend split with other tickets" : "\(s.sharedAgents) agents’ spend split with other tickets")
        }
        return parts.joined(separator: " · ")
    }

    /// "0 fix rounds", "1 fix round".
    public static func fixRounds(_ n: Int) -> String { n == 1 ? "1 fix round" : "\(n) fix rounds" }

    /// "Builder Opus, 3 h": one turn at a ticket, the agent's role, model and
    /// how long it ran (to now while it's open).
    public static func work(_ w: PlanWork, agent: PlanAgent?, now: Int64) -> String {
        let role =
            switch w.role {
            case .review: "Reviewer"
            case .fix: "Fixer"
            case .land: "Lander"
            case .build, .unknown: "Builder"
            }
        let model = model(agent?.model ?? "")
        let ran = TaskUsageFormat.duration(ms: max(0, (w.endedAt ?? now) - w.startedAt))
        return model.isEmpty ? "\(role), \(ran)" : "\(role) \(model), \(ran)"
    }

    /// How long ago, coarsely: "just now", "5 min ago", "3 h ago", "2 d ago".
    public static func ago(_ ms: Int64, now: Int64) -> String {
        let minutes = max(0, now - ms) / 60_000
        switch minutes {
        case 0: return "just now"
        case 1..<60: return "\(minutes) min ago"
        case 60..<1440: return "\(minutes / 60) h ago"
        default: return "\(minutes / 1440) d ago"
        }
    }

    /// A stale ticket's warning, in words beside its mark: "No move in 2 h".
    public static func stale(_ line: PlanTicketLine, now: Int64) -> String? {
        guard line.stale else { return nil }
        let minutes = max(0, now - line.stageSince) / 60_000
        return minutes < 120 ? "No move in an hour" : "No move in \(minutes / 60) h"
    }
}

/// One part of a theme's bar.
public struct PlanSegment: Equatable, Sendable {
    public enum Kind: Sendable, CaseIterable { case done, inReview, inProgress, notStarted }
    public var kind: Kind
    public var count: Int
}

// MARK: - Records

/// One line of a theme's or ticket's timeline.
public struct PlanTimelineRow: Equatable, Sendable, Identifiable {
    public var id: Int
    public var at: Int64
    public var text: String
}

extension PlanRecord {
    /// The record as a timeline: newest last, with a run of cards added (or
    /// taken out) in one write said once, and a story rewrite said as one.
    public var timeline: [PlanTimelineRow] {
        var rows: [PlanTimelineRow] = []
        var run: (at: Int64, verb: String, keys: [String])?
        func flush() {
            guard let open = run else { return }
            rows.append(PlanTimelineRow(id: rows.count, at: open.at, text: Self.cardsSentence(open.verb, open.keys)))
            run = nil
        }
        for event in events {
            if event.kind == "cards", let (verb, key) = Self.cardChange(event.body) {
                if let open = run, open.verb == verb, event.at - open.at < 60_000 {
                    run?.keys.append(key)
                } else {
                    flush()
                    run = (event.at, verb, [key])
                }
                continue
            }
            flush()
            let text = event.kind == "story" ? "Rewrote where it stands." : event.body
            rows.append(PlanTimelineRow(id: rows.count, at: event.at, text: text))
        }
        flush()
        return rows
    }

    /// The story the last rewrite replaced, and when: "What Changed". Nil
    /// before the first rewrite.
    public var previousStory: (story: String, at: Int64)? {
        guard let last = events.last(where: { $0.kind == "story" }), !last.body.isEmpty else { return nil }
        return (last.body, last.at)
    }

    /// "Added ov-155." → ("Added", "ov-155").
    static func cardChange(_ body: String) -> (String, String)? {
        let words = body.split(separator: " ", maxSplits: 1).map(String.init)
        guard words.count == 2, ["Added", "Removed"].contains(words[0]), words[1].hasSuffix("."),
            !words[1].dropLast().contains(" ")
        else { return nil }
        return (words[0], String(words[1].dropLast()))
    }

    /// "Added ov-1.", "Added ov-1 and ov-2.", "Added ov-1, ov-2 and ov-3.",
    /// "Added ov-1, ov-2 and 8 more cards."
    static func cardsSentence(_ verb: String, _ keys: [String]) -> String {
        switch keys.count {
        case 1: return "\(verb) \(keys[0])."
        case 2: return "\(verb) \(keys[0]) and \(keys[1])."
        case 3: return "\(verb) \(keys[0]), \(keys[1]) and \(keys[2])."
        default: return "\(verb) \(keys[0]), \(keys[1]) and \(keys.count - 2) more cards."
        }
    }
}
