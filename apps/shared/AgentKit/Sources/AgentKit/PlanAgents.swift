import Foundation

// Every agent that still exists for a board, as the plan carries them (ov-503,
// design 6; ov-518; ov-872): derived on the runner from the turns it read and
// the agents it recorded, so the orchestrator is in it. An agent that finished
// or exited is not: the runner leaves it out, and its spend stays in the day's
// total. An agent has a role (builder, reviewer, lander, or the
// orchestrator), the tickets it works now, and what it is doing now.

/// What an agent is for.
public enum PlanAgentRole: String, Decodable, Sendable, CaseIterable {
    case orchestrator, builder, reviewer, lander, unknown

    public init(from decoder: Decoder) throws {
        self = PlanAgentRole(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
    }

    public var word: String {
        switch self {
        case .orchestrator: "Orchestrator"
        case .builder: "Builder"
        case .reviewer: "Reviewer"
        case .lander: "Lander"
        case .unknown: "Agent"
        }
    }
}

/// A ticket an agent works now, and as what.
public struct PlanAgentTicket: Decodable, Equatable, Sendable {
    public var task: String
    public var key: String
    public var role: WorkRole
    public var startedAt: Int64
}

public struct PlanAgent: Decodable, Equatable, Identifiable, Sendable {
    /// The agent's id.
    public var id: String
    public var short: String
    public var role: PlanAgentRole
    public var harness: String
    /// The model that spent the most; empty when none was named.
    public var model: String
    /// What the orchestrator called it; empty when it gave no title.
    public var title: String
    /// The pane, to open it.
    public var terminal: String?
    /// A subagent's id inside its parent's pane; empty for a pane of its own.
    public var subagentId: String
    public var parentTerminal: String?
    public var startedAt: Int64
    /// When a person asked it to stop; nil if nobody has.
    public var stopAskedAt: Int64?
    /// The tickets it works now.
    public var tickets: [PlanAgentTicket]
    public var working: Bool
    /// What it is doing now, in its own words; empty when it says nothing.
    public var doingNow: String
    public var lastAt: Int64
    public var spend: PlanSpend

    /// "Builder · Sonnet": its role and its model, else its harness.
    public var who: String {
        let model = PlanWords.model(model)
        return "\(role.word) · \(model.isEmpty ? harness : model)"
    }

    /// What it works for: the plan, its tickets by key, else "Not on a ticket".
    public var worksFor: String {
        if role == .orchestrator { return "Plan" }
        if !tickets.isEmpty { return tickets.map(\.key).joined(separator: ", ") }
        return PlanWords.onNoTicket
    }

    /// "Working · 22 min" while it works; "Idle · active 1 h ago" for a pane
    /// that still runs and is between turns.
    public func state(now: Int64) -> String {
        if working { return "Working · \(PlanWords.span(now - startedAt))" }
        // An orchestrator with no turn in the window has no last turn to name.
        return lastAt > 0 ? "Idle · active \(PlanWords.ago(lastAt, now: now))" : "Idle"
    }

    /// "380K", tokens first; nil when no turn stated any.
    public func tokens(locale: Locale = .current) -> String? {
        spend.totalTokens > 0 ? TaskUsageFormat.tokens(spend.totalTokens, locale: locale) : nil
    }

    /// "≈$1.10", API-equivalent dollars second; nil when no price is known.
    public func cost(locale: Locale = .current) -> String? {
        spend.costMicros.map { "\u{2248}" + TaskUsageFormat.dollars($0, locale: locale) }
    }
}

extension PlanAgent {
    /// "“Design ov-503” · ov-503 · Working · 22 min": what it was told, what it
    /// works for, and its state.
    public func second(now: Int64) -> String {
        var parts: [String] = []
        if !title.isEmpty { parts.append("\u{201C}\(title)\u{201D}") }
        parts.append(worksFor)
        parts.append(state(now: now))
        return parts.joined(separator: " \u{00B7} ")
    }
}

extension PlanWords {
    public static let onNoTicket = "Not on a ticket"
}

/// The agents list as the Plan draws it: the agents that exist, working
/// first, by spend. The header's total is the day's, the agents that finished
/// included, as the runner counts it.
public struct PlanFleet: Equatable, Sendable {
    public var agents: [PlanAgent]
    /// What every agent of the last 24 hours spent, those gone included; nil
    /// from a runner before ov-518, and the listed agents' sum stands in.
    public var spend: PlanSpend?

    public init(_ agents: [PlanAgent], spend: PlanSpend? = nil) {
        self.agents = agents.sorted {
            if $0.working != $1.working { return $0.working }
            return $0.spend.totalTokens > $1.spend.totalTokens
        }
        self.spend = spend
    }

    public var isEmpty: Bool { agents.isEmpty }

    public var workingCount: Int { agents.filter(\.working).count }

    /// Whether the orchestrator is among them. This list is every agent that
    /// still exists, the orchestrator and the idle ones included; the Agents
    /// card counts the agents the orchestrator has running
    /// (`RunningAgents.count`), without it. The two numbers can differ by the
    /// orchestrator and by idle agents, so the header says what it counts
    /// (ov-660).
    public var includesOrchestrator: Bool { agents.contains { $0.role == .orchestrator } }

    /// The day's tokens.
    public var totalTokens: UInt64 { spend?.totalTokens ?? agents.reduce(0) { $0 + $1.spend.totalTokens } }

    /// Dollars, only when every agent is priced: a sum with a hole in it
    /// would read low.
    public var totalCostMicros: Int64? {
        if let spend { return spend.costMicros }
        return agents.reduce(Int64?.some(0)) { sum, agent in
            guard let sum, let micros = agent.spend.costMicros else { return nil }
            return sum + micros
        }
    }

    /// What the count says it counts: "5 agents, including the orchestrator".
    /// The Agents card beside the conversation counts the agents the
    /// orchestrator is running, without it, so a list that includes it says
    /// so rather than showing a different number under the same word (ov-660).
    public var countWords: String {
        let noun = agents.count == 1 ? "1 agent" : "\(agents.count) agents"
        return includesOrchestrator ? "\(noun), including the orchestrator" : noun
    }

    /// "5 agents, including the orchestrator · 4 working · 3.9M tokens in the
    /// last 24 hours · ≈$26"; with none alive, "No agents running · 1.2M
    /// tokens in the last 24 hours · ≈$12" while the day spent something, and
    /// "No agents running." when it spent nothing.
    public func header(locale: Locale = .current) -> String {
        guard !isEmpty || totalTokens > 0 else { return PlanWords.noAgents }
        var parts = isEmpty ? ["No agents running"] : [countWords, "\(workingCount) working"]
        parts.append("\(TaskUsageFormat.tokens(totalTokens, locale: locale)) tokens in the last 24 hours")
        if let micros = totalCostMicros { parts.append("\u{2248}" + TaskUsageFormat.dollars(micros, locale: locale)) }
        return parts.joined(separator: " · ")
    }
}

extension PlanWords {
    public static let noAgents = "No agents running."
}
