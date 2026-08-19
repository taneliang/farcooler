import Foundation

/// How much the watch knows, and whether it may act on it.
///
/// Three states and no fourth, because the fourth everyone reaches for —
/// "probably still true, the data looks recent" — is the one that gets somebody
/// hurt. A watch that offers an Allow button it cannot deliver is worse than a
/// watch that says it cannot reach the phone: the person taps it, sees nothing
/// wrong, walks away believing they answered, and the agent is still sitting
/// there an hour later.
///
/// **In AgentKit rather than in the watch target, and only for the tests.**
/// Nothing on the phone compiles this file. The rule it carries is the spec's
/// — the three reachability states are distinguishable, and actions are
/// disabled in two of them — and a rule that lives in a watchOS app target is
/// a rule `swift test` cannot reach, which left the whole of it enforced by
/// nobody. It is pure: a snapshot or no snapshot, reachable or not. Nothing
/// here imports WatchConnectivity, SwiftUI, or anything else the watch alone
/// has.
public enum WatchState: Sendable, Equatable {
    /// The phone is reachable and this is what it last said. Actions work.
    case live(FleetSnapshot)
    /// The last thing the phone said, with no way to reach it now. Render it,
    /// say how old it is, and disable every action.
    case cached(FleetSnapshot)
    /// The phone has never been heard from on this watch, or its snapshot could
    /// not be read. An empty state, not a spinner: there is nothing pending.
    case nothing

    /// The whole rule, in one place that can be tested.
    ///
    /// `reachable` is `WCSession.isReachable` and nothing else. `capturedAt` is
    /// deliberately not a parameter: age is a separate question, answered where
    /// every other surface answers it — `FleetSnapshot.confidence(in:at:)` —
    /// and a five-second-old snapshot with an unreachable phone is still
    /// `.cached`, because recency is not a link. Passing the date in at all
    /// would be an invitation to consult it.
    public static func resolve(snapshot: FleetSnapshot?, reachable: Bool) -> WatchState {
        guard let snapshot else { return .nothing }
        return reachable ? .live(snapshot) : .cached(snapshot)
    }

    /// What to draw, or nil when there is nothing known.
    public var snapshot: FleetSnapshot? {
        switch self {
        case let .live(snapshot), let .cached(snapshot): snapshot
        case .nothing: nil
        }
    }

    /// Whether a button on this screen can actually do what it says.
    ///
    /// Asked here rather than re-derived per screen, so a screen added later
    /// cannot forget the rule and ship an Allow button that goes nowhere.
    public var canAct: Bool {
        if case .live = self { return true }
        return false
    }
}
