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
    /// How many finished cards it worked: the n behind every figure here.
    public var cards: Int
    public var tokens: UInt64
    /// Millionths of a dollar; nil unless every one of its turns was priced.
    public var costMicros: Int64?

    public var id: String { "\(harness)/\(model)" }

    public init(harness: String, model: String, cards: Int, tokens: UInt64, costMicros: Int64? = nil) {
        self.harness = harness
        self.model = model
        self.cards = cards
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

    public init(weekTokens: UInt64 = 0, compare: [PlanHarnessCost] = [], compareHeldBack: Int = 0) {
        self.weekTokens = weekTokens
        self.compare = compare
        self.compareHeldBack = compareHeldBack
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        weekTokens = try c.decodeIfPresent(UInt64.self, forKey: .weekTokens) ?? 0
        compare = try c.decodeIfPresent([PlanHarnessCost].self, forKey: .compare) ?? []
        compareHeldBack = try c.decodeIfPresent(Int.self, forKey: .compareHeldBack) ?? 0
    }

    private enum CodingKeys: String, CodingKey { case weekTokens, compare, compareHeldBack }

    /// Something to draw: tokens this week, or a comparison, or pairs held back.
    public var isWorthShowing: Bool { weekTokens > 0 || !compare.isEmpty || compareHeldBack > 0 }
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
    /// "5 finished cards · 1.5M tokens a card · about $2.50 a card".
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
    /// none, 160 thousand today. 320 thousand tokens in all."
    public static func trendSpoken(_ t: PlanTrend, locale: Locale = .current) -> String {
        let each = t.days.map { $0 == 0 ? "none" : spokenTokens($0, locale: locale) }
        var said = each.dropLast().joined(separator: ", ")
        said += ", \(each.last ?? "none") today"
        return "Last 7 days, oldest first: \(said). \(spokenTokens(t.total, locale: locale)) tokens in all."
    }

    /// "34M tokens in the last 7 days".
    public static func week(_ tokens: UInt64, locale: Locale = .current) -> String {
        "\(TaskUsageFormat.tokens(tokens, locale: locale)) tokens in the last 7 days"
    }

    /// Said once under the week: why there is no percentage.
    public static let weekNote =
        "Your plan’s weekly limit isn’t something your runner can read, so this counts tokens and shows no percentage."

    /// One comparison row. A token or dollar figure is per finished card.
    public static func compareRow(_ p: PlanHarnessCost, locale: Locale = .current) -> PlanCompareRow {
        let model = p.model.isEmpty ? "no model named" : p.model
        let cards = p.cards == 1 ? "1 finished card" : "\(p.cards) finished cards"
        let each = p.tokens / UInt64(max(p.cards, 1))
        var detail = [cards, "\(TaskUsageFormat.tokens(each, locale: locale)) tokens a card"]
        var spoken = [cards, "\(spokenTokens(each, locale: locale)) tokens a card"]
        if let micros = p.costMicros, micros > 0 {
            let dollars = TaskUsageFormat.dollars(micros / Int64(max(p.cards, 1)), locale: locale)
            detail.append("about \(dollars) a card")
            spoken.append("about \(dollars) a card, API-equivalent")
        }
        let title = "\(harnessName(p.harness)) · \(model)"
        return PlanCompareRow(
            id: p.id, title: title, detail: detail.joined(separator: " · "),
            spoken: "\(harnessName(p.harness)), \(model). \(spoken.joined(separator: ", "))")
    }

    /// "2 other pairs held back until three cards have finished", or nil.
    public static func heldBack(_ n: Int) -> String? {
        guard n > 0 else { return nil }
        let pairs = n == 1 ? "1 other harness and model pair" : "\(n) other harness and model pairs"
        return "\(pairs) held back until three cards have finished"
    }

    /// Said under the comparison: what the numbers are, and that dollars are
    /// API-equivalent.
    public static let compareNote =
        "Finished cards only, by the harness and model that worked them. A card two models worked counts once for each."
}
