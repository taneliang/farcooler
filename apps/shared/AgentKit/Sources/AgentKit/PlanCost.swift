import Foundation

// Cost on the plan (ov-307, folding in ov-261): a token budget on a theme or a
// lane, a theme's seven-day trend, the runner's tokens this week, and cost per
// finished card by harness and model. EXPERIMENTAL with the plan layer, behind
// `board_cost`: a runner without it sends none of this, and nothing here draws.
//
// Tokens come first and dollars second, and every dollar is API-equivalent. A
// budget is a number of tokens counted as a lane's spend counts them (input,
// output and both cache counts), so what the plan flags over budget is what
// the plan prints as "6.1M tokens".
//
// **There is no weekly-limit percentage.** The runner can't know a plan's
// weekly limit: neither Claude Code nor Codex reports it, or how much of it is
// used, to the runner. The week is a count of tokens and says it has no limit.
// Never derive a percentage from a guessed limit.
//
// The Mac reads `farcooler plan --json` and the phones the client's `plan_json`,
// both held to `test/fixtures/plan.json`.

/// The fewest finished cards a harness and model need before the runner will
/// compare them. The runner holds the rest back and counts them, so every
/// client agrees; this is only the number a note says.
public let planCompareMinimumCards = 3

/// What the finished cards a harness and model worked cost.
public struct PlanHarnessCost: Decodable, Equatable, Identifiable, Sendable {
    public var harness: String
    /// Empty when the harness named no model.
    public var model: String
    /// Its share of the landed cards, in thousandths of a card: the n behind
    /// every figure here. A card two pairs worked is one card in total, shared
    /// out by each pair's fraction of its tokens.
    public var cardShareMilli: Int
    /// Tokens it spent on landed cards.
    public var tokens: UInt64
    /// Millionths of a dollar it spent on landed cards; nil unless every one
    /// of its turns was priced.
    public var costMicros: Int64?

    public var id: String { "\(harness)/\(model)" }

    public init(harness: String, model: String, cardShareMilli: Int, tokens: UInt64, costMicros: Int64? = nil) {
        self.harness = harness
        self.model = model
        self.cardShareMilli = cardShareMilli
        self.tokens = tokens
        self.costMicros = costMicros
    }
}

/// The week and the comparison, as one plan read carries them.
public struct PlanCostRead: Decodable, Equatable, Sendable {
    /// The runner's tokens over the last seven days, every harness and board.
    public var weekTokens: UInt64
    /// Harness and model pairs with enough finished cards to compare, most
    /// cards first.
    public var compare: [PlanHarnessCost]
    /// How many pairs had finished cards but too few to compare.
    public var compareHeldBack: Int
    /// Spend on the board's cards that haven't landed (open or cancelled),
    /// apart from the comparison: never charged to a landed card, never dropped.
    public var inFlightTokens: UInt64
    /// Millionths of a dollar of it; nil unless every turn was priced.
    public var inFlightCostMicros: Int64?

    public init(
        weekTokens: UInt64 = 0, compare: [PlanHarnessCost] = [], compareHeldBack: Int = 0, inFlightTokens: UInt64 = 0,
        inFlightCostMicros: Int64? = nil
    ) {
        self.weekTokens = weekTokens
        self.compare = compare
        self.compareHeldBack = compareHeldBack
        self.inFlightTokens = inFlightTokens
        self.inFlightCostMicros = inFlightCostMicros
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        weekTokens = try c.decodeIfPresent(UInt64.self, forKey: .weekTokens) ?? 0
        compare = try c.decodeIfPresent([PlanHarnessCost].self, forKey: .compare) ?? []
        compareHeldBack = try c.decodeIfPresent(Int.self, forKey: .compareHeldBack) ?? 0
        inFlightTokens = try c.decodeIfPresent(UInt64.self, forKey: .inFlightTokens) ?? 0
        inFlightCostMicros = try c.decodeIfPresent(Int64.self, forKey: .inFlightCostMicros)
    }

    private enum CodingKeys: String, CodingKey { case weekTokens, compare, compareHeldBack, inFlightTokens, inFlightCostMicros }

    /// Something to draw: tokens this week, or a comparison, or pairs held back.
    public var isWorthShowing: Bool { weekTokens > 0 || !compare.isEmpty || compareHeldBack > 0 || inFlightTokens > 0 }
}

/// Where spend stands against a budget. Equal is within it.
public enum PlanBudget: Equatable, Sendable {
    case within(used: UInt64, budget: UInt64)
    case over(used: UInt64, budget: UInt64)

