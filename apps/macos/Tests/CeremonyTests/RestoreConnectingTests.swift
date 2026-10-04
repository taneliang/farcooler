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
        #expect(FleetPlaceholder.connectingTitle("e-liang@studio") == "Connecting to studio…")
        #expect(FleetPlaceholder.connectingTitle("") == "Connecting to This Mac…")
        #expect(FleetPlaceholder.unreachableTitle("") == "Can’t Reach This Mac")
    }

    /// "Connecting…" ends (review H2): a runner that said what's wrong, this
    /// Mac's daemon's error included, shows it with Try Again at once, and one
    /// that hasn't answered within ssh's ten seconds reads as unreachable.
    @Test func connectingEndsInTheRunnersTroubleOrAfterTheConnectTimeout() {
        func phase(trouble: String? = nil, waited: TimeInterval = 0) -> FleetPlaceholder.Phase {
            FleetPlaceholder.phase(
                hasWorktrees: false, localLoaded: false, localError: trouble, hasRepositories: false,
                restoringOn: "", trouble: trouble, waited: waited)
        }
        #expect(phase(waited: 9) == .connecting(""))
        #expect(phase(trouble: "the daemon did not start") == .unreachable("", reason: "the daemon did not start"))
        #expect(phase(waited: FleetPlaceholder.connectingLimit) == .unreachable("", reason: nil))
    }
}
