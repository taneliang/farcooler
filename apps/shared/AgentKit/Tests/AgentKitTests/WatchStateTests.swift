import Foundation
import Testing

@testable import AgentKit

/// The spec's rule about the watch, which nothing else can check.
///
/// "The three reachability states are distinguishable, and actions are disabled
/// in two of them" is a verification bullet with no natural home: the code that
/// obeys it draws screens, and screens are in a watchOS app target `swift test`
/// cannot build. So the decision itself was moved to `WatchState.resolve` and
/// this is what stands behind it.
///
/// The tests that matter most here are the two about AGE. They look redundant —
/// `resolve` cannot see a date, so of course it ignores one — and that is
/// exactly what they are defending. The tempting change to this file is a
/// staleness parameter, on the reasoning that a fresh snapshot is good enough
/// to act on. It is not: a fresh snapshot with an unreachable phone still means
/// the Allow button goes nowhere, and the person walks away believing they
/// answered.
struct WatchStateTests {
    private func snapshot(
        agents: [FleetSnapshot.Agent] = [], capturedAt: Date = Date()
    ) -> FleetSnapshot {
        FleetSnapshot(agents: agents, capturedAt: capturedAt, complete: true)
    }

    private var blockedAgent: FleetSnapshot.Agent {
        FleetSnapshot.Agent(
            id: "t1", label: "Terminal 1", machine: "studio", status: "blocked",
            glyph: "?", headline: "Wants to run cargo test", line: "cargo test --workspace",
            feed: [], rank: 0, turnFailed: false, activityChangedAt: nil)
    }

    // MARK: - The three states are distinguishable

    @Test func aReachablePhoneWithASnapshotIsLive() {
        let fleet = snapshot(agents: [blockedAgent])
        #expect(WatchState.resolve(snapshot: fleet, reachable: true) == .live(fleet))
    }

    @Test func anUnreachablePhoneWithASnapshotIsCached() {
        let fleet = snapshot(agents: [blockedAgent])
        #expect(WatchState.resolve(snapshot: fleet, reachable: false) == .cached(fleet))
    }

    @Test func noSnapshotIsNothingWhicheverWayTheLinkIs() {
        #expect(WatchState.resolve(snapshot: nil, reachable: true) == .nothing)
        #expect(WatchState.resolve(snapshot: nil, reachable: false) == .nothing)
    }

    /// Reachable and empty-handed is still `nothing`, not a live empty fleet.
    ///
    /// The two read the same on a screen and are not the same thing: one is "no
    /// agents are running" and the other is "this watch has never been told".
    /// Only a snapshot can say the first.
    @Test func aReachablePhoneThatHasSaidNothingYetIsStillNothing() {
        #expect(WatchState.resolve(snapshot: nil, reachable: true).snapshot == nil)
    }

    // MARK: - Actions are disabled in two of the three

    @Test func onlyLiveMayAct() {
        let fleet = snapshot(agents: [blockedAgent])
        #expect(WatchState.live(fleet).canAct)
        #expect(!WatchState.cached(fleet).canAct)
        #expect(!WatchState.nothing.canAct)
    }

    // MARK: - Age decides nothing, in either direction

    @Test func aSnapshotFromASecondAgoIsStillCachedWhenThePhoneIsUnreachable() {
        let fresh = snapshot(agents: [blockedAgent], capturedAt: Date())
        let state = WatchState.resolve(snapshot: fresh, reachable: false)
        #expect(state == .cached(fresh))
        #expect(!state.canAct)
    }

    @Test func aSnapshotFromHoursAgoIsStillLiveWhenThePhoneIsReachable() {
        // Past `FleetSnapshot.staleAfter`, so every surface will render it with
        // reduced confidence — and it is still actionable, because the phone is
        // right there to carry the tap.
        let old = snapshot(
            agents: [blockedAgent],
            capturedAt: Date().addingTimeInterval(-FleetSnapshot.staleAfter * 3))
        let state = WatchState.resolve(snapshot: old, reachable: true)
        #expect(state == .live(old))
        #expect(state.canAct)
    }

    // MARK: - What the screens read

    @Test func bothStatesThatHoldAFleetHandItBack() {
        let fleet = snapshot(agents: [blockedAgent])
        #expect(WatchState.live(fleet).snapshot == fleet)
        #expect(WatchState.cached(fleet).snapshot == fleet)
        #expect(WatchState.nothing.snapshot == nil)
    }
}
