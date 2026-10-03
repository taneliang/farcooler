import Combine
import Foundation
import Testing

@testable import Far_Cooler

/// A runner whose news changes nothing must not redraw the window (ov-229).
///
/// `DaemonClient` is `ObservableObject`, so every `@Published` assignment fires
/// `objectWillChange` whether or not the value moved, and `FleetStore` turns
/// each one into a whole-window update. A busy fleet sends an event per agent
/// per second, and a full read after each fleet change. These pin that an
/// event or a read that changes nothing publishes nothing, and that one that
/// changes something publishes once.
@MainActor
struct IdlePublishTests {
    private static let fleetJSON = """
        {"live_panes":1,"runtime_healthy":true,"worktrees":[{"branch":"lane-1","host":"",
        "id":"w-1","short":"w1","repository":"demo","state":"active","task":"lane 1",
        "worktree":"/tmp/lane-1","terminals":[{"id":"t-1","short":"t1","title":"shell",
        "preset":"claude","state":"running","activity":"working","activitySince":1000,
        "turnStartedAt":1000,"line":"reading","feed":[],"subagents":[],"turnFailed":false,
        "chatCapable":true,"epoch":0,"exitCode":null,"exitSignal":null,"blockedQuestion":null}]}]}
        """

    private static func event(line: String) throws -> TerminalEvent {
        let json = """
            {"id":"t-1","short":"t1","worktree":"w-1","title":"shell","preset":"claude",
            "state":"running","activity":"working","activitySince":1000,"turnStartedAt":1000,
            "line":"\(line)","feed":[],"subagents":[],"turnFailed":false,"chatCapable":true}
            """
        return try JSONDecoder().decode(TerminalEvent.self, from: Data(json.utf8))
    }

    /// A client that has read the fleet once and let everything that read
    /// started settle, and a count of what it publishes from then on.
    private static func settledClient() async -> (DaemonClient, Counter) {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            if args.prefix(2) == ["worktree", "list"] { return (Data(fleetJSON.utf8), nil) }
            return (nil, "not in this test")
        }
        await client.refresh()
        try? await Task.sleep(for: .milliseconds(200))
        let counter = Counter()
        counter.cancellable = client.objectWillChange.sink { _ in counter.count += 1 }
        return (client, counter)
    }

    final class Counter {
        var count = 0
        var cancellable: AnyCancellable?
    }

    @Test func theFixtureDecodes() throws {
        _ = try JSONDecoder().decode(Fleet.self, from: Data(Self.fleetJSON.utf8))
    }

    @Test func anEventThatChangesNothingPublishesNothing() async throws {
        let (client, counter) = await Self.settledClient()
        #expect(client.fleet.worktrees.first?.terminals.first?.line == "reading")

        client.apply(try Self.event(line: "reading"))
        #expect(counter.count == 0, "an unchanged event published \(counter.count) times")
    }

    @Test func anEventThatChangesSomethingPublishesOnce() async throws {
        let (client, counter) = await Self.settledClient()

        client.apply(try Self.event(line: "editing"))
        #expect(client.fleet.worktrees.first?.terminals.first?.line == "editing")
        #expect(counter.count == 1, "one changed event published \(counter.count) times")
    }

    @Test func aReadThatChangesNothingPublishesNothing() async throws {
        let (client, counter) = await Self.settledClient()

        await client.refresh()
        try? await Task.sleep(for: .milliseconds(200))
        #expect(counter.count == 0, "an unchanged read published \(counter.count) times")
    }
}
