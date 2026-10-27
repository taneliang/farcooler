import Foundation
import os

// Coming back to the app, and bringing a new link up (ov-613). The decisions
// are AgentKit's (`LinkRevival`), where a fake clock and a fake core test
// them; this file is the mechanism and the step log.
//
// The log is how the time to reconnect is measured rather than estimated:
// `log stream --predicate 'subsystem == "com.farcooler.FarCooler" AND
// category == "link"'`, or the "link" intervals in Instruments. Each line starts
// with the connection's ordinal, `[2]`, so several runners can be told apart.
// Step names and milliseconds only, never a runner's name, an address or an
// error's text.

extension Connection {
    static let linkLog = Logger(subsystem: "com.farcooler.FarCooler", category: "link")
    static let linkSignposts = OSSignposter(subsystem: "com.farcooler.FarCooler", category: "link")
    private static var ordinalsIssued = 0

    static func nextOrdinal() -> Int {
        ordinalsIssued += 1
        return ordinalsIssued
    }

    /// When the app last came back, until the fleet it repaints from lands.
    private static var returnedAt: [Int: ContinuousClock.Instant] = [:]

    /// The app came back: probe a link that says it's up, dial one that's
    /// retrying or stuck, and leave a question for a person alone.
    ///
    /// Not the boards, as a rule. A link that survived kept its event channel,
    /// and the core queued the notices that arrived meanwhile, so each board
    /// that moved is read on its own news. Only a runner with no live channel
    /// had nobody to tell it; see `boardsMayHaveMissedNews`. A link that did
    /// not survive comes back through `reconnect`, which reads every board.
    func returnToForeground() {
        let standing: LinkRevival.Standing
        switch phase {
        case .connected:
            standing = .connected
        case .connecting, .reconnecting:
            if let began = dialStartedAt {
                standing = .dialing(for: .now - began, beganBeforeLeaving: leftAt.map { began < $0 } ?? false)
            } else {
                // Between attempts: a backoff waiting, or a dial not yet out.
                standing = phase == .connecting ? .dialing(for: .zero, beganBeforeLeaving: false) : .retrying
            }
        case .failed:
            standing = .retrying
        case .needsApproval:
            standing = .waitingOnPerson
        }
        let began = ContinuousClock.now
        let me = linkOrdinal
        Self.returnedAt[me] = began
        log("foreground: \(standing)")
        let vouching = standing == .connected
        if vouching { returns += 1 }
        let core = core
        Task { [weak self] in
            let outcome = await LinkRevival.onReturn(
                standing, clock: ContinuousClock(),
                probe: { await Self.probe(core, me) },
                dialNow: { [weak self] outcome in self?.dialOnReturn(outcome) },
                refresh: { [weak self] in
                    guard let self else { return }
                    await self.refresh()
                    if await self.boardsMayHaveMissedNews() { self.loadBoardsDetached() }
                })
            if vouching { self?.returns -= 1 }
            self?.log("foreground verdict: \(outcome) in \(Self.ms(since: began)) ms")
        }
    }

    /// A dial the return decided on. A dead link's is skipped if a drop the
    /// fleet read found has already dialed.
    private func dialOnReturn(_ outcome: LinkRevival.Outcome) {
        if case .redialed = outcome, phase != .connected { return }
        log("dialing on return: \(outcome)")
        reconnectIfDue()
    }

    /// The probe, as a verdict. A refusal or an unreadable answer is still
    /// the runner speaking.
    private static func probe(_ core: ClientCore, _ ordinal: Int) async -> LinkRevival.Probe {
        let began = ContinuousClock.now
        let found: LinkRevival.Probe
        do {
            _ = try await core.call("probe")
            found = .answered
        } catch let error as ClientCore.CoreError {
            switch error {
            case .timedOut: found = .silent
            case .disconnected, .unreached: found = .dropped
            // A core that never started has no link a dial would mend.
            case .rejected, .malformed, .notStarted: found = .answered
            }
        } catch {
            found = .answered
        }
        linkLog.info("[\(ordinal)] probe: \(String(describing: found), privacy: .public) in \(ms(since: began)) ms")
        return found
    }

    /// A dial starting now: stamped for a return that finds it stuck, and
    /// timed in the log.
    func dialing() -> Dialing {
        let began = ContinuousClock.now
        dialStartedAt = began
        log("dial")
        return Dialing(began: began, interval: Self.linkSignposts.beginInterval("dial"))
    }

    /// The dial is over, either way: its interval ends, its time is logged,
    /// and it is no longer the dial in flight unless a newer one is.
    func dialEnded(_ dial: Dialing, connected: Bool) {
        Self.linkSignposts.endInterval("dial", dial.interval)
        log("\(connected ? "connected" : "dial failed") in \(Self.ms(since: dial.began)) ms")
        if dialStartedAt == dial.began { dialStartedAt = nil }
        if connected { lastDialBegan = dial.began }
    }

    struct Dialing {
        let began: ContinuousClock.Instant
        let interval: OSSignpostIntervalState
    }

    /// The first fleet on a new link landed, decoded, in `fleet`: what the
    /// screen repaints from.
    func firstFleetLanded() {
        if let began = lastDialBegan { log("first fleet in \(Self.ms(since: began)) ms from dial") }
    }

    /// Any fleet landed on the link that is up: the end of a return.
    func fleetLanded() {
        guard phase == .connected, let back = Self.returnedAt.removeValue(forKey: linkOrdinal) else { return }
        log("live in \(Self.ms(since: back)) ms from foreground")
    }

    /// What a new link reads, all at once: the fleet (and the build beside
    /// it), the repositories and the themes. The fleet paints as soon as it
    /// lands, whatever the others are doing. The boards are swept once all of
    /// them are in, because the sweep's list is built from them.
    func bringUpReads(_ dialed: Dialing) async {
        let interval = Self.linkSignposts.beginInterval("bring-up")
        await LinkRevival.bringUp(
            [
                { [weak self] in await self?.refresh() },
                { [weak self] in await self?.loadRepositories() },
                { [weak self] in await self?.loadThemes() },
            ],
            afterwards: { [weak self] in
                self?.loadBoardsDetached()
                self?.startPolling()
            })
        Self.linkSignposts.endInterval("bring-up", interval)
        log("bring-up done in \(Self.ms(since: dialed.began)) ms from dial")
    }

    /// The `host` read in flight, or a new one: one call however many reads
    /// are waiting on the build.
    func readBuild() -> Task<Void, Never> {
        if let buildRead { return buildRead }
        let read = Task { [weak self] in
            await self?.loadDaemonBuild()
            self?.buildRead = nil
        }
        buildRead = read
        return read
    }

    private func log(_ step: String) {
        Self.linkLog.info("[\(self.linkOrdinal)] \(step, privacy: .public)")
    }

    nonisolated static func ms(since start: ContinuousClock.Instant) -> Int {
        let gone = ContinuousClock.now - start
        return Int(gone.components.seconds * 1000 + gone.components.attoseconds / 1_000_000_000_000_000)
    }
}
