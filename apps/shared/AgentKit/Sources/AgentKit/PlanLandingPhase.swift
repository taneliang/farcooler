import Foundation

// A landing's phase (ov-503, design 4): the one reading of where a landing
// stands that every surface shares. The runner derives it (`train_phase.rs`)
// and sends it on the landing; phase 0 is sent as null. The Rust core's plan
// view reads it, falling back to the stored `state` and to what the plan knows
// of the landing's tickets, and says it in words (ov-1016); this decodes it,
// and lays out the facts the view leaves to the app.

/// Where a landing stands, as the runner sends it.
public enum LandingPhase: String, Decodable, Sendable, CaseIterable {
    case waitingForTickets = "waiting_for_tickets"
    /// The collecting landing, gathering ready tickets while the landing ahead
    /// lands (ov-520, ov-522). The runner sends it; no `state` reads as it.
    case waitingForPreviousLanding = "waiting_for_previous_landing"
    /// The collecting landing with nothing ahead, held off until main's own
    /// checks pass (ov-520, proto 11). The runner sends it; no `state` reads
    /// as it.
    case waitingForMain = "waiting_for_main"
    case integrating
    case gatingLocally = "gating_locally"
    case waitingForCi = "waiting_for_ci"
    case ciFailed = "ci_failed"
    case readyToLand = "ready_to_land"
    case landed
    case stalled
    case dropped
    case unknown

    public init(from decoder: Decoder) throws {
        self = LandingPhase(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
    }
}

extension PlanWords {
    /// How long, coarsely: "45 min", "2 h", "3 d".
    public static func span(_ ms: Int64) -> String {
        let minutes = max(0, ms) / 60_000
        switch minutes {
        case 0..<60: return "\(minutes) min"
        case 60..<1440: return "\(minutes / 60) h"
        default: return "\(minutes / 1440) d"
        }
    }
}

extension PlanView.LandingCard {
    /// "3f78754f · 2 of 5 jobs done · 12 min · Lander · Opus · 1.4M": the
    /// commit, how CI stands, how long the landing has been in this phase and
    /// who lands it. Empty when none of it is known. The CI read is the app's
    /// (`PlanModel.ci`), which the view doesn't carry yet (ov-1015 phase 2).
    public func facts(ci: PlanCIRead?, now: Int64) -> String {
        var parts: [String] = []
        if let sha = pushedSha, !sha.isEmpty { parts.append(String(sha.prefix(8))) }
        if let ci {
            parts.append(PlanWords.ciSummary(ci))
            if let stale = PlanWords.ciStale(ci, now: now) { parts.append(stale) }
        } else if let ciUnread {
            parts.append(ciUnread)
        }
        if since > 0 { parts.append(PlanWords.span(now - since)) }
        if let landerWords { parts.append(landerWords) }
        return parts.joined(separator: " \u{00B7} ")
    }

    /// Facts for assistive technology, retaining the exact recorded model ID.
    public func accessibleFacts(ci: PlanCIRead?, now: Int64) -> String {
        PlanWords.accessibleModel(facts(ci: ci, now: now), raw: lander?.model ?? "")
    }

    /// `facts` with each "·"-separated span held together, so a line breaks
    /// between spans and never inside one ("12 min" is not cut after "12").
    public func factsUnbroken(ci: PlanCIRead?, now: Int64) -> String {
        PlanWords.unbroken(facts(ci: ci, now: now))
    }

    /// Drawn in amber: its phase says so (CI failed, stalled), or the CI read needs a look.
    public func needsAttention(ci: PlanCIRead?) -> Bool { attention || (ci?.needsAttention ?? false) }
}

extension PlanWords {
    /// `line` with the spaces inside each " · " span made non-breaking.
    public static func unbroken(_ line: String) -> String {
        line.components(separatedBy: " \u{00B7} ")
            .map { $0.replacingOccurrences(of: " ", with: "\u{00A0}") }
            .joined(separator: " \u{00B7} ")
    }
}
