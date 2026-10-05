import Foundation

extension Connection {
    /// What a runner notice moves besides the fleet: which BOARD to read
    /// again, if it names one.
    ///
    /// Every notice still means "re-read it" and none carries a delta, see
    /// `FleetEvent` in `crates/client/src/session.rs`, so the fleet is re-read
    /// from one place for all of them, and an app that applied deltas would
    /// have to be right about reconciliation creating and deleting rows in the
    /// same pass, and about the CLI and an agent editing the same state from
    /// outside it.
    ///
    /// A board is the exception because it is not in the fleet. A `task`
    /// notice names its board, the workspace the task is on and on a move the
    /// one it left, so a `task.list` per board it moved is what it costs, and
    /// `resync`, where the queue overflowed or the runner dropped events it
    /// owed this link (`events_missed`), is every board. Which boards a notice
    /// moves is `RunnerBoards.touched`'s to say. A `resync` reads the fleet
    /// before any board, since it is when the fleet is most behind.
    ///
    /// Its own function so a test can deliver a notice to it: the harness does
    /// (`-phone-plan`), since the client core's queue isn't there.
    func hearNews(_ notice: [String: Any]) async {
        switch notice["event"] as? String {
        case "task":
            guard let moved = BoardNotice(notice: notice) else { break }
            let touched = RunnerBoards.touched(by: moved, among: boardList)
            for board in touched { await readBoard(board) }
            await rereadPlans(for: touched)
        case "resync":
            await loadNeedsYou()
            await loadBoards()
            await rereadPlans(for: boardList)
        // The plan layer moved on a board (ov-274).
        case "plan": await hearPlan(notice)
        // A board's orchestrator page was written or removed (ov-285).
        case "pages": await hearPages(notice)
        // The rollup moved: an ask held or settled, a decision asked or
        // answered, a review in or out. See `loadNeedsYou`.
        case "needs_you": await loadNeedsYou()
        default: break
        }
    }
}
