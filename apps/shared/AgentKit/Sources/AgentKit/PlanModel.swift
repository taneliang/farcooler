import Foundation

// The plan layer (ov-268), as a client reads it: themes (why a group of cards
// exists), lanes (one unit of execution) and the plan (the queued lanes, first
// is next up). EXPERIMENTAL, behind the runner's `board_plan` capability, and
// removable: nothing on a task points here, and nothing outside the Plan view
// and the one line on a task page reads it.
//
// The Mac reads `farcooler plan --json` (`crates/cli/src/plan.rs`,
// `plan_json`), and a theme's or lane's timeline from `plan theme show` and
// `plan lane show`, which add `events`. Every rule the Plan view draws is here,
// so `swift test --package-path apps/shared/AgentKit` reaches it; the views
// only lay it out. The design is `.claude/agent/reports/ov-268/design.md`, 6.1
// to 6.4.

/// A card a theme or lane names, by id and key; a lane's may name a slice.
public struct PlanCardRef: Decodable, Equatable, Hashable, Sendable {
    public var task: String
    public var key: String
    /// Empty when the lane works the whole card. Absent on a theme's.
    public var slice: String

    public init(task: String, key: String, slice: String = "") {
        self.task = task
        self.key = key
        self.slice = slice
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        task = try c.decode(String.self, forKey: .task)
        key = try c.decodeIfPresent(String.self, forKey: .key) ?? ""
        slice = try c.decodeIfPresent(String.self, forKey: .slice) ?? ""
    }

    private enum CodingKeys: String, CodingKey { case task, key, slice }
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
    /// What its lanes spent on its cards, each lane's spend shared over its
    /// cards (ov-306). Nil from a runner before it.
    public var spend: PlanSpend?
}

/// Where a lane is. Moves go forward, with two loops back to fixing.
public enum LaneState: String, Decodable, Sendable, CaseIterable {
    case queued, building, review, fixing, landing, landed, dropped, unknown

    public init(from decoder: Decoder) throws {
        self = LaneState(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
    }

    /// Neither landed nor dropped.
    public var isLive: Bool { self != .landed && self != .dropped }
}

public struct PlanAgent: Decodable, Equatable, Sendable {
    public var harness: String
    public var agentId: String
    /// `build`, `review` or `fix`.
    public var role: String
    public var model: String
    public var startedAt: Int64
    public var endedAt: Int64?
}

/// What a lane's agents spent. An agent the runner has read no turn of is
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
    /// Agents also recorded on another lane, whose spend is split evenly
    /// across their lanes: the figures hold this lane's part.
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

public struct PlanLane: Decodable, Equatable, Identifiable, Sendable {
    public var id: String
    public var short: String
    public var name: String
    public var state: LaneState
    /// One line: why it's in the plan, or why it's where it is now.
    public var reason: String
    /// 1 is next up; nil outside the plan.
    public var planRank: Int?
    public var worktreePath: String
    public var branch: String
    public var harness: String
    public var model: String
    public var train: String?
    public var landedSha: String?
    public var stateSince: Int64
    /// Sat in one live state for over an hour, by the runner's clock.
    public var stale: Bool
    public var fixRounds: Int
    public var cards: [PlanCardRef]
    public var agents: [PlanAgent]
    public var spend: PlanSpend
}

/// A card's row, as the plan read it: what a theme page lists for a card
/// the board hasn't read. `status` is the CLI's word for it, for showing.
public struct PlanCard: Decodable, Equatable, Sendable {
    public var task: String
    public var key: String
    public var title: String
    public var status: String
}

/// One read of a board's plan: every Plan surface draws from it.
public struct PlanModel: Decodable, Equatable, Sendable {
    /// The runner's clock when it answered.
    public var nowMs: Int64
    public var themes: [PlanTheme]
    /// Live lanes, and the ones that finished in the last week.
    public var lanes: [PlanLane]
    /// The queued lanes in the plan, by id; first is next up.
    public var order: [String]
    public var cards: [PlanCard]
    /// Decided for you (ov-304): standing rulings newest first, then the
    /// settled ones, in the runner's order. Empty from a runner without
    /// `board_rulings`, and absent from an older CLI's answer.
    public var rulings: [PlanRuling]
    /// Trains (ov-309): those not landed or dropped, oldest first, then the
    /// settled ones. Empty from a runner without `board_trains`.
    public var trains: [PlanTrain]
    /// What the runner last read of CI for each subject the board names: its
    /// trains' pushed SHAs and its pages' CI references (ov-306).
    public var ci: [PlanCIRead]
    /// How many of the board's cards are in each status (ov-306), what a
    /// page's card counts draw; nil from a runner before it.
    public var boardCounts: PlanCounts?

