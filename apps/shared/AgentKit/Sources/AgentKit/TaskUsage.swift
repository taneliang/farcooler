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
        inputTokens = try n(.inputTokens)
        outputTokens = try n(.outputTokens)
        cacheReadTokens = try n(.cacheReadTokens)
        cacheWriteTokens = try n(.cacheWriteTokens)
        costReportedMicros = try i(.costReportedMicros)
        costEstimatedMicros = try i(.costEstimatedMicros)
        unpricedTokens = try n(.unpricedTokens)
    }

    private enum CodingKeys: String, CodingKey {
        case turns, turnsPartial, turnsNotReported, subagentRuns, inputTokens, outputTokens
        case cacheReadTokens, cacheWriteTokens, costReportedMicros, costEstimatedMicros, unpricedTokens
    }

    public var totalTokens: UInt64 { inputTokens + outputTokens + cacheReadTokens + cacheWriteTokens }
    /// Nothing recorded: no turn, and no subagent run.
    public var isEmpty: Bool { turns == 0 && subagentRuns == 0 }
    var pricedMicros: Int64 { costReportedMicros + costEstimatedMicros }
    var partlyUnknown: Bool { unpricedTokens > 0 || turnsPartial > 0 || turnsNotReported > 0 }
}

/// A task's spend, as the Usage section draws it: the lines the client core
/// says (`farcooler_client::usage_json`, ov-1048), the same the CLI prints.
public struct TaskUsage: Decodable, Equatable, Sendable {
    public var said: WireUsageSaid

    public static func decode(_ data: Data) throws -> TaskUsage {
        try JSONDecoder().decode(TaskUsage.self, from: data)
    }

    /// What the client core answers `usage.task` with for `spend`, the runner's
    /// numbers: for a harness that fakes the route. Nil without the bridge.
    public static func shown(spend: Data) -> Data? {
        let answer = PlanViewPresenter.ask(#"{"schema":1,"usage":"# + String(decoding: spend, as: UTF8.self) + "}")
        let object = answer.flatMap { try? JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any] }
        guard let usage = object?["usage"] as? [String: Any] else { return nil }
        return try? JSONSerialization.data(withJSONObject: usage)
    }
}

extension WireUsageRowSaid: Identifiable {}

/// The words a Usage section says.
public enum TaskUsageFormat {
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
