import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A relaunch keeps its window's place while that window's runner connects,
/// and says it's connecting (ov-279).
struct RestoreConnectingTests {
    typealias World = DestinationResolver.World

    static let kept = DestinationOpen(
        destination: Destination(runner: .init(host: "studio"), place: .workspace("ws-1")), arrival: .restore,
        since: Date())

    @Test func aRestoreWaitsOnlyForAConfiguredRunnerThatIsntUp() {
        #expect(DestinationOpener.waitsForRunner(Self.kept, in: World(seats: [World.Seat(host: "studio", ready: false)])))
        #expect(!DestinationOpener.waitsForRunner(Self.kept, in: World(seats: [World.Seat(host: "studio", ready: true)])))
        // Gone from the configuration: nothing to wait for.
        #expect(!DestinationOpener.waitsForRunner(Self.kept, in: World(seats: [World.Seat(host: "other", ready: false)])))
        var click = Self.kept
        click.arrival = .notification
        #expect(!DestinationOpener.waitsForRunner(click, in: World(seats: [World.Seat(host: "studio", ready: false)])))
    }

    @Test func theWindowSaysItsConnectingWhateverTheFleetHolds() {
        let phase = FleetPlaceholder.phase(
            hasWorktrees: true, localLoaded: true, localError: nil, hasRepositories: true, restoringOn: "studio")
        #expect(phase == .connecting("studio"))
        #expect(
            FleetPlaceholder.phase(hasWorktrees: true, localLoaded: true, localError: nil, hasRepositories: true)
                == .chooseWorkspace)
    }

    @Test func theConnectingWordsNameTheRunner() {
        #expect(FleetPlaceholder.connectingTitle("studio") == "Connecting to studio…")
        #expect(FleetPlaceholder.connectingTitle("") == "Connecting to this Mac’s runner…")
    }
}
