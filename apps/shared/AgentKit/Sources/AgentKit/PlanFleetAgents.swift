import Foundation

// Every agent working for a board, as the plan carries them (ov-503, design
// 6): derived on the runner from the turns it read, never typed, so the
// orchestrator and a subagent nobody recorded are in it. Behind
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

    /// "Working · 22 min" while it works, "Done 1 h ago" after.
    public func state(now: Int64) -> String {
        working ? "Working · \(PlanWords.span(now - startedAt))" : "Done \(PlanWords.ago(lastAt, now: now))"
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

extension PlanWords {
    public static let notOnALane = "Not on a lane"
}

/// The agents list as the Plan draws it: working first, by spend, then the
/// ones that finished.
public struct PlanFleet: Equatable, Sendable {
    public var working: [PlanFleetAgent]
    public var finished: [PlanFleetAgent]

    public init(_ agents: [PlanFleetAgent]) {
        working = agents.filter(\.working).sorted { $0.spend.totalTokens > $1.spend.totalTokens }
        finished = agents.filter { !$0.working }.sorted { $0.lastAt > $1.lastAt }
    }

    public var isEmpty: Bool { working.isEmpty && finished.isEmpty }

    /// Every agent's tokens in the window.
    public var totalTokens: UInt64 { (working + finished).reduce(0) { $0 + $1.spend.totalTokens } }

    /// Dollars, only when every agent is priced: a sum with a hole in it
    /// would read low.
    public var totalCostMicros: Int64? {
        (working + finished).reduce(Int64?.some(0)) { sum, agent in
            guard let sum, let micros = agent.spend.costMicros else { return nil }
            return sum + micros
        }
    }

    /// "4 working · 3.9M tokens in the last 24 hours · ≈$26", or "No agents
    /// running." with none at all.
    public func header(locale: Locale = .current) -> String {
        guard !isEmpty else { return PlanWords.noAgents }
        var parts = ["\(working.count) working", "\(TaskUsageFormat.tokens(totalTokens, locale: locale)) tokens in the last 24 hours"]
        if let micros = totalCostMicros { parts.append("\u{2248}" + TaskUsageFormat.dollars(micros, locale: locale)) }
        return parts.joined(separator: " · ")
    }
}

extension PlanWords {
    public static let noAgents = "No agents running."
}
