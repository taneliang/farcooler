import Foundation

// How a phone reads the plan layer (ov-274): what its Plan view is showing
// while a read is out, and when none comes back. EXPERIMENTAL, as the layer
// is, and removable with it: nothing outside the Plan view reads this.
//
// The phones ask the runner for `plan.get` over their one connection (the Mac
// shells out to the CLI instead), so there's a read that can be refused, can
// fail, or can simply never be answered. Every one of those ends in a
// sentence and a Try Again. A spinner that never ends was the finding (review
// 1004i P6) and is a state this type can't be in for longer than `timeout`.

/// What the Plan view on a phone is showing.
public enum PlanReadState: Equatable, Sendable {
    /// A read is out and hasn't come back yet.
    case loading
    /// The runner is older than the plan layer: `PlanWords.needsUpdate`.
    case needsUpdate
    /// The read was refused, failed, wasn't understood or timed out:
    /// `PlanWords.couldntRead`, with Try Again.
    case unavailable
    case loaded(PlanModel)

    /// How long a phone waits for `plan.get` before it says it couldn't.
    public static let timeout: Duration = .seconds(15)

    /// Read the plan, and land in one state whatever happens.
    ///
    /// - Parameters:
    ///   - runnerCan: whether the runner advertises `board_plan`; false is
    ///     `needsUpdate` without asking, and nil (a build not read yet) asks.
    ///   - timeout: how long to wait for an answer.
    ///   - isUnsupported: whether an error is the runner saying it lacks the
    ///     capability, which reads as `needsUpdate` rather than a failure.
    ///   - fetch: the `plan.get` answer's bytes.
    public static func read(
        runnerCan: Bool?, timeout: Duration = PlanReadState.timeout,
        isUnsupported: @escaping @Sendable (Error) -> Bool = { _ in false },
        fetch: @escaping @Sendable () async throws -> Data
    ) async -> PlanReadState {
        if runnerCan == false { return .needsUpdate }
        // Whichever comes first, the answer or the timer, resumes this once.
        // Not a task group: a group waits for every child, and a call into the
        // client core can't be cancelled (`ClientCore.submit` holds a
        // continuation under a ticket), so a runner that never answers would
        // hold the group, and the spinner, open for good. The late answer, if
        // one ever comes, is dropped.
        return await withCheckedContinuation { (continuation: CheckedContinuation<PlanReadState, Never>) in
            let first = FirstOnly(continuation)
            let work = Task {
                do {
                    first.resume(.loaded(try PlanModel.decode(try await fetch())))
                } catch {
                    first.resume(isUnsupported(error) ? .needsUpdate : .unavailable)
                }
            }
            Task {
                try? await Task.sleep(for: timeout)
                first.resume(.unavailable)
                work.cancel()
            }
        }
    }

    /// The plan, once read.
    public var plan: PlanModel? {
        if case .loaded(let plan) = self { return plan }
        return nil
    }
}

/// A continuation resumed by whichever caller gets there first; the rest are
/// ignored.
private final class FirstOnly: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<PlanReadState, Never>?

    init(_ continuation: CheckedContinuation<PlanReadState, Never>) { self.continuation = continuation }

    func resume(_ state: PlanReadState) {
        lock.lock()
        let held = continuation
        continuation = nil
        lock.unlock()
        held?.resume(returning: state)
    }
}

/// The Tasks | Plan choice, kept per board on this phone as the Unread
/// section's collapsed state is.
public enum PlanChoice {
    /// Whether the board shows its plan. Tasks until someone picks Plan.
    public static func shown(
        host: String, workspace: String, defaults: UserDefaults = .standard
    ) -> Bool {
        defaults.bool(forKey: key(host: host, workspace: workspace))
    }

    public static func set(
        _ shown: Bool, host: String, workspace: String, defaults: UserDefaults = .standard
    ) {
        defaults.set(shown, forKey: key(host: host, workspace: workspace))
    }

    /// What the board draws: the plan, only where the runner keeps one and
    /// someone chose it. A runner without `board_plan` draws its tasks, and
    /// no control, whatever was chosen before.
    public static func showing(runnerKeepsPlan: Bool, chosen: Bool) -> Bool {
        runnerKeepsPlan && chosen
    }

    /// The Mac's key, so a person's choice reads the same on both.
    static func key(host: String, workspace: String) -> String {
        "board.plan.shown.\(host).\(workspace)"
    }
}

extension PlanWords {
    /// How many lines a theme's outcome gets before it truncates: the owner's
    /// ruling (ov-273), after "where each st…" was clipped at two.
    public static let outcomeLines = 3

    /// The button under a read that didn't come back.
    public static let tryAgain = "Try Again"
}

/// A theme's or lane's page on a phone, by the plan's own id, or an
/// orchestrator's page by its slot (ov-285).
public enum PhonePlanPage: Hashable, Codable, Sendable {
    case theme(String)
    case lane(String)
    /// An orchestrator's page (ov-269), by slot: `train`, `spend`.
    case page(String)

    /// What it is, for a title with no name to give it.
    public var word: String {
        switch self {
        case .theme: "Theme"
        case .lane: "Lane"
        case .page: "Page"
        }
    }
}

/// The client core's `plan` notice: that a board's plan was written, naming
/// the board. `{"event": "plan", "workspace": ...}`, as `event_line` writes it.
public struct PlanNews: Equatable, Sendable {
    public let workspace: String

    public init(workspace: String) { self.workspace = workspace }

    public init?(notice: [String: Any]) {
        guard notice["event"] as? String == "plan", let workspace = notice["workspace"] as? String,
            !workspace.isEmpty
        else { return nil }
        self.workspace = workspace
    }

    /// Whether it's about `board`, by workspace id; case-blind, since the
    /// runner writes ids in lowercase and a phone may hold them otherwise.
    public func touches(_ board: String) -> Bool { board.lowercased() == workspace.lowercased() }
}