    public init(
        nowMs: Int64 = 0, themes: [PlanTheme] = [], lanes: [PlanLane] = [], order: [String] = [],
        cards: [PlanCard] = [], rulings: [PlanRuling] = [], trains: [PlanTrain] = [], ci: [PlanCIRead] = [],
        boardCounts: PlanCounts? = nil
    ) {
        self.nowMs = nowMs
        self.themes = themes
        self.lanes = lanes
        self.order = order
        self.cards = cards
        self.rulings = rulings
        self.trains = trains
        self.ci = ci
        self.boardCounts = boardCounts
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        nowMs = try c.decode(Int64.self, forKey: .nowMs)
        themes = try c.decode([PlanTheme].self, forKey: .themes)
        lanes = try c.decode([PlanLane].self, forKey: .lanes)
        order = try c.decode([String].self, forKey: .order)
        cards = try c.decode([PlanCard].self, forKey: .cards)
        rulings = try c.decodeIfPresent([PlanRuling].self, forKey: .rulings) ?? []
        trains = try c.decodeIfPresent([PlanTrain].self, forKey: .trains) ?? []
        ci = try c.decodeIfPresent([PlanCIRead].self, forKey: .ci) ?? []
        boardCounts = try c.decodeIfPresent(PlanCounts.self, forKey: .boardCounts)
    }

    private enum CodingKeys: String, CodingKey { case nowMs, themes, lanes, order, cards, rulings, trains, ci, boardCounts }

    public static let empty = PlanModel()

    public static func decode(_ data: Data) throws -> PlanModel {
        try PlanJSON.decoder.decode(PlanModel.self, from: data)
    }

    /// Nothing planned: no theme and no lane. Rulings don't count: they
    /// never switch a board into its plan layout (review 1005a F1).
    public var isEmpty: Bool { themes.isEmpty && lanes.isEmpty }

    /// Nothing at all to show: nothing planned and no ruling, so the Plan
    /// view's "Nothing is planned" notice stands alone.
    public var showsNothing: Bool { isEmpty && rulings.isEmpty }
}

/// One entry in a theme's or lane's record, oldest first.
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

/// `plan theme show --json` and `plan lane show --json`: the subject, with
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
    /// Next Up: the plan's queued lanes, first is next.
    public var nextUp: [PlanLane] {
        order.compactMap { id in lanes.first { $0.id == id && $0.state == .queued } }
    }

    /// Now: the lanes being built, reviewed, fixed or landed, in the order
    /// they started.
    public var working: [PlanLane] {
        lanes.filter { $0.state.isLive && $0.state != .queued }
    }

    /// Queued lanes the plan doesn't rank: written, but not yet ordered.
    public var unranked: [PlanLane] {
        lanes.filter { $0.state == .queued && !order.contains($0.id) }
    }

    /// The lanes that landed on the runner's day, in `calendar`'s time zone,
    /// most recent first.
    public func landedToday(calendar: Calendar = .current) -> [PlanLane] {
        let today = Date(timeIntervalSince1970: Double(nowMs) / 1000)
        return lanes
            .filter {
                $0.state == .landed
                    && calendar.isDate(Date(timeIntervalSince1970: Double($0.stateSince) / 1000), inSameDayAs: today)
            }
            .sorted { $0.stateSince > $1.stateSince }
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

    /// The theme a lane serves: the one most of its cards are in, the
    /// earlier on the board when two tie. Nil when none of them is in one.
    public func theme(of lane: PlanLane) -> PlanTheme? {
        var best: (theme: PlanTheme, count: Int)?
        for theme in shownThemes {
            let keys = Set(theme.cards.map(\.task))
            let count = lane.cards.filter { keys.contains($0.task) }.count
            if count > 0, count > (best?.count ?? 0) { best = (theme, count) }
        }
        return best?.theme
    }

    /// The lanes working any of a theme's cards: live ones as they're
    /// listed, then queued in plan order, then the finished.
    public func lanes(in theme: PlanTheme) -> [PlanLane] {
        let keys = Set(theme.cards.map(\.task))
        let mine = lanes.filter { lane in lane.cards.contains { keys.contains($0.task) } }
        func group(_ lane: PlanLane) -> Int {
            switch lane.state {
            case .queued: 1
            case .landed, .dropped: 2
            default: 0
            }
        }
        return mine.enumerated().sorted { l, r in
            let (a, b) = (group(l.element), group(r.element))
            if a != b { return a < b }
            if a == 1 { return (l.element.planRank ?? .max, l.offset) < (r.element.planRank ?? .max, r.offset) }
            if a == 2 { return l.element.stateSince > r.element.stateSince }
            return l.offset < r.offset
        }.map(\.element)
    }

    /// The theme a card is in, if any.
    public func theme(ofTask task: String) -> PlanTheme? {
        themes.first { $0.state != "dropped" && $0.cards.contains { $0.task == task } }
    }

    /// The lane a card's page names: one working it now, else one queued for
    /// it, else the last that landed it.
    public func lane(ofTask task: String) -> PlanLane? {
        let mine = lanes.filter { lane in lane.cards.contains { $0.task == task } && lane.state != .dropped }
        return mine.first { $0.state.isLive && $0.state != .queued }
            ?? mine.filter { $0.state == .queued }.min { ($0.planRank ?? .max) < ($1.planRank ?? .max) }
            ?? mine.filter { $0.state == .landed }.max { $0.stateSince < $1.stateSince }
    }

    /// The line under a task page's title (6.4), or nil when the card is in
    /// no lane and no theme.
    public func taskLine(_ task: String) -> PlanTaskLine? {
        let lane = lane(ofTask: task)
        let theme = theme(ofTask: task)
        guard lane != nil || theme != nil else { return nil }
        return PlanTaskLine(lane: lane, theme: theme)
    }

    /// Whether a live lane is waiting on the owner: one of its cards needs
    /// a decision, by the board's own statuses. With staleness, the only
    /// thing that draws a lane in color.
    public func waitsOnOwner(_ lane: PlanLane, statuses: [String: TaskStatus]) -> Bool {
        lane.state.isLive && lane.cards.contains { statuses[$0.task] == .needsDecision }
    }

    /// A card's row, from the plan's own read: for a card the board hasn't.
    public func card(_ task: String) -> PlanCard? { cards.first { $0.task == task } }
}

/// "In lane mac-ux · Visual language": where a card stands in the plan.
public struct PlanTaskLine: Equatable, Sendable {
    public var lane: PlanLane?
    public var theme: PlanTheme?

