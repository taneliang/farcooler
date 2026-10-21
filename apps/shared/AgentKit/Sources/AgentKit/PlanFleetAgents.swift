import Foundation

// Every agent that still exists for a board, as the plan carries them (ov-503,
// design 6; ov-518): derived on the runner from the turns it read, never
// typed, so the orchestrator and a subagent nobody recorded are in it. An
// agent that finished or exited is not: the runner leaves it out, and its
// spend stays in the day's total. Behind
// `board_agents`. The type is `PlanFleetAgent` because `PlanAgent` is a lane's
// typed agent record, which this is not.

/// What an agent does.
public enum PlanFleetRole: String, Decodable, Sendable, CaseIterable {
    case orchestrator, build, review, fix, integrator, subagent, agent

    public init(from decoder: Decoder) throws {
        self = PlanFleetRole(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .agent
    }

    public var word: String {
        switch self {
        case .orchestrator: "Orchestrator"
        case .build: "Builder"
        case .review: "Reviewer"
        case .fix: "Fixer"
        case .integrator: "Integrator"
        case .subagent: "Subagent"
        case .agent: "Agent"
        }
    }
}

public struct PlanFleetAgent: Decodable, Equatable, Identifiable, Sendable {
    public struct Lane: Decodable, Equatable, Sendable {
        public var id: String
        public var name: String
    }
    public struct Train: Decodable, Equatable, Sendable {
        public var id: String
        public var title: String
    }

    /// `agent:<agentId>`, `pane:<terminal id>` or `turns:<harness>`.
    public var id: String
    public var role: PlanFleetRole
    public var harness: String
    /// The model that spent the most; empty when none was named.
    public var model: String
    /// What the orchestrator called a subagent; empty when nothing kept it.
    public var label: String
    public var lane: Lane?
    public var train: Train?
    public var card: PlanCardRef?
    /// The pane, to open it.
    public var terminal: String?
    public var working: Bool
    public var startedAt: Int64
    public var lastAt: Int64
    public var spend: PlanSpend

    /// "Builder · Sonnet": its role and its model, else its harness.
    public var who: String {
        let model = PlanWords.model(model)
        return "\(role.word) · \(model.isEmpty ? harness : model)"
    }

    /// What it works for: the plan, a lane, a train or a card, else "Not on a
    /// lane".
    public var worksFor: String {
        if role == .orchestrator { return "Plan" }
        if let lane { return lane.name }
        if let train { return train.title }
        if let card { return card.key }
        return PlanWords.notOnALane
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

extension PlanFleetAgent {
    /// "“Design ov-503” · Not on a lane · Working · 22 min": what it was told, what it
    /// works for, and its state.
    public func second(now: Int64) -> String {
        var parts: [String] = []
        if !label.isEmpty { parts.append("\u{201C}\(label)\u{201D}") }
        parts.append(worksFor)
        parts.append(state(now: now))
        return parts.joined(separator: " \u{00B7} ")
    }
}

extension PlanWords {
    public static let notOnALane = "Not on a lane"
}

/// The agents list as the Plan draws it: the agents that exist, working
/// first, by spend. The header's total is the day's, the agents that finished
/// included, as the runner counts it.
public struct PlanFleet: Equatable, Sendable {
    public var agents: [PlanFleetAgent]
    /// What every agent of the last 24 hours spent, those gone included; nil
    /// from a runner before ov-518, and the listed agents' sum stands in.
    public var spend: PlanSpend?

    public init(_ agents: [PlanFleetAgent], spend: PlanSpend? = nil) {
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
