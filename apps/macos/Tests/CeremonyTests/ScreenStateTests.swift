import AppKit
import Foundation
import Testing

@testable import Far_Cooler

/// `ScreenState` keeps what no AppKit property answers — display asleep,
/// screen locked, session switched away — from the notifications that say
/// each one changed, and says `personLeft` whenever somebody went.
///
/// Posted on centers of the test's own, so no display sleeps and no session
/// locks. What this can't show is that macOS still posts these names; that
/// is the live check, and it is not here.
@MainActor
struct ScreenStateTests {
    private struct Centers {
        let workspace = NotificationCenter()
        let distributed = NotificationCenter()
        let announce = NotificationCenter()
    }

    /// A `ScreenState` on `centers`, and how many times it has said
    /// `personLeft`.
    @MainActor
    private final class Watched {
        let state: ScreenState
        var left = 0
        private var token: NSObjectProtocol?

        init(_ centers: Centers) {
            state = ScreenState(
                workspace: centers.workspace, distributed: centers.distributed,
                announce: centers.announce)
            token = centers.announce.addObserver(
                forName: ScreenState.personLeft, object: nil, queue: nil
            ) { [weak self] _ in MainActor.assumeIsolated { self?.left += 1 } }
        }
    }

    @Test func theDisplaySleepingIsSomebodyLeavingAndWakingIsNot() {
        let centers = Centers()
        let watched = Watched(centers)
        #expect(!watched.state.displayAsleep)

        centers.workspace.post(name: NSWorkspace.screensDidSleepNotification, object: nil)
        #expect(watched.state.displayAsleep)
        #expect(watched.left == 1)

        centers.workspace.post(name: NSWorkspace.screensDidWakeNotification, object: nil)
        #expect(!watched.state.displayAsleep)
        #expect(watched.left == 1, "coming back is not leaving")
    }

    @Test func theScreenLockingIsSomebodyLeaving() {
        let centers = Centers()
        let watched = Watched(centers)

        centers.distributed.post(name: ScreenState.screenIsLocked, object: nil)
        #expect(watched.state.locked)
        #expect(watched.left == 1)

        centers.distributed.post(name: ScreenState.screenIsUnlocked, object: nil)
        #expect(!watched.state.locked)
        #expect(watched.left == 1)
    }

    @Test func switchingToAnotherUserIsSomebodyLeaving() {
        let centers = Centers()
        let watched = Watched(centers)

        centers.workspace.post(name: NSWorkspace.sessionDidResignActiveNotification, object: nil)
        #expect(!watched.state.sessionActive)
        #expect(watched.left == 1)

        centers.workspace.post(name: NSWorkspace.sessionDidBecomeActiveNotification, object: nil)
        #expect(watched.state.sessionActive)
        #expect(watched.left == 1)
    }

    /// `Presence.live` reads its display and session from `ScreenState`:
    /// each way a person leaves, posted, turns the live closure that should
    /// see it false, and coming back turns it true again. The app-active and
    /// idle closures are this Mac's own and aren't asked here.
    @Test func theLivePresenceReadsTheScreenState() {
        let centers = Centers()
        let watched = Watched(centers)
        let live = Presence.live(watched.state)
        #expect(live.screenAwake() && live.sessionUnlocked())

        centers.workspace.post(name: NSWorkspace.screensDidSleepNotification, object: nil)
        #expect(!live.screenAwake())
        centers.workspace.post(name: NSWorkspace.screensDidWakeNotification, object: nil)
        #expect(live.screenAwake())

        centers.distributed.post(name: ScreenState.screenIsLocked, object: nil)
        #expect(!live.sessionUnlocked(), "locked")
        centers.distributed.post(name: ScreenState.screenIsUnlocked, object: nil)
        #expect(live.sessionUnlocked())

        centers.workspace.post(name: NSWorkspace.sessionDidResignActiveNotification, object: nil)
        #expect(!live.sessionUnlocked(), "switched away")
        centers.workspace.post(name: NSWorkspace.sessionDidBecomeActiveNotification, object: nil)
        #expect(live.sessionUnlocked())
    }
}