    /// The lane's half: "In lane mac-ux", "Queued in mac-fu3", "Landed in
    /// land-129"; nil without a lane.
    public var laneWords: String? {
        guard let lane else { return nil }
        switch lane.state {
        case .queued: return "Queued in \(lane.name)"
        case .landed: return "Landed in \(lane.name)"
        default: return "In lane \(lane.name)"
        }
    }

    /// The whole line, as VoiceOver reads it.
    public var text: String { [laneWords, theme?.name].compactMap { $0 }.joined(separator: " · ") }
}

// MARK: - Words

/// What the Plan view says. Title case for a state, as the board's statuses
/// are; no parenthesized counts.
public enum PlanWords {
    public static let nothingPlanned = "Nothing is planned on this board yet."
    public static let nothingPlannedDetail =
        "The orchestrator writes the plan: themes say why a group of cards exists, and lanes are the agents working them."
    public static let needsUpdate = "This runner needs an update to keep a plan."
    public static let couldntRead = "Far Cooler couldn’t read this board’s plan."
    public static let notReported = TaskUsageFormat.notReported

    /// A lane's state alone.
    public static func state(_ state: LaneState) -> String {
        switch state {
        case .queued: "Queued"
        case .building: "Building"
        case .review: "In Review"
        case .fixing: "Fixing"
        case .landing: "Landing"
        case .landed: "Landed"
        case .dropped: "Dropped"
        case .unknown: "Unknown"
        }
    }

    /// A lane's state with what it needs said beside it: "Fixing · round
    /// 1", "Landing · in integ-8", "Queued · 2nd", "Landed · ac840108".
    public static func status(_ lane: PlanLane) -> String {
        var parts = [state(lane.state)]
        if lane.state == .fixing, lane.fixRounds > 0 { parts.append("round \(lane.fixRounds)") }
        if lane.state == .queued, let rank = lane.planRank { parts.append(ordinal(rank)) }
        if lane.state == .landed, let sha = lane.landedSha, !sha.isEmpty { parts.append(String(sha.prefix(8))) }
        if lane.state.isLive, let train = lane.train, !train.isEmpty { parts.append("in \(train)") }
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
                    ? "1 agent’s spend split with other lanes" : "\(s.sharedAgents) agents’ spend split with other lanes")
        }
        return parts.joined(separator: " · ")
    }

    /// "0 fix rounds", "1 fix round".
    public static func fixRounds(_ n: Int) -> String { n == 1 ? "1 fix round" : "\(n) fix rounds" }

    /// "Builder Opus, 3 h": one agent, its role, model and how long it ran
    /// (to now while it's open).
    public static func agent(_ a: PlanAgent, now: Int64) -> String {
        let role =
            switch a.role {
            case "review": "Reviewer"
            case "fix": "Fixer"
            default: "Builder"
            }
        let model = model(a.model)
        let ran = TaskUsageFormat.duration(ms: max(0, (a.endedAt ?? now) - a.startedAt))
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

    /// A stale lane's warning, in words beside its mark: "No move in 2 h".
    public static func stale(_ lane: PlanLane, now: Int64) -> String? {
        guard lane.stale else { return nil }
        let minutes = max(0, now - lane.stateSince) / 60_000
        return minutes < 120 ? "No move in an hour" : "No move in \(minutes / 60) h"
    }

    /// A lane's cards, by key, with each slice: "ov-113 s3-4 phones".
    public static func cardKeys(_ lane: PlanLane) -> String {
        lane.cards.map { $0.slice.isEmpty ? $0.key : "\($0.key) \($0.slice)" }.joined(separator: ", ")
    }
}

/// One part of a theme's bar.
public struct PlanSegment: Equatable, Sendable {
    public enum Kind: Sendable, CaseIterable { case done, inReview, inProgress, notStarted }
    public var kind: Kind
    public var count: Int
}

// MARK: - Records

/// One line of a theme's or lane's timeline.
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
