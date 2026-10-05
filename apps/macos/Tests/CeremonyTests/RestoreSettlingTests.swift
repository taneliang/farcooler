import Combine
import Foundation
import Testing

@testable import Far_Cooler

/// A runner that answers quickly never flashes "Connecting…" on a relaunch
/// (ov-296): a restore draws nothing for its first `connectingDelay`, says
/// it's connecting after that, and goes back to its place as soon as its
/// runner changes. Every wait here is a number handed in, never the wall
/// clock, except the change wake's, which a send ends.
struct RestoreSettlingTests {
    typealias Placeholder = FleetPlaceholder

    /// The phase a window going back to a place on `runner` shows after
    /// `waited` seconds, over a fleet that already has worktrees.
    static func phase(_ runner: String?, waited: TimeInterval, restoring: Bool = true) -> Placeholder.Phase {
        Placeholder.phase(
            hasWorktrees: true, localLoaded: true, localError: nil, hasRepositories: true,
            restoringOn: runner, waited: waited, restoring: restoring)
    }

    /// This Mac's runner answers in about 300 ms: inside the delay, the
    /// window shows nothing, not "Connecting to This Mac…".
    @Test func aRunnerThatAnswersWithinTheDelayNeverShowsConnecting() {
        for waited in [0, 0.1, 0.3, Placeholder.connectingDelay - 0.001] {
            #expect(Self.phase("", waited: waited) == .settling, "waited \(waited)")
            #expect(Self.phase("e-liang@studio", waited: waited) == .settling, "waited \(waited)")
        }
    }

    /// One still not up once the delay is over says so, and after the
    /// connect timeout, that it can't be reached.
    @Test func aRunnerStillComingAfterTheDelayShowsConnecting() {
        #expect(Self.phase("", waited: Placeholder.connectingDelay) == .connecting(""))
        #expect(Self.phase("studio", waited: 3) == .connecting("studio"))
        #expect(Self.phase("studio", waited: Placeholder.connectingLimit) == .unreachable("studio", reason: nil))
    }

    /// Once its runner is up, a restore still on its way back shows nothing
    /// rather than "No Workspace Selected" for the moment before it lands;
    /// past the delay, the fleet's own state as before.
    @Test func aReadyRunnersRestoreDoesntFlashNoWorkspaceSelected() {
        #expect(Self.phase(nil, waited: 0.2) == .settling)
        #expect(Self.phase(nil, waited: Placeholder.connectingDelay) == .chooseWorkspace)
        #expect(Self.phase(nil, waited: 0, restoring: false) == .chooseWorkspace)
    }

    /// A runner that has said what's wrong says it at once, delay or not.
    @Test func troubleShowsWithoutTheDelay() {
        let phase = Placeholder.phase(
            hasWorktrees: false, localLoaded: false, localError: "no daemon", hasRepositories: false,
            restoringOn: "", trouble: "no daemon", waited: 0, restoring: true)
        #expect(phase == .unreachable("", reason: "no daemon"))
    }

    /// The placeholder reads its phase at the delay and at the limit, the
    /// two moments it can change, and never at one already gone.
    @Test func theTimelineWakesAtTheDelayAndTheLimit() {
        let since = Date(timeIntervalSinceReferenceDate: 1000)
        #expect(
            Placeholder.wakes(since: since, now: since) == [
                since, since.addingTimeInterval(Placeholder.connectingDelay),
                since.addingTimeInterval(Placeholder.connectingLimit),
            ])
        let later = since.addingTimeInterval(2)
        #expect(Placeholder.wakes(since: since, now: later) == [later, since.addingTimeInterval(Placeholder.connectingLimit)])
    }

    /// A restore's pass waits for the store to change, not for a quarter
    /// second: a send ends a wait long before its limit.
    ///
    /// The send needs the main actor, which the rest of the suite can hold
    /// for over 30 seconds on CI (30.9 s on main's d411e6ee, with one suite
    /// taking 77). So the limit is ten minutes and the bar is half of it:
    /// wide enough that only the limit, never a busy runner, can cross it.
    @MainActor @Test func aChangeEndsTheRestoresWait() async {
        let publisher = ObservableObjectPublisher()
        let started = ContinuousClock.now
        // Runs once the wait below has subscribed and let the main actor go.
        Task { @MainActor in publisher.send() }
        await ChangeWake.next(of: [publisher], orAfter: .seconds(600))
        #expect(ContinuousClock.now - started < .seconds(300))
    }
}
