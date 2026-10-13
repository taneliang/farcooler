import Foundation
import Testing

@testable import AgentKit

/// The ladder a train climbs, said compactly for the phones (ov-503, design 7).
struct PlanLadderTests {
    @Test("a phase on the ladder says its word and its step of five")
    func compactOnTheLadder() {
        #expect(PlanLadder.compact(.waitingForCi) == "Waiting for CI \u{00B7} step 4 of 5")
        #expect(PlanLadder.compact(.waitingForLanes) == "Waiting for Lanes \u{00B7} step 1 of 5")
        #expect(PlanLadder.compact(.gatingLocally) == "Gating Locally \u{00B7} step 3 of 5")
        // CI failing is still the fourth rung; stalled keeps the second.
        #expect(PlanLadder.compact(.ciFailed) == "CI Failed \u{00B7} step 4 of 5")
        #expect(PlanLadder.compact(.stalled) == "Stalled \u{00B7} step 2 of 5")
    }

    @Test("a train past the ladder says the phase alone")
    func compactPastTheLadder() {
        #expect(PlanLadder.compact(.landed) == "Landed")
        #expect(PlanLadder.compact(.dropped) == "Dropped")
    }

    @Test("every phase has a step the ladder holds or sits past it")
    func everyPhaseHasAStep() {
        for phase in TrainPhase.allCases {
            #expect((0...PlanLadder.steps.count).contains(PlanLadder.step(phase)), "\(phase)")
        }
    }
}
