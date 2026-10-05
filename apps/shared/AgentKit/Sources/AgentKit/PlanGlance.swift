import Foundation

/// The plan on the glance (ov-310, ov-268 P8): the one board the watch, the
/// widgets and the Live Activity say something about.
///
/// The watch has no sockets, so it hears this from the relay and nowhere
/// else: `/v1/pulse` (`RunnerPulse.Reading`) and the card's content state
/// (`AgentCardState.plan`). The runner decided it (`plan_glance.rs`) and the
/// relay picked the board (`leadPlan`); a surface only says it.
///
/// Lane names and the board's name, never card text.
public struct PlanGlance: Codable, Sendable, Hashable {
    /// A lane in Now.
    public struct Lane: Codable, Sendable, Hashable {
        public var name: String
        public var state: LaneState

        public init(name: String, state: LaneState) {
            self.name = name
            self.state = state
        }

        private enum CodingKeys: String, CodingKey { case name, state }

        public func encode(to encoder: Encoder) throws {
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encode(name, forKey: .name)
            try container.encode(state.rawValue, forKey: .state)
        }
    }

    /// The board, by its workspace's name.
    public var workspace: String
    /// The board's Needs You count: the number its title bar says on the Mac
    /// (`WorkspaceNeedsYou.count`), its needs-you items and themes asking.
    public var needsYou: Int
    /// Up to two lanes being built, reviewed, fixed or landed.
    public var now: [Lane]
    /// The lane next up, or nil when nothing is queued.
    public var next: String?

    public init(workspace: String, needsYou: Int, now: [Lane], next: String? = nil) {
        self.workspace = workspace
        self.needsYou = needsYou
        self.now = now
        self.next = next
    }

    private enum CodingKeys: String, CodingKey { case workspace, needsYou, now, next }

    /// Lenient, as the card around it is: a lane of the wrong shape costs the
    /// lane, and a count of the wrong shape reads as none.
    public init(from decoder: Decoder) throws {
        struct Loose: Decodable {
            var lane: Lane?
            init(from decoder: Decoder) throws { lane = try? Lane(from: decoder) }
        }
        let container = try decoder.container(keyedBy: CodingKeys.self)
        workspace = try container.decode(String.self, forKey: .workspace)
        needsYou = max(0, ((try? container.decodeIfPresent(Int.self, forKey: .needsYou)) ?? nil) ?? 0)
        now = (((try? container.decodeIfPresent([Loose].self, forKey: .now)) ?? nil) ?? []).compactMap(\.lane)
        let next = (try? container.decodeIfPresent(String.self, forKey: .next)) ?? nil
        self.next = next?.isEmpty == false ? next : nil
    }

    // MARK: - Words

    /// "2 need you", "1 needs you"; nil when nothing does, which says nothing
    /// rather than "0 need you".
    public var needsYouWords: String? {
        needsYou > 0 ? "\(needsYou) \(needsYou == 1 ? "needs" : "need") you" : nil
    }

    /// The board's line: "Main · 2 need you", or "Main".
    public var heading: String {
        [workspace, needsYouWords].compactMap { $0 }.joined(separator: " · ")
    }

    /// One lane as a row says it: "mac-ux · In Review".
    public static func words(_ lane: Lane) -> String {
        "\(lane.name) · \(lane.state.word)"
    }

    /// Now on one line: "Now: mac-ux In Review, ov-310 Building"; nil when
    /// nothing is past queued.
    public var nowLine: String? {
        guard !now.isEmpty else { return nil }
        return "Now: " + now.map { "\($0.name) \($0.state.word)" }.joined(separator: ", ")
    }

    /// "Next: mac-fu3"; nil when nothing is queued.
    public var nextLine: String? { next.map { "Next: \($0)" } }

    /// What VoiceOver reads for the whole glance.
    public var spoken: String {
        var parts = ["\(workspace) board" + (needsYouWords.map { ", \($0)" } ?? "")]
        if !now.isEmpty {
            parts.append("Now: " + now.map { "\($0.name), \($0.state.word.lowercased())" }.joined(separator: "; "))
        }
        if let next { parts.append("Next up: \(next)") }
        return parts.joined(separator: ". ") + "."
    }
}

/// Where a lane is. Moves go forward, with two loops back to fixing. Here
/// rather than in `PlanModel.swift` so the watch and the widgets, which
/// compile this file and not that one, say a state as the Plan view does.
public enum LaneState: String, Decodable, Sendable, CaseIterable {
    case queued, building, review, fixing, landing, landed, dropped, unknown

    public init(from decoder: Decoder) throws {
        self = LaneState(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
    }

    /// Neither landed nor dropped.
    public var isLive: Bool { self != .landed && self != .dropped }

    /// The state alone, in title case as the board's statuses are
    /// (`PlanWords.state`).
    public var word: String {
        switch self {
        case .queued: "Queued"
        case .building: "Building"
        case .review: "In Review"
        case .fixing: "Fixing"
        case .landing: "Landing"
        case .landed: "Landed"
        case .dropped: "Dropped"
        case .unknown: "Unknown"
        }
    }
}

extension RunnerPulse {
    /// The board the relay's pulse answer leads with, or nil when it names
    /// none: an account with no plan, and every relay older than ov-310.
    public static func decodePlan(_ data: Data) -> PlanGlance? {
        struct Answer: Decodable {
            var plan: PlanGlance?
            init(from decoder: Decoder) throws {
                let container = try decoder.container(keyedBy: Key.self)
                plan = try? container.decodeIfPresent(PlanGlance.self, forKey: .plan)
            }
            enum Key: String, CodingKey { case plan }
        }
        return (try? JSONDecoder().decode(Answer.self, from: data))?.plan
    }
}

extension RunnerPulse.Reading {
    /// The board the relay's answer named, if it answered and named one.
    public var glance: PlanGlance? {
        if case let .answered(_, _, plan) = self { return plan }
        return nil
    }

    /// When a plan widget looks again: a look later after any answer or a
    /// failed ask, since a plan moves without telling it; never without a
    /// credential or with a refused one, which a new registration reloads.
    public func nextPlanLook(at now: Date, every: TimeInterval) -> Date? {
        switch self {
        case .answered, .failed: now.addingTimeInterval(every)
        case .noCredential, .refused: nil
        }
    }
}
