import Foundation

// What agents spent on one task (ov-195): the Usage section of a task's
// Overview on the Mac and its screen on iOS.
//
// The runner answers `usage.task` (the phone's route) and `farcooler task
// usage --json` (the Mac's) in one shape, `farcooler_core::usage_words::TaskSpend`,
// and this says it in the same words the CLI's `farcooler report` does and
// Android's `TaskUsage` does. All three are pinned by
// `test/fixtures/task-usage.json`.
//
// Every dollar is API-equivalent: the agent's own figure and the runner's
// price table are both API list prices, notional on a subscription and what
// was paid on an API key, and nothing here can tell which. A cost made partly
// from the table says "estimated" or "partly estimated", one with tokens
// nobody can price says "partly not reported", and a cost nobody knows is
// "Not reported", never a guess.

/// Spend over some turns, as the runner sums it.
public struct TaskSpend: Decodable, Equatable, Sendable {
    public var turns: UInt64 = 0
    /// Turns whose tokens are a floor: some model call stated none.
    public var turnsPartial: UInt64 = 0
    /// Turns that stated no usage at all.
    public var turnsNotReported: UInt64 = 0
    public var subagentRuns: UInt64 = 0
    public var activeMs: Int64 = 0
    public var inputTokens: UInt64 = 0
    public var outputTokens: UInt64 = 0
    public var cacheReadTokens: UInt64 = 0
    public var cacheWriteTokens: UInt64 = 0
    /// Millionths of a dollar the agents reported.
    public var costReportedMicros: Int64 = 0
    /// Millionths of a dollar from the price table.
    public var costEstimatedMicros: Int64 = 0
    /// Tokens with no known price.
    public var unpricedTokens: UInt64 = 0

    public init() {}

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        func n(_ key: CodingKeys) throws -> UInt64 { try c.decodeIfPresent(UInt64.self, forKey: key) ?? 0 }
        func i(_ key: CodingKeys) throws -> Int64 { try c.decodeIfPresent(Int64.self, forKey: key) ?? 0 }
        turns = try n(.turns)
        turnsPartial = try n(.turnsPartial)
        turnsNotReported = try n(.turnsNotReported)
        subagentRuns = try n(.subagentRuns)
        activeMs = try i(.activeMs)
        inputTokens = try n(.inputTokens)
        outputTokens = try n(.outputTokens)
        cacheReadTokens = try n(.cacheReadTokens)
        cacheWriteTokens = try n(.cacheWriteTokens)
        costReportedMicros = try i(.costReportedMicros)
        costEstimatedMicros = try i(.costEstimatedMicros)
        unpricedTokens = try n(.unpricedTokens)
    }

    private enum CodingKeys: String, CodingKey {
        case turns, turnsPartial, turnsNotReported, subagentRuns, activeMs, inputTokens, outputTokens
        case cacheReadTokens, cacheWriteTokens, costReportedMicros, costEstimatedMicros, unpricedTokens
    }

    public var totalTokens: UInt64 { inputTokens + outputTokens + cacheReadTokens + cacheWriteTokens }
    /// Nothing recorded: no turn, and no subagent run.
    public var isEmpty: Bool { turns == 0 && subagentRuns == 0 }
    var pricedMicros: Int64 { costReportedMicros + costEstimatedMicros }
    var partlyUnknown: Bool { unpricedTokens > 0 || turnsPartial > 0 || turnsNotReported > 0 }
}

/// One harness and model's share.
public struct TaskSpendRow: Decodable, Equatable, Sendable, Identifiable {
    public var harness: String
    /// Empty when the harness named no model.
    public var model: String
    public var totals: TaskSpend

    /// The runner keys a task's rows by harness and model, and files a turn
    /// that named no model under "", so this is unique within a task.
    public var id: String { "\(harness)\u{1F}\(model)" }
}

/// A task's spend: its totals and the same by harness and model.
public struct TaskUsage: Decodable, Equatable, Sendable {
    public var task: String
    public var priceTable: String
    public var totals: TaskSpend
    public var byHarnessModel: [TaskSpendRow]

