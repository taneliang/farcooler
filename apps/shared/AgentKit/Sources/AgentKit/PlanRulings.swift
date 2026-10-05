import Foundation

// Decided for you (ov-304): the reversible calls the orchestrator made on the
// owner's behalf, read with the plan (`farcooler plan --json`'s `rulings`, the
// phones' `plan.get`). EXPERIMENTAL with the plan layer, behind the runner's
// `board_rulings` capability.
//
// Nobody edits a ruling here. The owner confirms or reverses one by telling
// the orchestrator, which is its only writer, so the one action a client
// offers is Copy Reference: "ruling R-12: The inbox is amber."

/// Where a ruling stands.
public enum RulingState: String, Decodable, Sendable, CaseIterable {
    case standing, confirmed, reversed, unknown

    public init(from decoder: Decoder) throws {
        self = RulingState(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
    }
}

/// One ruling, as the plan read it.
public struct PlanRuling: Decodable, Equatable, Identifiable, Sendable {
    public var id: String
    /// "R-12": how the owner names it to the orchestrator.
    public var short: String
    public var number: Int
    /// What was decided, in one line.
    public var decision: String
    /// Why, in one or two lines.
    public var why: String
    /// What reversing it costs.
    public var reversal: String
    public var cards: [PlanCardRef]
    public var themeId: String?
    /// The theme's name; empty when it names none.
    public var theme: String
    public var state: RulingState
    /// The words given when it was confirmed or reversed.
    public var note: String
    public var actor: String
    public var createdAt: Int64
    public var settledBy: String?
    public var settledAt: Int64?

    public init(
        id: String, number: Int, decision: String, why: String, reversal: String, cards: [PlanCardRef] = [],
        theme: String = "", state: RulingState = .standing, note: String = "", actor: String = "manager",
        createdAt: Int64 = 0, settledAt: Int64? = nil
    ) {
        self.id = id
        self.short = "R-\(number)"
        self.number = number
        self.decision = decision
        self.why = why
        self.reversal = reversal
        self.cards = cards
        self.themeId = nil
        self.theme = theme
        self.state = state
        self.note = note
        self.actor = actor
        self.createdAt = createdAt
        self.settledBy = settledAt == nil ? nil : actor
        self.settledAt = settledAt
    }

    /// What Copy Reference puts on the clipboard, for telling the
    /// orchestrator: "ruling R-12: The inbox is amber."
    public var reference: String { "ruling \(short): \(decision)" }

    /// Standing until the owner says otherwise.
    public var isStanding: Bool { state == .standing }
}

extension PlanModel {
    /// The rulings that stand, newest first: what Decided For You leads with.
    public var standingRulings: [PlanRuling] { rulings.filter(\.isStanding) }

    /// The confirmed and reversed ones, most recently settled first.
    public var settledRulings: [PlanRuling] { rulings.filter { !$0.isStanding } }
}

extension PlanWords {
    /// The section's title, in Apple's title case. Android says "Decided for
    /// you".
    public static let decidedForYou = "Decided For You"
    /// The one action on a ruling.
    public static let copyReference = "Copy Reference"
    /// What a ruling row says under its decision, before the reason.
    public static let rulingWhy = "Why"
    /// Before what reversing costs.
    public static let rulingReversal = "Reversing"

    /// A settled ruling's state: "Confirmed", "Reversed".
    public static func rulingState(_ state: RulingState) -> String {
        switch state {
        case .standing: "Standing"
        case .confirmed: "Confirmed"
        case .reversed: "Reversed"
        case .unknown: "Unknown"
        }
    }

    /// "ov-1, ov-2 · Visual language": what a ruling touches; nil when
    /// nothing.
    public static func rulingTouches(_ ruling: PlanRuling) -> String? {
        var parts: [String] = []
        if !ruling.cards.isEmpty { parts.append(ruling.cards.map(\.key).joined(separator: ", ")) }
        if !ruling.theme.isEmpty { parts.append(ruling.theme) }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    /// What VoiceOver reads for a ruling: its id, decision, why, what
    /// reversing costs and, once settled, its state.
    public static func rulingAccessibility(_ ruling: PlanRuling) -> String {
        var parts = ["Ruling \(ruling.short)", ruling.decision, "\(rulingWhy): \(ruling.why)",
            "\(rulingReversal): \(ruling.reversal)"]
        if !ruling.isStanding { parts.append(rulingState(ruling.state)) }
        return parts.joined(separator: ". ")
    }

    /// A ruling's signature for the shared change animations: a row washes
    /// when any of what it shows moves.
    public static func rulingSignature(_ r: PlanRuling) -> String {
        [r.decision, r.why, r.reversal, r.state.rawValue, r.note, r.theme, r.cards.map(\.key).joined(separator: ",")]
            .joined(separator: "\u{1}")
    }
}
