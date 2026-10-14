import Foundation

// A train's phase (ov-503, design 4): the one reading of where a train stands
// that every surface shares. The runner derives it (`train_phase.rs`) and
// sends it on `BoardTrain` behind `board_train_phase`; a runner without it
// sends nothing, and phase 0 is sent as null. Both read through
// `PlanTrain.phase(in:)`, which falls back to the stored `state` and to what
// the plan knows of the train's lanes, so a surface never has to ask which
// runner it's talking to.

/// Where a train stands, as a person says it.
public enum TrainPhase: String, Decodable, Sendable, CaseIterable {
    case waitingForLanes = "waiting_for_lanes"
    /// The open train, collecting ready lanes while the train ahead lands
    /// (ov-522). The runner doesn't send it yet; a build that meets it says
    /// so in place of the first step, and no state of today's reads as it.
    case waitingForPreviousTrain = "waiting_for_previous_train"
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
        self = TrainPhase(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
    }

    /// The phase alone, in title case. Every word maps to a state the runner
    /// derives; combining the lanes, the local gates and the full test run
    /// are one step to a person, "Checking Here" (ov-522).
    public var word: String {
        switch self {
        case .waitingForLanes: "Waiting for Lanes"
        case .waitingForPreviousTrain: "Waiting for the Previous Train"
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

    /// The phase inside a sentence: "Train 86 checking on CI".
    public var sentence: String {
        switch self {
        case .waitingForLanes: "waiting for lanes"
        case .waitingForPreviousTrain: "waiting for the previous train"
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
        case .waitingForLanes: "Some of its lanes aren't ready to land yet."
        case .waitingForPreviousTrain: "Ready lanes are collected here while the train ahead lands."
        case .integrating, .gatingLocally: "The train's full test run, on the runner."
        case .waitingForCi: "Pushed, with CI running its checks."
        case .ciFailed: "A CI check failed, so the train needs a fix."
        case .readyToLand: "Every check passed, so it's ready to land."
        case .landed: "Everything passed and the train has landed on main."
        case .stalled: "Nothing on this train has moved for a while."
        case .dropped: "The train was dropped without landing."
        case .unknown: "This runner didn't say where the train stands."
        }
    }

    /// How far along it is, for ordering trains furthest first. A stalled
    /// train has lost its place, and an unknown one never leads.
    public var progress: Int {
        switch self {
        case .landed, .dropped: 8
        case .readyToLand: 7
        case .waitingForCi: 6
        case .ciFailed: 5
        case .gatingLocally: 4
        case .integrating: 3
        case .waitingForLanes, .waitingForPreviousTrain: 2
        case .stalled: 1
        case .unknown: 0
        }
    }

    /// The ones drawn in amber: the runner's CI failed, or nothing has moved.
    public var needsAttention: Bool { self == .ciFailed || self == .stalled }
}

extension PlanTrain {
    /// The phase to show: the runner's, else what `state` and the lanes say.
    ///
    /// A runner without `board_train_phase`, one that sent phase 0, and one
    /// with a phase this build has no word for all read the same way. Pushed,
    /// red, green, landed and dropped are one-to-one with a phase. Gating is
    /// gating locally. Integrating is waiting for lanes while any live lane on
    /// the train isn't ready (`landing`), else integrating. The fallback
    /// never says stalled or held: those need the runner's clock.
    public func phase(in plan: PlanModel) -> TrainPhase {
        if let phase, phase != .unknown { return phase }
        switch state {
        case .pushed: return .waitingForCi
        case .red: return .ciFailed
        case .green: return .readyToLand
        case .landed: return .landed
        case .dropped: return .dropped
        case .gating: return .gatingLocally
        case .integrating, .unknown:
            let blocked = plan.lanes(on: self).contains { $0.state.isLive && $0.state != .landing }
            return blocked ? .waitingForLanes : .integrating
        }
    }

    /// When the train entered the phase to show: the runner's clock, else the
    /// state's.
    public func phaseSince(in plan: PlanModel) -> Int64 {
        if let phase, phase != .unknown, let phaseSince { return phaseSince }
        return stateSince
    }

    /// The lanes holding it, or none.
    public var holders: [PlanTrainLane] { heldBy ?? [] }
}

extension PlanModel {
    /// The plan's lanes on `train`: those it lists, plus any lane that names
    /// it, in the plan's order.
    public func lanes(on train: PlanTrain) -> [PlanLane] {
        let ids = Set(train.lanes.map(\.lane))
        let name = train.name.lowercased()
        return lanes.filter { ids.contains($0.id) || $0.train?.lowercased() == name }
    }
}

extension PlanWords {
    /// What a train is, for a hover or a long press where one first appears
    /// (ov-522): the Landing header and the train card.
    public static let trainExplanation = "A train is a batch of reviewed lanes that land on main together, after one round of checks."

    /// How long, coarsely: "45 min", "2 h", "3 d".
    public static func span(_ ms: Int64) -> String {
        let minutes = max(0, ms) / 60_000
        switch minutes {
        case 0..<60: return "\(minutes) min"
        case 60..<1440: return "\(minutes / 60) h"
        default: return "\(minutes / 1440) d"
        }
    }

    /// A held train's line: "Waiting for ios-overwrite · 2 h", or nil when
    /// nothing holds it.
    public static func held(_ train: PlanTrain, since: Int64, now: Int64) -> String? {
        let names = train.holders.map(\.name)
        guard !names.isEmpty else { return nil }
        return "Waiting for \(names.joined(separator: ", ")) · \(span(now - since))"
    }
}

/// The ladder a train climbs: three steps, and which one a phase is on. The
/// Mac draws it as three segments; the phones say it as "Checking on CI ·
/// step 2 of 3" (ov-503, design 7; ov-522). Waiting for the Previous Train
/// (an open train, once the runner says so), Waiting for Lanes, CI Failed and
/// Stalled are said in place of the step they stand on.
public enum PlanLadder {
    public static let steps = ["Checking Here", "Checking on CI", "Landed"]

    /// What each step is, one line, for a hover or a long press.
    public static let explanations = [
        "The train's full test run, on the runner.",
        "Pushed, with CI running its checks.",
        "Everything passed and the train has landed on main.",
    ]

    /// The step a phase is standing on, 0 to 2; 3 once everything is done.
    public static func step(_ phase: TrainPhase) -> Int {
        switch phase {
        case .waitingForPreviousTrain, .waitingForLanes, .unknown, .integrating, .gatingLocally, .stalled: 0
        case .waitingForCi, .ciFailed: 1
        case .readyToLand: 2
        case .landed, .dropped: 3
        }
    }

    /// What step `index` says while a train stands on `current` in `phase`:
    /// the phase's own word on the step it's on.
    public static func label(_ index: Int, current: Int, phase: TrainPhase) -> String {
        index == current ? phase.word : steps[index]
    }

    /// What step `index` explains: the phase's own line on the step it's on.
    public static func explanation(_ index: Int, current: Int, phase: TrainPhase) -> String {
        index == current ? phase.explanation : explanations[index]
    }

    /// "Checking on CI · step 2 of 3": the phase and where it is on the
    /// ladder. A train past the ladder (landed, dropped) says the phase alone.
    public static func compact(_ phase: TrainPhase) -> String {
        let at = step(phase)
        return at >= steps.count ? phase.word : "\(phase.word) \u{00B7} step \(at + 1) of \(steps.count)"
    }
}

extension PlanSections.TrainCard {
    /// "3f78754f · 2 of 5 jobs done · 12 min · Integrator · Opus · 1.4M": the
    /// commit, how CI stands, how long the train has been in this phase and
    /// who integrates. Empty when none of it is known.
    public func facts(ci: PlanCIRead?, now: Int64) -> String {
        var parts: [String] = []
        if let sha = train.pushedSha, !sha.isEmpty { parts.append(String(sha.prefix(8))) }
        if let ci {
            parts.append(PlanWords.ciSummary(ci))
            if let stale = PlanWords.ciStale(ci, now: now) { parts.append(stale) }
        } else if train.pushedSha != nil {
            parts.append("CI not read yet")
        }
        if since > 0 { parts.append(PlanWords.span(now - since)) }
        if let agent = train.agent { parts.append(Self.integrator(agent)) }
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

    /// "Integrator · Opus · 1.4M", or "Integrator · Opus · done".
    public static func integrator(_ agent: PlanTrainAgent) -> String {
        var parts = ["Integrator"]
        let model = PlanWords.model(agent.model)
        if !model.isEmpty { parts.append(model) }
        if agent.spend.totalTokens > 0 {
            parts.append(TaskUsageFormat.tokens(agent.spend.totalTokens))
        } else if !agent.isWorking {
            parts.append("done")
        }
        return parts.joined(separator: " \u{00B7} ")
    }
}
