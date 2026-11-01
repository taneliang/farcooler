import Foundation

// A landing's phase (ov-503, design 4): the one reading of where a landing
// stands that every surface shares. The runner derives it (`train_phase.rs`)
// and sends it on the landing; phase 0 is sent as null. Both read through
// `PlanLanding.phase(in:)`, which falls back to the stored `state` and to what
// the plan knows of the landing's tickets, so a surface never has to ask which
// runner it's talking to.

/// Where a landing stands, as a person says it.
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

    /// The phase alone, in title case. Every word maps to a state the runner
    /// derives; combining the tickets, the local gates and the full test run
    /// are one step to a person, "Checking Here" (ov-522).
    public var word: String {
        switch self {
        case .waitingForTickets: "Waiting for Tickets"
        case .waitingForPreviousLanding: "Waiting for the Earlier Landing"
        case .waitingForMain: "Waiting for Main's Checks to Pass"
        case .integrating, .gatingLocally: "Checking Here"
        case .waitingForCi: "Checking on CI"
        case .ciFailed: "CI Failed"
        case .readyToLand: "Ready to Land"
        case .landed: "Landed"
        case .stalled: "Stalled"
        case .dropped: "Dropped"
        case .unknown: "Unknown"
        }
    }

    /// The phase inside a sentence: "Landing 86 checking on CI".
    public var sentence: String {
        switch self {
        case .waitingForTickets: "waiting for tickets"
        case .waitingForPreviousLanding: "waiting for the earlier landing"
        case .waitingForMain: "waiting for main's checks to pass"
        case .integrating, .gatingLocally: "checking here"
        case .waitingForCi: "checking on CI"
        case .ciFailed: "CI failed"
        case .readyToLand: "ready to land"
        case .landed: "landed"
        case .stalled: "stalled"
        case .dropped: "dropped"
        case .unknown: "unknown"
        }
    }

    /// One line on what is happening, for a hover or a long press.
    public var explanation: String {
        switch self {
        case .waitingForTickets: "Some of its tickets aren't ready to land yet."
        case .waitingForPreviousLanding: "Ready tickets are collected here while the earlier landing finishes."
        case .waitingForMain: "Ready tickets are collected here until main's checks pass, then they land."
        case .integrating, .gatingLocally: "The full test run, on the runner."
        case .waitingForCi: "Pushed, with CI running its checks."
        case .ciFailed: "A CI check failed, so it needs a fix."
        case .readyToLand: "Every check passed, so it's ready to land."
        case .landed: "Everything passed and it has landed on main."
        case .stalled: "Nothing here has moved for a while."
        case .dropped: "It was dropped without landing."
        case .unknown: "This runner didn't say where it stands."
        }
    }

    /// How far along it is, for ordering landings furthest first. A stalled
    /// landing has lost its place, and an unknown one never leads.
    public var progress: Int {
        switch self {
        case .landed, .dropped: 8
        case .readyToLand: 7
        case .waitingForCi: 6
        case .ciFailed: 5
        case .gatingLocally: 4
        case .integrating: 3
        case .waitingForTickets, .waitingForPreviousLanding, .waitingForMain: 2
        case .stalled: 1
        case .unknown: 0
        }
    }

    /// The ones drawn in amber: the runner's CI failed, or nothing has moved.
    public var needsAttention: Bool { self == .ciFailed || self == .stalled }
}

extension PlanLanding {
    /// The phase to show: the runner's, else what `state` and the tickets say.
    ///
    /// A runner that sent phase 0, and one with a phase this build has no word
    /// for, read the same way. Pushed, red, green, landed and dropped are
    /// one-to-one with a phase. Gating is gating locally. Open is waiting for
    /// tickets while any live ticket on the landing isn't ready (`landing`),
    /// else integrating. The fallback never says stalled: that needs the
    /// runner's clock.
    public func phase(in plan: PlanModel) -> LandingPhase {
        if let phase, phase != .unknown { return phase }
        switch state {
        case .pushed: return .waitingForCi
        case .red: return .ciFailed
        case .green: return .readyToLand
        case .landed: return .landed
        case .dropped: return .dropped
        case .gating: return .gatingLocally
        case .open, .unknown:
            let blocked = plan.lines(on: self).contains { $0.stage.isLive && $0.stage != .landing }
            return blocked ? .waitingForTickets : .integrating
        }
    }

