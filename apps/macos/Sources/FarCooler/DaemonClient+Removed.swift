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
    /// verbose dial proves the runner refused the very key it named as the
    /// one this Mac came in with (`KeyRefusals.isProvenKeyRefusal`). No probe
    /// when the CLI's stderr already shows the agent in trouble, since the
    /// probe would only ask it again, and none oftener than every 30 s. Short
    /// of any of that, it's left as it was: a runner that can't be reached,
    /// retried on the backoff.
    ///
    /// A Mac connected when it was removed reads `(released)` over its
    /// still-open ssh master (`a_real_sshd_master_outlives_a_revoke`), so it
    /// shows `.removed` first, and meets the refused key once that master
    /// ends. `.removed` holds through refusals short of proof; a proven one
    /// moves it to `.noAccess`, whose words don't claim the member was
    /// removed, which a device-only revoke (`member.set`) didn't do.
    ///
    /// And `.noAccess` for a failure to reach the runner once it has said it
    /// released this Mac (`heardReleased`, ov-820), until a read gets in: over
    /// Tailcat a revoked Mac is never refused, only met with silence. Not a
    /// changed host key or a version mismatch, which say something else and
    /// which a re-admitted Mac must still be shown, as on the phones.
    func removal(after reason: String) async -> HostState? {
        if !target.isEmpty, keyRefusals.released, !KeyRefusals.showsHostKeyTrouble(reason),
            !looksVersionMismatch(reason)
        {
            forgetTeamWork()
            return .noAccess
        }
        if !target.isEmpty, KeyRefusals.isKeyRefusal(reason) {
            if state == .noAccess { return state }
            let held = state == .removed ? HostState.removed : nil
            guard keyRefusals.refused(at: now()) == .revoked, let key = keyRefusals.key,
                !KeyRefusals.showsAgentTrouble(reason), keyRefusals.mayProbe(at: now()),
                KeyRefusals.isProvenKeyRefusal(verbose: await refusalProbe(target), key: key)
            else { return held }
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

    /// The runner named the key this Mac came in with (`status --json`'s
    /// `keyFingerprint`): remembered, so a later refusal counts only when
    /// it's this key's.
    func learnKey(_ key: String?) {
        guard !target.isEmpty, let key, !key.isEmpty else { return }
        keyRefusals.learn(key: key, for: target, in: admissions)
    }

    /// The verbose dial `removal(after:)` reads: the same `ssh` the CLI runs
    /// (the first on the login shell's PATH, `CLI.environment`, so the same
    /// binary and system config) and the same `~/.ssh/config`, with no master
    /// to reuse, strict host key checking whatever the config says, and
    /// nothing asked of a person. Killed by its PID after `probeDeadline`,
    /// whatever it was waiting for. Its stderr.
    nonisolated static func probeRefusal(
        _ target: String, environment: [String: String] = CLI.environment, deadline: TimeInterval = probeDeadline
    ) async -> String {
        await runProbe(target, environment: environment, deadline: deadline).stderr
    }

    /// One probe's stderr, and whether its deadline killed it.
    struct ProbeRun: Sendable {
        var stderr: String
        var killed: Bool
    }

    /// The probe itself, `launch` being how `ssh` is found: `/usr/bin/env
    /// ssh` on the PATH in `environment`, or what a test gives.
    nonisolated static func runProbe(
        _ target: String, environment: [String: String], deadline: TimeInterval,
        launch: (program: String, leading: [String]) = ("/usr/bin/env", ["ssh"])
    ) async -> ProbeRun {
        await Task.detached {
            let ssh = Process()
            ssh.executableURL = URL(fileURLWithPath: launch.program)
            ssh.arguments = launch.leading + probeArguments(target)
            ssh.environment = environment
            let err = Pipe()
            ssh.standardError = err
            ssh.standardOutput = FileHandle.nullDevice
            ssh.standardInput = FileHandle.nullDevice
            guard (try? ssh.run()) != nil else { return ProbeRun(stderr: "", killed: false) }
            // By its own PID, and cancelled the moment it exits.
            let pid = ssh.processIdentifier
            let kill = DispatchWorkItem { Darwin.kill(pid, SIGKILL) }
            DispatchQueue.global().asyncAfter(deadline: .now() + deadline, execute: kill)
            ssh.waitUntilExit()
            kill.cancel()
            // Drained without waiting for every writer to close: a
            // ProxyCommand ssh started may still hold the pipe. `-v` writes a
            // few kilobytes, well inside a pipe's buffer.
            let stderr = String(decoding: drain(err.fileHandleForReading.fileDescriptor), as: UTF8.self)
            let killed = ssh.terminationReason == .uncaughtSignal && ssh.terminationStatus == SIGKILL
            return ProbeRun(stderr: stderr, killed: killed)
        }.value
    }

    /// What's in `fd` now, without blocking.
    nonisolated static func drain(_ fd: Int32) -> Data {
        _ = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK)
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 16_384)
        while true {
            let n = read(fd, &buffer, buffer.count)
            guard n > 0 else { break }
            data.append(buffer, count: n)
        }
        return data
    }

    nonisolated static let probeDeadline: TimeInterval = 10

    nonisolated static func probeArguments(_ target: String) -> [String] {
        [
            "-v", "-n", "-o", "BatchMode=yes", "-o", "ControlMaster=no", "-o", "ControlPath=none",
            "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=8", "-o", "ServerAliveInterval=3",
            "-o", "ServerAliveCountMax=1", "--", target, "true",
        ]
    }

    /// A read got in: this runner accepts this Mac's key, and remembers so
    /// across launches (ov-818). Written only when that's news.
    func letIn() {
        teamWorkForgotten = false
        guard !target.isEmpty, !keyRefusals.admitted || keyRefusals.refusals > 0 || keyRefusals.released else { return }
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