    public static func decode(_ data: Data) throws -> TaskUsage {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(TaskUsage.self, from: data)
    }

    /// The breakdown, most tokens first, then by title.
    public var rows: [TaskSpendRow] {
        byHarnessModel.sorted {
            let (a, b) = ($0.totals.totalTokens, $1.totals.totalTokens)
            return a != b ? a > b : TaskUsageFormat.title($0) < TaskUsageFormat.title($1)
        }
    }
}

/// The words a Usage section says.
public enum TaskUsageFormat {
    /// What an empty section says.
    public static let nothingYet = "No agent usage recorded yet."
    public static let notReported = "Not reported"
    /// What a runner too old to record spend gets.
    public static let needsUpdate = "This runner needs an update to show spend."
    /// What a read that didn't come back gets, beside `tryAgain`.
    public static let couldntRead = "Far Cooler couldn’t read this task’s usage."
    /// The button beside `couldntRead`: the Apple platforms' own label, in
    /// title case. Not in the shared fixture, which holds sentences only.
    public static let tryAgain = "Try Again"
    /// What "API-equivalent" means, said once under the spend total.
    public static let apiEquivalent =
        "API-equivalent: what these tokens would cost at API list prices. On a subscription plan, you pay your plan’s price instead."

    /// Whether a cost line has a dollar figure, and so wants `apiEquivalent`
    /// beneath it.
    public static func isPriced(_ s: TaskSpend) -> Bool { s.pricedMicros > 0 }

    /// A token count, short, in `locale`'s digits: "999", "1.2K", "12K",
    /// "1M", "2.1B". One decimal below ten of a unit, none above; a count
    /// that rounds up to a thousand of one unit is one of the next.
    public static func tokens(_ n: UInt64, locale: Locale = .current) -> String {
        let formatter = NumberFormatter()
        formatter.locale = locale
        formatter.numberStyle = .decimal
        formatter.minimumFractionDigits = 0
        guard n >= 1000 else {
            formatter.maximumFractionDigits = 0
            return formatter.string(from: NSNumber(value: n)) ?? "\(n)"
        }
        let units: [(Double, String)] = [(1e3, "K"), (1e6, "M"), (1e9, "B")]
        var unit = units.lastIndex { Double(n) >= $0.0 } ?? 0
        while true {
            let (scale, suffix) = units[unit]
            let value = Double(n) / scale
            let digits = value < 9.95 ? 1 : 0
            let power = digits == 1 ? 10.0 : 1.0
            let rounded = (value * power).rounded() / power
            if rounded >= 1000, unit + 1 < units.count {
                unit += 1
                continue
            }
            formatter.maximumFractionDigits = digits
            return (formatter.string(from: NSNumber(value: rounded)) ?? "\(rounded)") + suffix
        }
    }

    /// Millionths of a dollar in `locale`'s currency style: "$1,234.56";
    /// above zero and below a cent, "Under $0.01".
    public static func dollars(_ micros: Int64, locale: Locale = .current) -> String {
        let formatter = NumberFormatter()
        formatter.locale = locale
        formatter.numberStyle = .currency
        formatter.currencyCode = "USD"
        formatter.minimumFractionDigits = 2
        formatter.maximumFractionDigits = 2
        if micros > 0 && micros < 10_000 {
            return "Under " + (formatter.string(from: 0.01) ?? "$0.01")
        }
        let cents = (Double(micros) / 10_000).rounded()
        return formatter.string(from: NSNumber(value: cents / 100)) ?? "$\(cents / 100)"
    }

    /// A total of agent time: "under a minute", "6 min", "3 h 10 min", and
    /// whole hours from ten on.
    public static func duration(ms: Int64) -> String {
        let minutes = ms / 60_000
        switch minutes {
        case ..<1: return "under a minute"
        case ..<60: return "\(minutes) min"
        case ..<600:
            let rest = minutes % 60
            return rest == 0 ? "\(minutes / 60) h" : "\(minutes / 60) h \(rest) min"
        default: return "\(minutes / 60) h"
        }
    }

