import Foundation

/// The plan on the glance (ov-310, ov-268 P8): the one board the watch, the
/// widgets and the Live Activity say something about.
///
/// The watch has no sockets, so it hears this from the relay and nowhere
/// else: `/v1/pulse` (`RunnerPulse.Reading`) and the card's content state
/// (`AgentCardState.plan`). The runner decided it (`plan_glance.rs`) and the
/// relay picked the board (`leadPlan`); a surface only says it.
///
/// Ticket keys and the board's name, never card text.
public struct PlanGlance: Codable, Sendable, Hashable {
    /// A ticket in Now: its key and its stage (ov-872).
    public struct Ticket: Codable, Sendable, Hashable {
        public var ticket: String
        public var stage: TicketStage

        public init(ticket: String, stage: TicketStage) {
            self.ticket = ticket
            self.stage = stage
        }

        private enum CodingKeys: String, CodingKey { case ticket, stage }

        public func encode(to encoder: Encoder) throws {
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encode(ticket, forKey: .ticket)
            try container.encode(stage.rawValue, forKey: .stage)
        }
    }

    /// The board, by its workspace's name.
    public var workspace: String
    /// The board's Needs You count: the number its title bar says on the Mac
    /// (`WorkspaceNeedsYou.count`), its needs-you items and themes asking.
    public var needsYou: Int
    /// Up to two tickets being built, reviewed or landed.
    public var now: [Ticket]
    /// The ticket next up, or nil when nothing is queued.
    public var next: String?
    /// The runner that said it, by the name it beats with (ov-310 review H2).
    /// Nil from a relay before it.
    public var runner: String?
    /// How long ago, in milliseconds, the relay last heard that runner, by
    /// the relay's own clock. Nil from a relay before it.
    public var heardAgo: Double?
    /// The runner has gone quiet: this is its last word, and its Now is a
    /// claim nobody has vouched for since.
    public var quiet: Bool

    public init(
        workspace: String, needsYou: Int, now: [Ticket], next: String? = nil,
        runner: String? = nil, heardAgo: Double? = nil, quiet: Bool = false
    ) {
        self.workspace = workspace
        self.needsYou = needsYou
        self.now = now
        self.next = next
        self.runner = runner
        self.heardAgo = heardAgo
        self.quiet = quiet
    }

    private enum CodingKeys: String, CodingKey { case workspace, needsYou, now, next, runner, heardAgo, quiet }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(workspace, forKey: .workspace)
        try container.encode(needsYou, forKey: .needsYou)
        try container.encode(now, forKey: .now)
        try container.encodeIfPresent(next, forKey: .next)
        try container.encodeIfPresent(runner, forKey: .runner)
        try container.encodeIfPresent(heardAgo, forKey: .heardAgo)
        if quiet { try container.encode(true, forKey: .quiet) }
    }

    /// Lenient, as the card around it is: a ticket of the wrong shape costs the
    /// ticket, and a count of the wrong shape reads as none. Since ov-872 a Now
    /// entry is `{ticket, stage}`; one in the old `{name, state}` shape is
    /// dropped, as the relay drops it.
    public init(from decoder: Decoder) throws {
        struct Loose: Decodable {
            var ticket: Ticket?
            init(from decoder: Decoder) throws { ticket = try? Ticket(from: decoder) }
        }
        let container = try decoder.container(keyedBy: CodingKeys.self)
        workspace = try container.decode(String.self, forKey: .workspace)
        needsYou = max(0, ((try? container.decodeIfPresent(Int.self, forKey: .needsYou)) ?? nil) ?? 0)
        now = (((try? container.decodeIfPresent([Loose].self, forKey: .now)) ?? nil) ?? []).compactMap(\.ticket)
        let next = (try? container.decodeIfPresent(String.self, forKey: .next)) ?? nil
        self.next = next?.isEmpty == false ? next : nil
        let runner = (try? container.decodeIfPresent(String.self, forKey: .runner)) ?? nil
        self.runner = runner?.isEmpty == false ? runner : nil
        heardAgo = ((try? container.decodeIfPresent(Double.self, forKey: .heardAgo)) ?? nil).map { max(0, $0) }
        quiet = ((try? container.decodeIfPresent(Bool.self, forKey: .quiet)) ?? nil) ?? false
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

    /// One ticket as a row says it: "ov-312 · In Review".
    public static func words(_ ticket: Ticket) -> String {
        "\(ticket.ticket) · \(ticket.stage.word)"
    }

    /// Now on one line: "Now: ov-312 In Review, ov-310 Building"; nil when
    /// nothing is past waiting.
    public var nowLine: String? {
        guard !now.isEmpty else { return nil }
        return "Now: " + now.map { "\($0.ticket) \($0.stage.word)" }.joined(separator: ", ")
    }

    /// "Next: ov-313"; nil when nothing is queued.
    public var nextLine: String? { next.map { "Next: \($0)" } }

    /// What VoiceOver reads for the whole glance.
    public var spoken: String {
        var parts = ["\(workspace) board" + (needsYouWords.map { ", \($0)" } ?? "")]
        if !now.isEmpty {
            parts.append("Now: " + now.map { "\($0.ticket), \($0.stage.word.lowercased())" }.joined(separator: "; "))
        }
        if let next { parts.append("Next up: \(next)") }
        return parts.joined(separator: ". ") + "."
    }
}

/// Where a ticket is on the line (ov-872). Here rather than in
/// `PlanModel.swift` so the watch and the widgets, which compile this file and
/// not that one, say a stage as the Plan view does. The words are the
/// runner's store's (`waiting`, `building`, `reviewing`, `needs_you`,
/// `landing`, `landed`, `dropped`).
public enum TicketStage: String, Decodable, Sendable, CaseIterable {
    case waiting, building, reviewing
    case needsYou = "needs_you"
    case landing, landed, dropped, unknown

    public init(from decoder: Decoder) throws {
        self = TicketStage(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
    }

    /// Neither landed nor dropped.
    public var isLive: Bool { self != .landed && self != .dropped }

    /// On the line and being worked, or waiting on the owner: past waiting.
    public var isPastWaiting: Bool { isLive && self != .waiting }

    /// The stage alone, in title case as the board's statuses are
    /// (`PlanWords.stage`).
    public var word: String {
        switch self {
        case .waiting: "Waiting"
        case .building: "Building"
        case .reviewing: "In Review"
        case .needsYou: "Needs You"
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
