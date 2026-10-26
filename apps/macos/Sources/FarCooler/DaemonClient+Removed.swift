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
    /// `.noAccess` when ssh says the runner refused this Mac's key, it has
    /// refused it `KeyRefusals.settlesAfter` reads in a row since last
    /// letting it in, over `KeyRefusals.settlesOver` at least (ov-818), and a
    /// verbose dial proves the refusal is the runner's own (review S1:
    /// `KeyRefusals.isProvenKeyRefusal`). Short of any of that, it's left as
    /// it was: a runner that can't be reached, retried on the backoff.
    ///
    /// A runner that already said this Mac was removed, or that settled to
    /// no access, keeps saying so through the refusals that follow. A Mac
    /// connected when it was removed reads `(released)` over its still-open
    /// ssh master (`a_real_sshd_master_outlives_a_revoke`), and meets the
    /// refused key only once that master ends.
    func removal(after reason: String) async -> HostState? {
        if !target.isEmpty, KeyRefusals.isKeyRefusal(reason) {
            if state == .removed || state == .noAccess { return state }
            guard keyRefusals.refused(at: now()) == .revoked, keyRefusals.mayProbe(at: now()),
                KeyRefusals.isProvenKeyRefusal(verbose: await refusalProbe(target))
            else { return nil }
            forgetTeamWork()
            return .noAccess
        }
        guard Self.mayBeRemoval(reason) else { return nil }
        let (data, _) = await runRaw(["member", "list", "--json"], background: true)
        guard let data, Self.saysRemoved(data) else { return nil }
        forgetTeamWork()
        return .removed
    }

    /// Forgets everything read from this runner, for one that doesn't let
    /// this Mac see its work anymore (ov-784, ov-818): its worktrees and
    /// panes, what they changed, what needs you there, its repositories,
    /// folders and layouts, its board and plan news, and (through
    /// `teamWorkForgotten`, which `TaskBoardStore.isHeld` reads) any board
    /// or plan the window holds open. Set before the fleet empties, since
    /// the window prunes its boards when the fleet publishes.
    ///
    /// The glances, widgets, watch and Live Activity rows are ov-820's.
    func forgetTeamWork() {
        teamWorkForgotten = true
        if fleet != .empty { fleet = .empty }
        if !changesInbox.isEmpty { changesInbox = [:] }
        if !repositories.isEmpty { repositories = [] }
        if !roots.isEmpty { roots = [] }
        if !layouts.isEmpty { layouts = [:] }
        if !planNews.isEmpty { planNews = [:] }
        if !heardReads.isEmpty { heardReads = [:] }
        forgetNeedsYou()
    }

    /// The verbose dial `removal(after:)` reads (review S1): the Mac's own
    /// ssh and config, with no master to reuse, strict host key checking
    /// whatever the config says, and nothing asked of a person. Its stderr.
    nonisolated static func probeRefusal(_ target: String) async -> String {
        await Task.detached {
            let ssh = Process()
            ssh.executableURL = URL(fileURLWithPath: "/usr/bin/ssh")
            ssh.arguments = probeArguments(target)
            let err = Pipe()
            ssh.standardError = err
            ssh.standardOutput = FileHandle.nullDevice
            ssh.standardInput = FileHandle.nullDevice
            guard (try? ssh.run()) != nil else { return "" }
            let data = err.fileHandleForReading.readDataToEndOfFile()
            ssh.waitUntilExit()
            return String(decoding: data, as: UTF8.self)
        }.value
    }

    nonisolated static func probeArguments(_ target: String) -> [String] {
        [
            "-v", "-n", "-o", "BatchMode=yes", "-o", "ControlMaster=no", "-o", "ControlPath=none",
            "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=10", "--", target, "true",
        ]
    }

    /// A read got in: this runner accepts this Mac's key, and remembers so
    /// across launches (ov-818). Written only when that's news.
    func letIn() {
        teamWorkForgotten = false
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
