import Foundation

// Every agent that still exists for a board, as the plan carries them (ov-503,
// design 6; ov-518; ov-872): derived on the runner from the turns it read and
// the agents it recorded, so the orchestrator is in it. An agent that finished
// or exited is not: the runner leaves it out, and its spend stays in the day's
// total. An agent has a role (builder, reviewer, lander, or the
// orchestrator), the tickets it works now, and what it is doing now. The
// Plan's Agents rows and their header are the Rust core's (`WireAgents`,
// ov-1016); the sidebar's ticket tree still names each agent from here.

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

    /// "380K", tokens first; nil when no turn stated any.
    public func tokens(locale: Locale = .current) -> String? {
        spend.totalTokens > 0 ? TaskUsageFormat.tokens(spend.totalTokens, locale: locale) : nil
    }
}
