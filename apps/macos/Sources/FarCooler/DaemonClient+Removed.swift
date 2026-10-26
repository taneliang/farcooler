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
    ///
    /// `.noAccess` when ssh says the runner refused this Mac's key, and it
    /// has refused it `KeyRefusals.settlesAfter` reads in a row since last
    /// letting it in, over `KeyRefusals.settlesOver` at least (ov-818),
    /// which drops what was read from the runner too. Fewer are left as they
    /// were, a runner that can't be reached, retried on the backoff: a
    /// refusal can pass.
    func removal(after reason: String) async -> HostState? {
        if !target.isEmpty, KeyRefusals.isKeyRefusal(reason) {
            guard keyRefusals.refused(at: now()) == .revoked else { return nil }
            forgetTeamWork()
            return .noAccess
        }
        guard Self.mayBeRemoval(reason) else { return nil }
        let (data, _) = await runRaw(["member", "list", "--json"], background: true)
        guard let data, Self.saysRemoved(data) else { return nil }
        if fleet != .empty { fleet = .empty }
        return .removed
    }

    /// Forgets everything read from this runner, for one that doesn't let
    /// this Mac see its work anymore (ov-818): its worktrees and panes, what
    /// they changed, and what needs you there.
    func forgetTeamWork() {
        if fleet != .empty { fleet = .empty }
        if !changesInbox.isEmpty { changesInbox = [:] }
        forgetNeedsYou()
    }

    /// A read got in: this runner accepts this Mac's key, and remembers so
    /// across launches (ov-818). Written only when that's news.
    func letIn() {
        guard !target.isEmpty, !keyRefusals.admitted || keyRefusals.refusals > 0 else { return }
        keyRefusals.letIn(target, in: admissions)
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
