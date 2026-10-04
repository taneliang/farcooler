import Foundation

// The plan layer on a phone (ov-268 design 6.5, phase P5 is ov-274): one
// board's plan, as this connection last read it, and each theme's and lane's
// record.
//
// EXPERIMENTAL and opt-in. Nothing here is asked of a runner that doesn't
// advertise `board_plan`, and nothing is asked until someone picks Plan on a
// board. Removing the layer means deleting this file, the Plan views, the
// `plan` route and the call sites that name `plans`.
//
// The rules are AgentKit's (`PlanModel`, `PlanReadState`); this reads over
// the connection and keeps the answers.

/// What this connection holds of the plan layer, by board.
///
/// A class of its own, held by one property on `Connection`, so the plan's
/// state redraws only the screens that show a plan.
@MainActor
final class PlanReads: ObservableObject {
    /// Each board's plan read, by workspace id.
    @Published private(set) var states: [String: PlanReadState] = [:]
    /// Each theme's and lane's record, by page, once read.
    @Published private(set) var records: [PhonePlanPage: PlanRecord] = [:]
    /// Boards with a read out, and boards whose news came while it was.
    private var reading: Set<String> = []
    private var movedAgain: Set<String> = []

    func state(_ workspace: String) -> PlanReadState? { states[workspace] }

    fileprivate func set(_ state: PlanReadState, for workspace: String) {
        if states[workspace] != state { states[workspace] = state }
    }

    fileprivate func set(_ record: PlanRecord, for page: PhonePlanPage) {
        if records[page] != record { records[page] = record }
    }

    /// Every board whose plan has been asked for, for a notice that doesn't
    /// name one.
    var asked: [String] { Array(states.keys) }

    /// Claim a board's read, or say another is out and must go round again.
    fileprivate func begin(_ workspace: String) -> Bool {
        guard !reading.contains(workspace) else {
            movedAgain.insert(workspace)
            return false
        }
        reading.insert(workspace)
        return true
    }

    /// Whether the board moved while it was read, and so is read once more.
    fileprivate func again(_ workspace: String) -> Bool { movedAgain.remove(workspace) != nil }

    fileprivate func end(_ workspace: String) { reading.remove(workspace) }
}

extension Connection {
    /// Whether this runner keeps a plan: only then is there a control. A
    /// runner not read yet, or too old, offers nothing new.
    var keepsPlan: Bool { knownBuild?.can(.boardPlan) == true }

    /// How long a read waits for the runner. A UI test shortens it
    /// (`-phone-plan-timeout`), so an unanswered read is seen to end.
    private static var planTimeout: Duration {
        #if DEBUG
        if let seconds = UserDefaults.standard.string(forKey: "phone-plan-timeout").flatMap(Double.init) {
            return .seconds(seconds)
        }
        #endif
        return PlanReadState.timeout
    }

    /// Read `summary`'s board's plan, landing in a state whatever the runner
    /// does: the plan, an update to ask for, or a read that failed or was
    /// never answered (`PlanReadState.read`).
    func readPlan(_ summary: WorkspaceSummary) async {
        let key = summary.id
        guard plans.begin(key) else { return }
        defer { plans.end(key) }
        if plans.state(key)?.plan == nil { plans.set(.loading, for: key) }
        repeat {
            guard let board = summary.boardWorkspace else {
                plans.set(.needsUpdate, for: key)
                return
            }
            let state = await PlanReadState.read(
                runnerCan: knownBuild?.can(.boardPlan), timeout: Self.planTimeout,
                isUnsupported: { ClientCore.refusalWord(of: $0) == "capability-unsupported" },
                fetch: { try await self.rpc("plan.get", ["workspace": board]) })
            // A read that fails over a plan already in hand keeps the plan:
            // the screen draws it, and the next notice reads again.
            if state == .unavailable, plans.state(key)?.plan != nil { break }
            plans.set(state, for: key)
        } while plans.again(key)
    }

    /// A theme's or lane's record, for its page's timeline. A read that
    /// doesn't come back leaves the page without one: the page doesn't wait
    /// on it.
    func readPlanRecord(_ page: PhonePlanPage) async {
        let subject: [String: Any]
        switch page {
        case .theme(let id): subject = ["theme": id]
        case .lane(let id): subject = ["lane": id]
        }
        guard keepsPlan, let data = try? await rpc("plan.events", subject),
            let record = try? PlanRecord.decode(data)
        else { return }
        plans.set(record, for: page)
    }

    /// News that a plan moved: `notice` is the client core's line. Boards
    /// that have asked for their plan read it again; others never will until
    /// someone picks Plan.
    func hearPlan(_ notice: [String: Any]) async {
        guard let news = PlanNews(notice: notice) else { return }
        for board in boardList where plans.state(board.id) != nil && news.touches(board.id) {
            await readPlan(board)
            for page in plans.records.keys { await readPlanRecord(page) }
        }
    }

    /// Boards a task or resync notice moved re-read their plan too: a
    /// theme's counts and "needs you" are the board's statuses, derived on
    /// read.
    func rereadPlans(for boards: [WorkspaceSummary]) async {
        for board in boards where plans.state(board.id) != nil { await readPlan(board) }
    }
}
