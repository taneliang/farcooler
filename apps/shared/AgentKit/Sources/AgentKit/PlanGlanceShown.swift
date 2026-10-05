import Foundation

/// What a plan surface shows, and why (ov-310 review H2).
///
/// "No board has a plan yet" is a fact about the owner's boards, so it's said
/// only when the relay answered and named none. Everything else a surface can
/// meet says what it actually knows:
///
/// - a quiet runner's last plan, with "Can't reach Studio" and how long ago;
/// - the last plan the relay gave, with its age, when the relay can't be
///   asked now (offline, a timeout, a locked watch);
/// - signed out, which only signing in fixes;
/// - and, with nothing remembered and no answer, that it can't check.
public enum PlanGlanceShown: Equatable, Sendable {
    case plan(PlanGlance, caveat: PlanCaveat?)
    case noPlan
    case signedOut
    case unknown

    /// The sentence a surface draws in place of a board.
    public var message: String? {
        switch self {
        case .plan: nil
        case .noPlan: "No board has a plan yet."
        case .signedOut: "Sign in to see your plan."
        case .unknown: "Can’t check your plan right now."
        }
    }
}

/// Why a plan on screen isn't current, and how old it is.
public struct PlanCaveat: Equatable, Sendable {
    /// How long ago the plan was last vouched for.
    public var age: TimeInterval
    /// The runner that went quiet, when that's the reason.
    public var cantReach: String?

    public init(age: TimeInterval, cantReach: String? = nil) {
        self.age = age
        self.cantReach = cantReach
    }

    /// "Can’t reach Studio · 3h ago", or "As of 3h ago".
    public var line: String {
        if let cantReach { return "Can’t reach \(cantReach) · \(GlanceAge.stated(age))" }
        return "As of \(GlanceAge.stated(age))"
    }
}

/// The last plan a surface was given, kept beside the fleet's snapshot in the
/// app group's container so a widget, the watch app or its complication that
/// can't ask the relay now still says what it last knew, with its age.
public struct PlanGlanceSeen: Codable, Equatable, Sendable {
    public var glance: PlanGlance
    /// When the runner last vouched for it: the answer's time less the
    /// relay's `heardAgo`.
    public var asOf: Date

    public init(glance: PlanGlance, asOf: Date) {
        self.glance = glance
        self.asOf = asOf
    }
}

public enum PlanGlanceMemory {
    static let fileName = "plan-glance.json"

    /// What to show for `reading`, given what was `remembered`, and what to
    /// remember from now on (nil forgets).
    public static func shown(
        reading: RunnerPulse.Reading, remembered: PlanGlanceSeen?, at now: Date
    ) -> (shown: PlanGlanceShown, remember: PlanGlanceSeen?) {
        switch reading {
        case .answered(_, _, nil):
            return (.noPlan, nil)
        case let .answered(_, _, glance?):
            let seen = PlanGlanceSeen(glance: glance, asOf: now.addingTimeInterval(-(glance.heardAgo ?? 0) / 1000))
            return (.plan(glance, caveat: caveat(seen, at: now, current: true)), seen)
        case .failed:
            guard let remembered else { return (.unknown, nil) }
            return (.plan(remembered.glance, caveat: caveat(remembered, at: now, current: false)), remembered)
        case .noCredential, .refused:
            return (.signedOut, nil)
        }
    }

    /// A quiet runner's plan always says so; a plan from an answer that came
    /// now says nothing more; one remembered says how old it is.
    private static func caveat(_ seen: PlanGlanceSeen, at now: Date, current: Bool) -> PlanCaveat? {
        let age = max(0, now.timeIntervalSince(seen.asOf))
        if seen.glance.quiet { return PlanCaveat(age: age, cantReach: seen.glance.runner ?? "your runner") }
        return current ? nil : PlanCaveat(age: age)
    }

    public static func read(from container: URL) -> PlanGlanceSeen? {
        guard let data = try? Data(contentsOf: container.appendingPathComponent(fileName)) else { return nil }
        return try? JSONDecoder().decode(PlanGlanceSeen.self, from: data)
    }

    public static func write(_ seen: PlanGlanceSeen?, to container: URL) {
        let file = container.appendingPathComponent(fileName)
        guard let seen, let data = try? JSONEncoder().encode(seen) else {
            try? FileManager.default.removeItem(at: file)
            return
        }
        try? data.write(to: file, options: .atomic)
    }

    /// `shown`, against the app group's own memory, which it then updates.
    public static func look(_ reading: RunnerPulse.Reading, at now: Date = Date()) -> PlanGlanceShown {
        let container = SnapshotStore.groupIdentifier.flatMap(SnapshotStore.container(forGroup:))
        let (shown, remember) = shown(reading: reading, remembered: container.flatMap(read(from:)), at: now)
        if let container { write(remember, to: container) }
        return shown
    }
}

extension PlanGlance {
    /// The caveat a card draws its plan with: a quiet runner's, aged as of
    /// the push. A card from a beating runner is as current as its push.
    public var cardCaveat: PlanCaveat? {
        quiet ? PlanCaveat(age: (heardAgo ?? 0) / 1000, cantReach: runner ?? "your runner") : nil
    }
}