    /// "1.2M tokens", or "Not reported" when no turn stated any.
    public static func tokensLine(_ s: TaskSpend, locale: Locale = .current) -> String {
        s.totalTokens == 0 ? notReported : "\(tokens(s.totalTokens, locale: locale)) tokens"
    }

    /// "12K input · 3.4K output · 1.1M cache", or nil with no tokens.
    public static func tokenDetail(_ s: TaskSpend, locale: Locale = .current) -> String? {
        guard s.totalTokens > 0 else { return nil }
        return [
            "\(tokens(s.inputTokens, locale: locale)) input",
            "\(tokens(s.outputTokens, locale: locale)) output",
            "\(tokens(s.cacheReadTokens + s.cacheWriteTokens, locale: locale)) cache",
        ].joined(separator: " · ")
    }

    /// "$3.20 · API-equivalent", with "estimated", "partly estimated" and
    /// "partly not reported" as they apply; "Not reported" with no known
    /// cost.
    public static func cost(_ s: TaskSpend, locale: Locale = .current) -> String {
        guard s.pricedMicros > 0 else { return notReported }
        var line = "\(dollars(s.pricedMicros, locale: locale)) · API-equivalent"
        if s.costReportedMicros == 0 {
            line += ", estimated"
        } else if s.costEstimatedMicros > 0 {
            line += ", partly estimated"
        }
        if s.partlyUnknown { line += ", partly not reported" }
        return line
    }

    /// "Agent time 3 h 10 min · 12 turns", either half alone, or nil.
    public static func time(_ s: TaskSpend) -> String? {
        var parts: [String] = []
        if s.activeMs > 0 { parts.append("Agent time \(duration(ms: s.activeMs))") }
        switch s.turns {
        case 0: break
        case 1: parts.append("1 turn")
        default: parts.append("\(s.turns) turns")
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    /// "claude · claude-opus-5", or the harness alone for an unnamed model.
    public static func title(_ row: TaskSpendRow) -> String {
        row.model.isEmpty ? row.harness : "\(row.harness) · \(row.model)"
    }

    /// "1.2M tokens · $3.20", with "estimated", "partly estimated" and
    /// "partly not reported" as they apply to the row's own part; "Cost not
    /// reported"; or "Not reported" when the row stated nothing.
    public static func detail(_ row: TaskSpendRow, locale: Locale = .current) -> String {
        let s = row.totals
        guard s.totalTokens > 0 || s.pricedMicros > 0 else { return notReported }
        let cost: String
        if s.pricedMicros <= 0 {
            // A model the price table has no rate for says so by name.
            cost = !row.model.isEmpty && s.unpricedTokens > 0 ? "No price listed for \(row.model)" : "Cost not reported"
        } else {
            var words: [String] = []
            if s.costReportedMicros == 0 {
                words.append("estimated")
            } else if s.costEstimatedMicros > 0 {
                words.append("partly estimated")
            }
            if s.partlyUnknown { words.append("partly not reported") }
            let amount = dollars(s.pricedMicros, locale: locale)
            cost = words.isEmpty ? amount : "\(amount) \(words.joined(separator: ", "))"
        }
        return "\(tokens(s.totalTokens, locale: locale)) tokens · \(cost)"
    }
}

/// What a Usage section shows.
public enum TaskUsageState: Equatable, Sendable {
    case loading
    /// The runner is older than spend: `TaskUsageFormat.needsUpdate`.
    case needsUpdate
    /// The read didn't come back: `couldntRead`, with Try Again.
    case failed
    case loaded(TaskUsage)

    /// The state a read lands in: the usage, or why there's none. A runner
    /// that says it lacks `agent_usage` needs an update; one whose build
    /// isn't known yet is asked, and a refusal then reads as a failure.
    public static func after(read usage: TaskUsage?, runnerCan: Bool?) -> TaskUsageState {
        if runnerCan == false { return .needsUpdate }
        return usage.map(TaskUsageState.loaded) ?? .failed
    }
}
