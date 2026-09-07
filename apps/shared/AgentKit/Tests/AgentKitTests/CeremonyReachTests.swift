import Foundation
import Testing

@testable import AgentKit

/// The rule about `authorized_keys`, on the one code path that writes it.
///
/// The phone could not get this wrong while it held one connection — there was
/// one session and therefore at most one runner to write to. A connection per
/// runner makes the intersection real, and `theLiveRunnerNobodyGrantedIsNotWrittenTo`
/// is the case that costs something if it is: a key on a machine the ceremony
/// said nothing about.
struct CeremonyReachTests {
    /// The whole reason the port touches this flow at all. One connection could
    /// answer with at most one runner however many the manifest granted; this
    /// is the answer that was unreachable before.
    @Test func everyGrantedRunnerThatIsLiveIsWrittenTo() {
        let writable = CeremonyReach.writable(
            granted: ["a", "b", "c"], live: ["a", "b", "c"])
        #expect(writable == ["a", "b", "c"])
    }

    /// **A live connection is not an authorization.** The manifest is. A runner
    /// this phone happens to have a session to, that nobody granted, must not
    /// have a key appended to it.
    @Test func theLiveRunnerNobodyGrantedIsNotWrittenTo() {
        let writable = CeremonyReach.writable(granted: ["a"], live: ["a", "intruder"])
        #expect(writable == ["a"])
    }

    /// Granted and not reachable is ordinary and silent: `confirm()` marks
    /// those pending, which is the truthful thing to say about a file nobody
    /// wrote to.
    @Test func aGrantedRunnerWithNoConnectionIsSimplyAbsent() {
        let writable = CeremonyReach.writable(granted: ["a", "b", "c"], live: ["b"])
        #expect(writable == ["b"])
    }

    /// Nothing live is not an error, and it is the phone's ordinary state for a
    /// manifest full of runners it does not reach.
    @Test func nothingLiveWritesNothing() {
        #expect(CeremonyReach.writable(granted: ["a", "b"], live: []).isEmpty)
    }

    /// Manifest order, so what the screen reports reads in the order somebody
    /// ticked the rows rather than in whatever order a dictionary of
    /// connections enumerates in.
    @Test func theOrderIsTheManifestsAndNotTheConnections() {
        let writable = CeremonyReach.writable(
            granted: ["c", "a", "b"], live: ["a", "b", "c"])
        #expect(writable == ["c", "a", "b"])
    }

    /// One runner is written to once. A runner listed twice is one runner —
    /// `FleetMembership.plan` says the same about a reconcile — and enrolling
    /// twice would be two appends to one file for one tap.
    @Test func aRunnerListedTwiceIsWrittenToOnce() {
        let writable = CeremonyReach.writable(granted: ["a", "a", "b"], live: ["a", "b"])
        #expect(writable == ["a", "b"])
    }
}
