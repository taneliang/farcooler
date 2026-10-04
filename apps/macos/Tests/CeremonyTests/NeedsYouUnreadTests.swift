import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A runner whose Needs You list could not be read is not a runner with
/// nothing waiting (ov-159).
///
/// `refreshNeedsYou` marked the runner known after a failed read and
/// `needsYouItems` derived a list only for runners without `needs_you`, so a
/// serving runner whose read failed contributed nothing, and the page said
/// "Nothing Needs You" with no caveat. The phones show the blocked agents the
/// fleet already carries and name the runner that isn't answering
/// (`PhoneInbox`); the Mac now asks the same two functions.
@MainActor
struct NeedsYouUnreadTests {
    private static let fleetJSON = #"""
        {"runtime_healthy":true,"live_panes":1,"worktrees":[{"id":"w-159","short":"w","task":"lane",
          "branch":"b","worktree":"/tmp/w","state":"active",
          "terminals":[{"id":"t-159","short":"t1","title":"claude","preset":"claude","state":"running",
            "activity":"blocked","blockedQuestion":"Allow touch x?","activitySince":1000,"epoch":0}]}]}
        """#

    /// A connected client whose fleet holds one blocked agent, and whose
    /// `needs-you` read is whatever `reading` says.
    private func client(
        reading: @escaping @MainActor () -> (data: Data?, message: String?)
    ) async -> DaemonClient {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            if args.prefix(2) == ["worktree", "list"] { return (Data(Self.fleetJSON.utf8), nil) }
            if args.first == "needs-you" { return reading() }
            return (nil, "not in this test")
        }
        await client.refresh()
        client.daemonBuild = DaemonBuild(
            version: "v", matches: true, platform: "linux", capabilities: ["needs_you"])
        return client
    }

    @Test func aFailedReadShowsTheDerivedItemsAndNamesTheRunner() async throws {
        let client = await client { (nil, "ssh: connection timed out") }
        await client.refreshNeedsYou()

        let shown = FleetStore.shownNeedsYou(["": client])
        #expect(shown.count == 1, "the blocked agent is shown, not 'Nothing Needs You'")
        #expect(shown.first?.terminal?.id == "t-159")
        #expect(client.needsYouUnanswered)
        // Through the view's own wiring, not the phone's helper alone: the
        // name opens the sentence, so it is capitalized, and it is the runner.
        let names = FleetStore.unansweredNames([""], ["": client])
        #expect(names == ["This Mac’s runner"])
        #expect(
            PhoneInbox.caveat(unanswered: names)
                == "This Mac’s runner isn’t answering, so this may not be everything.")
    }

    @Test func aReadListReplacesTheDerivedItemsAndDropsTheCaveat() async throws {
        let client = await client { (Data(#"{"items":[]}"#.utf8), nil) }
        await client.refreshNeedsYou()

        #expect(FleetStore.shownNeedsYou(["": client]).isEmpty, "a runner that answered nothing waiting says so")
        #expect(!client.needsYouUnanswered)
    }

    @Test func aRunnerThatCannotBeReachedIsNamedEvenWithAListFromBefore() async throws {
        let client = await client { (Data(#"{"items":[]}"#.utf8), nil) }
        await client.refreshNeedsYou()
        #expect(!client.needsYouUnanswered)
        // The runner goes away: its list from before stays, and it is named.
        client.commandRunnerForTesting = { _ in (nil, "ssh: timed out") }
        await client.refresh()
        #expect(client.needsYouUnanswered)
    }
}
