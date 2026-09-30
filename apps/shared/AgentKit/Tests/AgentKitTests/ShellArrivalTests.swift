import Foundation
import Testing

@testable import AgentKit

/// Which runners keep a worktree order a drag can write to.
///
/// The rule lives on the build (`DaemonBuild.keepsWorktreeOrder`) so every app
/// asks the one question in the one spelling; the Mac's sidebar offers the drag
/// on it. The two cases that matter are the ones that look alike on the wire.
struct WorktreeOrderCapabilityTests {
    /// Two worktrees as `Session::fleet` puts them on the wire for a runner
    /// that predates `worktree_order`.
    ///
    /// **`ordinal` is PRESENT, and 0.** It is a proto3 scalar with no
    /// presence, prost decodes an old daemon's silence as 0, and
    /// `crates/client/src/session.rs` emits `"ordinal": w.ordinal`
    /// unconditionally — so "no ordinal" is not a thing a client ever sees,
    /// and a rule that waited for one would offer every old runner a drag.
    private static func wire(ordinals: [Int]) throws -> [Worktree] {
        let rows = ordinals.enumerated().map { index, ordinal in
            """
            {"id": "w\(index)", "short": "w\(index)", "task": "t\(index)", "branch": "b\(index)",
             "state": "ready", "ordinal": \(ordinal), "terminals": []}
            """
        }
        return try JSONDecoder().decode(
            [Worktree].self, from: Data("[\(rows.joined(separator: ","))]".utf8))
    }

    /// **An old runner, all zeros and no `worktree_order`, is not offered a
    /// drag.** It would accept nothing — `worktree.reorder` is unknown to it —
    /// and the row would spring back with no error anywhere.
    @Test func aRunnerWithoutTheWorktreeOrderCapabilityKeepsNoOrder() throws {
        let old = DaemonBuild(
            version: "0.1.0+old", matches: true, platform: "macos",
            capabilities: ["workspaces", "terminals", "watching"])
        let worktrees = try Self.wire(ordinals: [0, 0])
        #expect(
            worktrees.allSatisfy { $0.ordinal == 0 },
            "an old runner's ordinals arrive, as 0 — there is no absence to read")
        #expect(!old.keepsWorktreeOrder)
    }

    /// The capability is the whole answer, and a runner nobody has asked yet
    /// is refused until it has been asked rather than offered a drag on a
    /// guess.
    @Test func theWorktreeOrderCapabilityIsWhatDecides() {
        let new = DaemonBuild(
            version: "0.1.0+new", matches: true, platform: "macos",
            capabilities: ["workspaces", "terminals", "workspace_order"])
        #expect(new.keepsWorktreeOrder)
        // A daemon so old it answered no capabilities at all is read as the
        // two features that existed then — which does not include this.
        let ancient = DaemonBuild(version: "0.0.1", matches: true, platform: "macos")
        #expect(!ancient.keepsWorktreeOrder)
    }
}

/// Which rests move the selected runner.
///
/// The selection is persisted, so a rest that follows the wrong thing is a
/// regression that outlives the process: one launch where the selected runner
/// was slow to answer seats the shell on another runner's first worktree, and
/// a rule that followed that landing would write it down as the choice.
struct ShellSelectionTests {
    private func follows(_ arrival: ShellArrival, every: Bool = true) -> Bool {
        ShellSelection.follows(arrival, everyRunnerAtOnce: every, arrived: "B", selected: "A")
    }

    /// The launch landing is a race, not a choice.
    @Test func theLandingAtLaunchIsNotFollowed() {
        #expect(!follows(.appeared))
    }

    /// A worktree vanishing, or a runner answering, re-seats the shell;
    /// nobody chose where.
    @Test func aReseatIsNotFollowed() {
        #expect(!follows(.reseated))
    }

    /// A notification tapped is not a person choosing a runner to work on.
    @Test func aDeepLinkIsNotFollowed() {
        #expect(!follows(.linked))
    }

    /// A swipe onto another runner's worktree is.
    @Test func aMoveOntoAnotherRunnerIsFollowed() {
        #expect(follows(.moved))
        #expect(
            !ShellSelection.follows(
                .moved, everyRunnerAtOnce: true, arrived: "A", selected: "A"),
            "already selected: nothing to write")
    }

    /// With one runner connected at a time, the selection IS which runner is
    /// connected, and a rest never changes it.
    @Test func withOneRunnerAtATimeNothingIsFollowed() {
        #expect(!follows(.moved, every: false))
    }
}

/// **A connected runner answers only once this link has read its fleet**
/// (ov-22 M3). `start` and `reconnect` set `.connected` and then await the
/// fleet, so for a round trip every card, tab and "Working…" was the last
/// link's fleet said as live. Android's `RunnerLink.given`, and the same rule.
///
/// Mutation: `answering` reading `connected` alone. Red.
@Test func aConnectedRunnerAnswersOnlyOnceItsLinkHasReadTheFleet() {
    #expect(!RunnerLink.answering(connected: true, fleetReadOnThisLink: false))
    #expect(RunnerLink.answering(connected: true, fleetReadOnThisLink: true))
    #expect(!RunnerLink.answering(connected: false, fleetReadOnThisLink: true))
}
