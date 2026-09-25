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

    /// What `Presence.live` reads is this state, so a lock read here is a
    /// person absent there.
    @Test func aLockedScreenIsNobodyPresent() {
        let centers = Centers()
        let watched = Watched(centers)
        let presence = Presence(
            appActive: { true }, screenAwake: { !watched.state.displayAsleep },
            sessionUnlocked: { !watched.state.locked && watched.state.sessionActive },
            secondsSinceInput: { 1 })
        #expect(presence.isPresent)
        centers.distributed.post(name: ScreenState.screenIsLocked, object: nil)
        #expect(!presence.isPresent)
    }
}
