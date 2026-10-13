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

    /// The phase alone, in title case.
    public var word: String {
        switch self {
        case .waitingForLanes: "Waiting for Lanes"
        case .integrating: "Integrating"
        case .gatingLocally: "Gating Locally"
        case .waitingForCi: "Waiting for CI"
        case .ciFailed: "CI Failed"
        case .readyToLand: "Ready to Land"
        case .landed: "Landed"
        case .stalled: "Stalled"
        case .dropped: "Dropped"
        case .unknown: "Unknown"
        }
    }

    /// The phase inside a sentence: "Train 86 waiting for CI".
    public var sentence: String {
        switch self {
        case .waitingForLanes: "waiting for lanes"
        case .integrating: "integrating"
        case .gatingLocally: "gating locally"
        case .waitingForCi: "waiting for CI"
        case .ciFailed: "CI failed"
        case .readyToLand: "ready to land"
        case .landed: "landed"
        case .stalled: "stalled"
        case .dropped: "dropped"
        case .unknown: "unknown"
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
        case .waitingForLanes: 2
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
    /// How long, coarsely: "45 min", "2 h", "3 d".
    public static func span(_ ms: Int64) -> String {
        let minutes = max(0, ms) / 60_000
        switch minutes {
        case 0..<60: return "\(minutes) min"
        case 60..<1440: return "\(minutes / 60) h"
        default: return "\(minutes / 1440) d"
        }
    }

    /// A held train's line: "Holding for ios-overwrite · 2 h", or nil when
    /// nothing holds it.
    public static func held(_ train: PlanTrain, since: Int64, now: Int64) -> String? {
        let names = train.holders.map(\.name)
        guard !names.isEmpty else { return nil }
        return "Holding for \(names.joined(separator: ", ")) · \(span(now - since))"
    }
}

/// The ladder a train climbs: five steps, and which one a phase is on. The
/// Mac draws it as five segments; the phones say it as "Waiting for CI · step
/// 4 of 5" (ov-503, design 7).
public enum PlanLadder {
    public static let steps = ["Picked", "Integrated", "Gated", "Waiting for CI", "Landed"]

    /// The step a phase is standing on, 0 to 4; 5 once everything is done.
    public static func step(_ phase: TrainPhase) -> Int {
        switch phase {
        case .waitingForLanes, .unknown: 0
        case .integrating, .stalled: 1
        case .gatingLocally: 2
        case .waitingForCi, .ciFailed: 3
        case .readyToLand: 4
        case .landed, .dropped: 5
        }
    }

    /// What step `index` says while a train stands on `current` in `phase`:
    /// the phase's own word on the step it's on.
    public static func label(_ index: Int, current: Int, phase: TrainPhase) -> String {
        index == current ? phase.word : steps[index]
    }

    /// "Waiting for CI · step 4 of 5": the phase and where it is on the
    /// ladder. A train past the ladder (landed, dropped) says the phase alone.
    public static func compact(_ phase: TrainPhase) -> String {
        let at = step(phase)
        return at >= steps.count ? phase.word : "\(phase.word) \u{00B7} step \(at + 1) of \(steps.count)"
    }
}
