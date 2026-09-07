import Foundation
import Testing

@testable import AgentKit

/// A runner shaped like the app's, carrying only what the rule reads.
private struct Runner: Equatable {
    var id: String
    var standing: StopWaiting.Standing
}

/// What one tap on the app's only pre-fleet button costs.
///
/// Every one of these is a case the port's `for runner in fleet.runners` got
/// wrong, and not one of them could go red: `FleetView` lives in the iOS
/// target, which CI compiles and never runs.
struct StopWaitingTests {
    private func stopping(_ runners: [Runner]) -> [String] {
        StopWaiting.stopping(runners, standing: \.standing).map(\.id)
    }

    private func isOffered(_ runners: [Runner]) -> Bool {
        StopWaiting.isOffered(runners, standing: \.standing)
    }

    // MARK: - Who a tap stops

    /// The case the button was written for: one runner, dialing an address that
    /// routes nowhere, with over a minute of TCP timeout ahead of it.
    @Test func aDialingRunnerIsStopped() {
        #expect(stopping([Runner(id: "a", standing: .dialing)]) == ["a"])
    }

    /// A reconnecting runner is also waiting on the network, and its backoff
    /// can be longer than anybody wants to sit through.
    @Test func aReconnectingRunnerIsStopped() {
        #expect(stopping([Runner(id: "a", standing: .reconnecting)]) == ["a"])
    }

    /// **A fingerprint on screen is not a wait for the network.** Stopping it
    /// throws away the question, and the question is the flow onboarding cannot
    /// afford to lose.
    @Test func aRunnerAskingAboutItsKeyIsNeverStopped() {
        #expect(stopping([Runner(id: "a", standing: .asking)]).isEmpty)
    }

    /// **The state this screen exists for.** `phase` flips to connected a whole
    /// SSH round trip before the first `fleet` call returns, so a runner that
    /// is connected and fleet-less is the ordinary occupant of this screen —
    /// and stopping it kills a session that is working.
    @Test func aConnectedRunnerMidFirstRefreshIsNeverStopped() {
        #expect(stopping([Runner(id: "a", standing: .answering)]).isEmpty)
    }

    /// A runner already on a failure has the only diagnosis there is. Stopping
    /// it replaces "Nothing answered on port 22" with "Stopped waiting", which
    /// is the app forgetting what it knew.
    @Test func aRunnerAlreadyStoppedIsNotStoppedAgain() {
        #expect(stopping([Runner(id: "a", standing: .stopped)]).isEmpty)
    }

    /// **The port's bug, on the fleet that shows it.** One dialing, one holding
    /// a fingerprint, one mid-first-refresh: the old body hit all three.
    @Test func aMixedFleetLosesOnlyTheOneWaitingOnTheNetwork() {
        let fleet = [
            Runner(id: "dialing", standing: .dialing),
            Runner(id: "asking", standing: .asking),
            Runner(id: "answering", standing: .answering),
            Runner(id: "failed", standing: .stopped),
        ]
        #expect(stopping(fleet) == ["dialing"])
    }

    /// Order is the runner list's, because that is the list the rows were drawn
    /// from and a person reading them expects the same order back.
    @Test func theStoppedRunnersKeepTheListsOrder() {
        let fleet = [
            Runner(id: "c", standing: .dialing),
            Runner(id: "a", standing: .reconnecting),
            Runner(id: "b", standing: .asking),
        ]
        #expect(stopping(fleet) == ["c", "a"])
    }

    /// Exactly two of the five standings are a wait on the network. Named as a
    /// whole so a sixth standing added later has to answer the question rather
    /// than fall into a default.
    @Test func exactlyTwoStandingsAreAWaitOnTheNetwork() {
        let waiting = StopWaiting.Standing.allCases.filter(\.isWaitingOnTheNetwork)
        #expect(waiting == [.dialing, .reconnecting])
    }

    // MARK: - Whether the button is there at all

    @Test func theButtonIsOfferedWhenSomethingIsDialing() {
        #expect(isOffered([Runner(id: "a", standing: .dialing)]))
    }

    /// A lone runner holding a fingerprint question. A "Stop Waiting" here
    /// would run, change nothing, and tell somebody the app is stuck.
    @Test func theButtonIsNotOfferedForAFingerprintQuestionAlone() {
        #expect(!isOffered([Runner(id: "a", standing: .asking)]))
    }

    /// Nothing to stop and nothing to draw.
    @Test func theButtonIsNotOfferedWhenEveryRunnerHasSettled() {
        let fleet = [
            Runner(id: "a", standing: .stopped),
            Runner(id: "b", standing: .answering),
            Runner(id: "c", standing: .asking),
        ]
        #expect(!isOffered(fleet))
    }

    /// No runners at all — the phone with an empty runner list. The screen has
    /// no rows and needs no button.
    @Test func theButtonIsNotOfferedWithNoRunners() {
        #expect(!isOffered([]))
        #expect(stopping([]).isEmpty)
    }

    /// The two answers never disagree: the button is offered exactly when a tap
    /// would do something. They are separate functions because two screens read
    /// them at different moments, and this is what keeps them one rule.
    @Test func theButtonIsOfferedExactlyWhenATapWouldDoSomething() {
        for standing in StopWaiting.Standing.allCases {
            let fleet = [Runner(id: "a", standing: standing)]
            #expect(isOffered(fleet) == !stopping(fleet).isEmpty, "\(standing)")
        }
    }
}