    public var isOver: Bool {
        if case .over = self { return true }
        return false
    }
}

/// A theme's seven days as bars: each as a share of the busiest day.
public struct PlanTrend: Equatable, Sendable {
    public var days: [UInt64]
    public var total: UInt64 { days.reduce(0, +) }
    public var peak: UInt64 { days.max() ?? 0 }

    /// Each day's height from 0 to 1, against the busiest day. A day with
    /// anything in it never rounds away: it keeps a sliver.
    public func heights(sliver: Double = 0.08) -> [Double] {
        guard peak > 0 else { return days.map { _ in 0 } }
        return days.map { $0 == 0 ? 0 : max(sliver, Double($0) / Double(peak)) }
    }
}

/// One row of the comparison: the pair, then what its finished cards cost.
public struct PlanCompareRow: Equatable, Identifiable, Sendable {
    public var id: String
    /// "Claude Code · opus".
    public var title: String
    /// "5 finished cards · 1.5M tokens a card · about $2.50 a card API-equivalent".
    public var detail: String
    /// For VoiceOver and TalkBack: the same, with the numbers said in words.
    public var spoken: String
}

extension PlanWords {
    /// A harness as a person names it.
    public static func harnessName(_ harness: String) -> String {
        switch harness {
        case "claude": "Claude Code"
        case "codex": "Codex"
        case "cursor": "Cursor"
        case "opencode": "opencode"
        case "": "Unknown"
        default: harness.prefix(1).uppercased() + harness.dropFirst()
        }
    }

    /// Where `spend` stands against `budget`; nil without a budget.
    public static func budget(_ spend: PlanSpend?, against budget: UInt64?) -> PlanBudget? {
        guard let budget else { return nil }
        let used = spend?.totalTokens ?? 0
        return used > budget ? .over(used: used, budget: budget) : .within(used: used, budget: budget)
    }

    /// The warning a lane over its budget carries, in the words a row draws
    /// in amber; nil when it has no budget or is within it.
    public static func overBudget(_ lane: PlanLane, locale: Locale = .current) -> String? {
        guard let b = budget(lane.spend, against: lane.budgetTokens), b.isOver else { return nil }
        return budgetLine(b, locale: locale)
    }

    /// The same, said for VoiceOver with the counts as words.
    public static func overBudgetSpoken(_ lane: PlanLane, locale: Locale = .current) -> String? {
        guard let b = budget(lane.spend, against: lane.budgetTokens), b.isOver else { return nil }
        return budgetSpoken(b, locale: locale)
    }

    /// The same for a theme.
    public static func overBudget(_ theme: PlanTheme, locale: Locale = .current) -> PlanBudget? {
        guard let b = budget(theme.spend, against: theme.budgetTokens), b.isOver else { return nil }
        return b
    }

    /// "1.2M of 5M tokens budgeted", or "Over budget: 6.1M of 5M tokens".
    public static func budgetLine(_ b: PlanBudget, locale: Locale = .current) -> String {
        switch b {
        case .within(let used, let budget):
            "\(TaskUsageFormat.tokens(used, locale: locale)) of \(TaskUsageFormat.tokens(budget, locale: locale)) tokens budgeted"
        case .over(let used, let budget):
            "Over budget: \(TaskUsageFormat.tokens(used, locale: locale)) of \(TaskUsageFormat.tokens(budget, locale: locale)) tokens"
        }
    }

    /// The same for VoiceOver and TalkBack, with the counts said as words:
    /// "Over budget. 6.1 million of 5 million tokens used."
    public static func budgetSpoken(_ b: PlanBudget, locale: Locale = .current) -> String {
        switch b {
        case .within(let used, let budget):
            "\(spokenTokens(used, locale: locale)) of \(spokenTokens(budget, locale: locale)) tokens budgeted. Within budget."
        case .over(let used, let budget):
            "Over budget. \(spokenTokens(used, locale: locale)) of \(spokenTokens(budget, locale: locale)) tokens used."
        }
    }

    /// "6.1 million", "340 thousand", "812": a count a screen reader says whole
    /// rather than spelling a suffix letter.
    public static func spokenTokens(_ n: UInt64, locale: Locale = .current) -> String {
        let short = TaskUsageFormat.tokens(n, locale: locale)
        for (suffix, word) in [("K", " thousand"), ("M", " million"), ("B", " billion")] where short.hasSuffix(suffix) {
            return short.dropLast() + word
        }
        return short
    }

    /// The seven days as bars, or nil when the runner sent none or all are
    /// empty (a bar chart of nothing says nothing).
    public static func trend(_ days: [UInt64]?) -> PlanTrend? {
        guard let days, days.count == 7, days.contains(where: { $0 > 0 }) else { return nil }
        return PlanTrend(days: days)
    }