    /// When the landing entered the phase to show: the runner's clock, else
    /// the state's.
    public func phaseSince(in plan: PlanModel) -> Int64 {
        if let phase, phase != .unknown, let phaseSince { return phaseSince }
        return stateSince
    }
}

extension PlanWords {
    /// What a landing is, for a hover or a long press where one first appears
    /// (ov-522): the Landing header and the landing card.
    public static let landingExplanation = "Reviewed tickets land on main together, after one round of checks."

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

/// The ladder a landing climbs: three steps, and which one a phase is on. The
/// Mac draws it as three segments; the phones say it as "Checking on CI ·
/// step 2 of 3" (ov-503, design 7; ov-522). Waiting for the Earlier Landing
/// and Waiting for Main's Checks to Pass (the collecting landing), Waiting for
/// Tickets, CI Failed and Stalled are said in place of the step they stand on.
public enum PlanLadder {
    public static let steps = ["Checking Here", "Checking on CI", "Landed"]

    /// What each step is, one line, for a hover or a long press.
    public static let explanations = [
        "The full test run, on the runner.",
        "Pushed, with CI running its checks.",
        "Everything passed and it has landed on main.",
    ]

    /// The step a phase is standing on, 0 to 2; 3 once everything is done.
    public static func step(_ phase: LandingPhase) -> Int {
        switch phase {
        case .waitingForPreviousLanding, .waitingForMain, .waitingForTickets, .unknown, .integrating, .gatingLocally, .stalled: 0
        case .waitingForCi, .ciFailed: 1
        case .readyToLand: 2
        case .landed, .dropped: 3
        }
    }

    /// What step `index` says while a landing stands on `current` in `phase`:
    /// the phase's own word on the step it's on.
    public static func label(_ index: Int, current: Int, phase: LandingPhase) -> String {
        index == current ? phase.word : steps[index]
    }

    /// What step `index` explains: the phase's own line on the step it's on.
    public static func explanation(_ index: Int, current: Int, phase: LandingPhase) -> String {
        index == current ? phase.explanation : explanations[index]
    }

    /// "Checking on CI · step 2 of 3": the phase and where it is on the
    /// ladder. A landing past the ladder (landed, dropped) says the phase alone.
    public static func compact(_ phase: LandingPhase) -> String {
        let at = step(phase)
        return at >= steps.count ? phase.word : "\(phase.word) \u{00B7} step \(at + 1) of \(steps.count)"
    }
}

extension PlanSections.LandingCard {
    /// "3f78754f · 2 of 5 jobs done · 12 min · Lander · Opus · 1.4M": the
    /// commit, how CI stands, how long the landing has been in this phase and
    /// who lands it. Empty when none of it is known.
    public func facts(ci: PlanCIRead?, now: Int64) -> String {
        var parts: [String] = []
        if let sha = landing.pushedSha, !sha.isEmpty { parts.append(String(sha.prefix(8))) }
        if let ci {
            parts.append(PlanWords.ciSummary(ci))
            if let stale = PlanWords.ciStale(ci, now: now) { parts.append(stale) }
        } else if landing.pushedSha != nil {
            parts.append("CI not read yet")
        }
        if since > 0 { parts.append(PlanWords.span(now - since)) }
        if let lander { parts.append(Self.landerWords(lander)) }
        return parts.joined(separator: " \u{00B7} ")
    }

    /// `facts` with each "·"-separated span held together, so a line breaks
    /// between spans and never inside one ("12 min" is not cut after "12").
    /// For a view that wraps; the words are `facts`'s (ov-515).
    public func factsUnbroken(ci: PlanCIRead?, now: Int64) -> String {
        Self.unbroken(facts(ci: ci, now: now))
    }

    /// `line` with the spaces inside each " · " span made non-breaking.
    public static func unbroken(_ line: String) -> String {
        line.components(separatedBy: " \u{00B7} ")
            .map { $0.replacingOccurrences(of: " ", with: "\u{00A0}") }
            .joined(separator: " \u{00B7} ")
    }

    /// "Lander · Opus · 1.4M", or "Lander · Opus · done".
    public static func landerWords(_ agent: PlanAgent) -> String {
        var parts = [PlanAgentRole.lander.word]
        let model = PlanWords.model(agent.model)
        if !model.isEmpty { parts.append(model) }
        if agent.spend.totalTokens > 0 {
            parts.append(TaskUsageFormat.tokens(agent.spend.totalTokens))
        } else if !agent.working {
            parts.append("done")
        }
        return parts.joined(separator: " \u{00B7} ")
    }
}
