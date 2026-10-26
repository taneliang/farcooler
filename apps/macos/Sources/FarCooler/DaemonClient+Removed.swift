import AgentKit
import Foundation

/// A Mac whose member was removed from a runner (ov-784): the runner refuses
/// every read from it `not-yours` but `member list`, which answers
/// `(released)` (ov-758), so the window can say "You were removed from this
/// runner" instead of a runner that can't be reached.
extension DaemonClient {
    /// After a read `reason` failed with: `.removed` when the runner refused
    /// it as someone else's and says this Mac's member was removed, which
    /// also drops everything read from it before, so none of the team's work
    /// stays on screen. Nil for any other failure, or a list that can't be
    /// read, which leaves the failure as it was.
    func removal(after reason: String) async -> HostState? {
        guard Self.mayBeRemoval(reason) else { return nil }
        let (data, _) = await runRaw(["member", "list", "--json"], background: true)
        guard let data, Self.saysRemoved(data) else { return nil }
        if fleet != .empty { fleet = .empty }
        return .removed
    }

    /// Whether a failure could be a removal: a refusal as someone else's.
    nonisolated static func mayBeRemoval(_ reason: String) -> Bool {
        TaskFailure.code(in: reason) == "not-yours"
    }

    /// Whether `member list --json` says this device was a member's and is
    /// nobody's now.
    nonisolated static func saysRemoved(_ data: Data) -> Bool {
        RunnerPeople(json: data)?.isReleased == true
    }
}