    /// "Last 7 days, oldest first: none, none, 40 thousand, 120 thousand, none,
    /// none, 160 thousand today. 320 thousand tokens in all." The days are UTC.
    public static func trendSpoken(_ t: PlanTrend, locale: Locale = .current) -> String {
        let each = t.days.map { $0 == 0 ? "none" : spokenTokens($0, locale: locale) }
        var said = each.dropLast().joined(separator: ", ")
        said += ", \(each.last ?? "none") today"
        return "Last 7 days by UTC day, oldest first: \(said). \(spokenTokens(t.total, locale: locale)) tokens in all."
    }

    /// "34M tokens in the last 7 days on this runner": the same seven UTC days
    /// as the trend, today so far.
    public static func week(_ tokens: UInt64, locale: Locale = .current) -> String {
        "\(TaskUsageFormat.tokens(tokens, locale: locale)) tokens in the last 7 days on this runner"
    }

    /// Said once under the week: why there is no percentage, and which days.
    public static let weekNote =
        "Counted by UTC day. Your plan’s weekly limit isn’t something your runner can read, so this counts tokens and shows no percentage."

    /// What is said where a dollar figure can't be: API-equivalent dollars the
    /// runner has no price for.
    public static let dollarsNotReported = "API-equivalent dollars: Not reported"

    /// "4.2 finished cards", or "4 finished cards": a pair's share of the
    /// landed cards, with its decimal when it isn't whole.
    public static func cardShare(_ milli: Int, locale: Locale = .current) -> String {
        if milli % 1000 == 0 { return "\(milli / 1000) finished cards" }
        let formatter = NumberFormatter()
        formatter.locale = locale
        formatter.minimumFractionDigits = 1
        formatter.maximumFractionDigits = 1
        return "\(formatter.string(from: NSNumber(value: Double(milli) / 1000)) ?? "\(milli / 1000)") finished cards"
    }

    /// One comparison row: cost per landed card is the pair's spend on landed
    /// cards over its share of them. Dollars are API-equivalent, or Not reported.
    public static func compareRow(_ p: PlanHarnessCost, locale: Locale = .current) -> PlanCompareRow {
        let model = p.model.isEmpty ? "no model named" : p.model
        let share = max(p.cardShareMilli, 1)
        let cards = cardShare(p.cardShareMilli, locale: locale)
        let each = p.tokens * 1000 / UInt64(share)
        var detail = [cards, "\(TaskUsageFormat.tokens(each, locale: locale)) tokens a card"]
        var spoken = [cards, "\(spokenTokens(each, locale: locale)) tokens a card"]
        if let micros = p.costMicros {
            let dollars = TaskUsageFormat.dollars(micros * 1000 / Int64(share), locale: locale)
            detail.append("about \(dollars) a card API-equivalent")
            spoken.append("about \(dollars) a card, API-equivalent")
        } else {
            detail.append(dollarsNotReported)
            spoken.append(dollarsNotReported)
        }
        let title = "\(harnessName(p.harness)) · \(model)"
        return PlanCompareRow(
            id: p.id, title: title, detail: detail.joined(separator: " · "),
            spoken: "\(harnessName(p.harness)), \(model). \(spoken.joined(separator: ", "))")
    }

    /// "1.2M tokens on cards that haven't landed · about $9 API-equivalent", or
    /// Not reported for the dollars; nil with nothing in flight.
    public static func inFlight(_ cost: PlanCostRead, locale: Locale = .current) -> String? {
        guard cost.inFlightTokens > 0 else { return nil }
        let tokens = TaskUsageFormat.tokens(cost.inFlightTokens, locale: locale)
        let dollars = cost.inFlightCostMicros.map { "about \(TaskUsageFormat.dollars($0, locale: locale)) API-equivalent" }
        return "\(tokens) tokens on cards that haven’t landed · \(dollars ?? dollarsNotReported)"
    }

    /// "2 other harness and model pairs held back until three cards have landed", or nil.
    public static func heldBack(_ n: Int) -> String? {
        guard n > 0 else { return nil }
        let pairs = n == 1 ? "1 other harness and model pair" : "\(n) other harness and model pairs"
        return "\(pairs) held back until three cards have landed"
    }

    /// Said under the comparison: what the numbers are.
    public static let compareNote =
        "Cost per landed card: what a harness and model spent on cards that landed, over its share of them. A card two models worked is one card, shared by their tokens. Dollars are API-equivalent."
}
