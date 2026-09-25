import Foundation
import Testing

@testable import Far_Cooler

/// What the status bar says about the fleet's panes, from each runner's state
/// and the last fleet it read.
///
/// Only a connected runner's count is a reading. A runner that is down spends
/// most of its outage in `.reconnecting` between attempts, and before this
/// its last count came back as "live" for every one of those waits.
@MainActor
struct FleetReadingTests {
    private static func fleet(healthy: Bool = true, live: Int, rows: [String] = []) -> Fleet {
        Fleet(
            runtimeHealthy: healthy, livePanes: live,
            workspaces: rows.map {
                Workspace(
                    id: $0, short: $0, task: $0, branch: "feat/\($0)", repository: "overnight",
                    host: "", worktree: "/tmp/\($0)", state: "active", terminals: [])
            })
    }

    /// Every state that isn't `.connected` — the refusals and the two that
    /// only mean "not known yet".
    private static let notConnected: [HostState] = [
        .connecting, .reconnecting(attempt: 1), .unreachable(reason: "gone"), .notInstalled,
    ]

    /// A second runner that isn't connected adds nothing: not its count, and
    /// not its stale health.
    @Test func onlyConnectedRunnersAreCounted() {
        for state in Self.notConnected {
            let reading = FleetStore.reading(of: [
                ("", .connected, Self.fleet(live: 2)), ("", state, Self.fleet(live: 5)),
            ])
            #expect(reading == .live(2), "\(state)")
        }
    }

    /// The runner that answers has no tmux; the one that doesn't had it when
    /// it went quiet. The bar says what the answering one says.
    @Test func aStaleHealthyRunnerDoesNotSpeakForTheFleet() {
        for state in Self.notConnected {
            let reading = FleetStore.reading(of: [
                ("", .connected, Self.fleet(healthy: false, live: 0)),
                ("", state, Self.fleet(healthy: true, live: 5)),
            ])
            #expect(reading == .runtimeDown, "\(state)")
        }
    }

    /// A fleet of one between reconnection attempts: neither its last count
    /// nor "tmux unavailable" in red. It says it isn't connected, and stays
    /// saying that through the whole retry cycle — `.unreachable` after a
    /// failed read and `.reconnecting` during the wait read the same.
    @Test func aLoneRunnerThatIsDownCantSay() {
        for state: HostState in [.reconnecting(attempt: 3), .unreachable(reason: "gone")] {
            let reading = FleetStore.reading(of: [("", state, Self.fleet(live: 4))])
            #expect(reading == .unsaid, "\(state)")
            #expect(!reading.isTrouble, "\(state)")
            #expect(reading.sentence == "Not connected", "\(state)")
        }
    }

    /// Before any runner has answered, the bar says it's connecting, not that
    /// anything is broken.
    @Test func nothingAnsweredYetIsConnecting() {
        let reading = FleetStore.reading(of: [("", .connecting, .empty), ("", .connecting, .empty)])
        #expect(reading == .connecting)
        #expect(!reading.isTrouble)
        // One runner lost among ones still connecting: not "Connecting…".
        #expect(
            FleetStore.reading(of: [("", .connecting, .empty), ("", .reconnecting(attempt: 1), .empty)])
                == .unsaid)
    }

    /// Connected runners say what they say: a count, or red when none of
    /// them has tmux.
    @Test func connectedRunnersSpeak() {
        let live = FleetStore.reading(of: [
            ("", .connected, Self.fleet(live: 2)), ("", .connected, Self.fleet(healthy: false, live: 1)),
        ])
        #expect(live == .live(3))
        #expect(live.sentence == "3 live")
        let down = FleetStore.reading(of: [("", .connected, Self.fleet(healthy: false, live: 0))])
        #expect(down == .runtimeDown)
        #expect(down.isTrouble)
        #expect(down.sentence == "tmux unavailable")
    }

    /// The merged `Fleet` carries the same numbers, so nothing that reads
    /// `store.fleet` instead of `store.reading` gets the stale ones — while
    /// the rows of a runner that went quiet stay, in order, so the sidebar
    /// doesn't move.
    @Test func theMergedFleetCountsOnlyConnectedRunnersButKeepsEveryRow() {
        let merged = FleetStore.merge([
            ("", .connected, Self.fleet(healthy: false, live: 1, rows: ["here"])),
            ("", .reconnecting(attempt: 2), Self.fleet(healthy: true, live: 5, rows: ["there"])),
        ])
        #expect(merged.reading == .runtimeDown)
        #expect(merged.fleet.runtimeHealthy == false)
        #expect(merged.fleet.livePanes == 0)
        #expect(merged.fleet.workspaces.map(\.id) == ["here", "there"])

        let live = FleetStore.merge([
            ("", .connected, Self.fleet(live: 1, rows: ["here"])),
            ("", .reconnecting(attempt: 2), Self.fleet(live: 5, rows: ["there"])),
        ])
        #expect(live.fleet.runtimeHealthy == true)
        #expect(live.fleet.livePanes == 1)
    }

    /// A runner that answered and has no Far Cooler says so, by name — not
    /// the generic "Not connected", which would hide the one reason here
    /// there is something to do about. Neutral, like its trouble dot.
    @Test func aRunnerWithoutFarCoolerIsNamed() {
        let remote = FleetStore.reading(of: [("gpu-box", .notInstalled, .empty)])
        #expect(remote == .notInstalled(["gpu-box"]))
        #expect(remote.sentence == "Far Cooler isn’t installed on gpu-box")
        #expect(remote.sentence != FleetStore.Reading.unsaid.sentence)
        #expect(!remote.isTrouble)

        #expect(
            FleetStore.reading(of: [("", .notInstalled, .empty)]).sentence
                == "Far Cooler isn’t installed on this Mac")
        // One still connecting doesn't hide it; it has nothing to say yet.
        #expect(
            FleetStore.reading(of: [("", .connecting, .empty), ("gpu-box", .notInstalled, .empty)])
                == .notInstalled(["gpu-box"]))
        #expect(
            FleetStore.reading(of: [("a", .notInstalled, .empty), ("b", .notInstalled, .empty)])
                .sentence == "Far Cooler isn’t installed on 2 runners")
        // One lost among them: that one might come back with it. Generic.
        #expect(
            FleetStore.reading(of: [
                ("a", .notInstalled, .empty), ("b", .reconnecting(attempt: 1), .empty),
            ]) == .unsaid)
    }
}
