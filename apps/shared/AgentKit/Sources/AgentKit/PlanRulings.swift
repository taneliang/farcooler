import Foundation

// Decided for you (ov-304): the reversible calls the orchestrator made on the
// owner's behalf, read with the plan (`farcooler plan --json`'s `rulings`, the
// phones' `plan.get`). EXPERIMENTAL with the plan layer, behind the runner's
// `board_rulings` capability.
//
// A ruling is open until the owner acts (ov-333). Keep is the owner's own mark
// and changes nothing in the plan; Reverse asks the orchestrator, which does
// the work and marks the ruling reversed with the commit; Discuss starts a
// message about it. `RulingActions` holds the rules for the last two. The
// store's words are standing and confirmed; the owner reads open and kept.

/// Where a ruling stands: `standing` is open, `confirmed` is kept.
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
    /// The commit that reversed it, when the orchestrator marked it with one.
    public var reversedSha: String?

    public init(
        id: String, number: Int, decision: String, why: String, reversal: String, cards: [PlanCardRef] = [],
        theme: String = "", state: RulingState = .standing, note: String = "", actor: String = "manager",
        createdAt: Int64 = 0, settledAt: Int64? = nil, settledBy: String? = nil, reversedSha: String? = nil
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
        self.settledBy = settledBy ?? (settledAt == nil ? nil : actor)
        self.settledAt = settledAt
        self.reversedSha = reversedSha
    }

    /// What Copy Reference puts on the clipboard, for telling the
    /// orchestrator: "ruling R-12: The inbox is amber."
    public var reference: String { "ruling \(short): \(decision)" }

    /// Open until the owner keeps or reverses it.
    public var isStanding: Bool { state == .standing }

    /// Whether the owner has kept it.
    public var isKept: Bool { state == .confirmed }
}

extension PlanModel {
    /// The open rulings, newest first: all that Decided For You shows.
    public var openRulings: [PlanRuling] { rulings.filter(\.isStanding) }

    /// The kept and reversed ones, most recently settled first: Past
    /// Decisions.
    public var pastRulings: [PlanRuling] {
        rulings.filter { !$0.isStanding }.sorted {
            ($0.settledAt ?? 0, $0.number) > ($1.settledAt ?? 0, $1.number)
        }
    }
}

extension PlanWords {
    /// The section's title, in Apple's title case. Android says "Decided for
    /// you".
    public static let decidedForYou = "Decided For You"
    /// Copy Reference, on a ruling's context menu.
    public static let copyReference = "Copy Reference"
    /// The owner's actions on an open ruling (ov-333).
    public static let keepRuling = "Keep"
    public static let keepAllRulings = "Keep All"
    public static let reverseRuling = "Reverse"
    public static let discussRuling = "Discuss"
    /// The fold holding the kept and reversed rulings.
    public static let pastDecisions = "Past Decisions"
    /// Why Reverse and Discuss are off, and what turns them on.
    public static let rulingNeedsOrchestrator = "Start an orchestrator to reverse or discuss a ruling"
    /// What a ruling row says under its decision, before the reason.
    public static let rulingWhy = "Why"
    /// Before what reversing costs.
    public static let rulingReversal = "Reversing"

    /// A ruling's state in the owner's words: "Open", "Kept", "Reversed".
    public static func rulingState(_ state: RulingState) -> String {
        switch state {
        case .standing: "Open"
        case .confirmed: "Kept"
        case .reversed: "Reversed"
        case .unknown: "Unknown"
        }
    }

    /// "Kept", "Reversed in 6e7e5618": a past decision's state, with the
    /// commit that reversed it when the orchestrator named one.
    public static func rulingSettled(_ ruling: PlanRuling) -> String {
        let word = rulingState(ruling.state)
        if ruling.state == .reversed, let sha = ruling.reversedSha, !sha.isEmpty { return "\(word) in \(sha)" }
        return word
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
        [r.decision, r.why, r.reversal, r.state.rawValue, r.note, r.theme, r.reversedSha ?? "",
            r.cards.map(\.key).joined(separator: ",")]
            .joined(separator: "\u{1}")
    }
}
