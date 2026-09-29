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

// MARK: - What the list draws (ov-55 4C.2)

/// The watch's list, in the order it draws it: what needs you, then the
/// agents (spec §7).
///
/// Here rather than in the view for `WatchState`'s reason: the order is a rule
/// a test has to be able to reach, and a watchOS target has no tests.
///
/// Both halves come off the snapshot as the phone wrote them. The items are in
/// the app's merged rank order (`NeedsYou.merge`), and the agents in `ranked`,
/// the order the list always had. An agent with an item stays in the agents
/// too: the item is about what it's waiting on, the row is about the agent.
public struct WatchList: Sendable, Equatable {
    /// One row, of either kind.
    public enum Row: Sendable, Equatable, Identifiable {
        case item(NeedsYouItem)
        case agent(FleetSnapshot.Agent)

        public var id: String {
            switch self {
            case let .item(item): "item:\(item.key)"
            case let .agent(agent): "agent:\(agent.id)"
            }
        }
    }

    /// Nothing, for a snapshot from a phone too old to write a list: the
    /// section isn't drawn at all rather than drawn empty.
    public let items: [NeedsYouItem]
    public let agents: [FleetSnapshot.Agent]

    public init(_ snapshot: FleetSnapshot) {
        items = snapshot.needsYou ?? []
        agents = snapshot.ranked
    }

    /// Every row, items first.
    public var rows: [Row] { items.map(Row.item) + agents.map(Row.agent) }
}

/// What the watch can do about one item.
public enum WatchItemAction: Sendable, Equatable {
    /// Answer it here: the ask's own options, in its order. Each sends
    /// `WatchRequest.answer`, the path a pane's own card takes.
    case answer([NeedsYouAction])
    /// Open the agent it's about, which the watch lists: a block the watch
    /// can't answer, but whose agent it can show and prompt.
    case agent(terminal: String)
    /// Nothing the watch can do: a decision's options are a board note, and a
    /// review is read on a bigger screen. The row says "Open on iPhone".
    case onPhone

    /// The one rule.
    ///
    /// **An ask answers here only with everything answering needs:** its
    /// terminal, its ask id, and at least one option. Below Control scope a
    /// runner sends none of the last two, and a button with nothing to send is
    /// the Allow button that goes nowhere. `open` is never an answer.
    public static func of(_ item: NeedsYouItem, in snapshot: FleetSnapshot) -> WatchItemAction {
        let answers = item.actions.filter { !$0.isOpen }
        if item.kind == .ask, item.terminal != nil, item.askID != nil, !answers.isEmpty {
            return .answer(answers)
        }
        if item.kind == .ask || item.kind == .blocked, let terminal = item.terminal?.id,
            snapshot.agents.contains(where: { $0.id == terminal })
        {
            return .agent(terminal: terminal)
        }
        return .onPhone
    }
}

extension NeedsYouItem {
    /// What to send for one of this ask's options, or nil when it has nothing
    /// to send it with.
    public func watchRequest(answering action: NeedsYouAction) -> WatchRequest? {
        guard let terminal = terminal?.id, let askID, !action.isOpen else { return nil }
        return .answer(terminal: terminal, request: askID, option: action.id)
    }

    /// Where it is, for the line under its question: "Billing · claude", or
    /// "Billing · bil-7" for a task's item. The workspace leads, as it does on
    /// every other surface; without one, the subject alone.
    public var watchPlace: String {
        let subject = task?.key ?? terminal?.label ?? ""
        return [workspaceName, subject].filter { !$0.isEmpty }.joined(separator: " · ")
    }

    /// The hedge under an item the app derived from a runner too old to send
    /// its own list (spec §2.6), or nil for one the runner sent. It's a
    /// blocked agent the app saw, and the runner may have asks and decisions
    /// it can't say: the watch names the runner no further, since it holds
    /// the runner's id and not its name.
    public var watchHedge: String? {
        isDerived ? "Older runner: update it to see asks and decisions." : nil
    }

    /// The status word its mark is drawn from: amber for everything that
    /// needs you, and a review's own mark for a review, as the widget draws
    /// reviews.
    public var watchStatus: String { kind == .review ? "done" : "blocked" }
}
