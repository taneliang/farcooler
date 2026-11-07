import Foundation
import Testing

@testable import Far_Cooler

/// An update waits while the runner is mid-upgrade (ov-963 M3).
struct UpdateHoldTests {
    @Test func aHeldRunnerMakesTheUpdateWaitWithAPlainReason() {
        #expect(throws: UpdateHold.Held.self) { try UpdateHold.check(cached: true, probe: { false }) }
        let held = UpdateHold.Held()
        #expect(held.localizedDescription == UpdateHold.sentence)
        #expect(UpdateHold.sentence.contains("mid-upgrade"))
        #expect(UpdateHold.sentence.contains("Finish the upgrade first"))
    }

    @Test func aRunnerThatIsNotHeldLetsItGo() throws {
        try UpdateHold.check(cached: false, probe: { Issue.record("asked again"); return true })
    }

    @Test func noAnswerYetAsksOnceRatherThanGuessing() {
        #expect(throws: UpdateHold.Held.self) { try UpdateHold.check(cached: nil, probe: { true }) }
        #expect(throws: Never.self) { try UpdateHold.check(cached: nil, probe: { false }) }
    }

    @Test func onlyAnExplicitYesHoldsAnUpdate() {
        #expect(UpdateHold.held(inOutput: "{\"held\":true}\n"))
        #expect(!UpdateHold.held(inOutput: "{\"held\":false}"))
        #expect(!UpdateHold.held(inOutput: ""))
        #expect(!UpdateHold.held(inOutput: "not json"))
        #expect(!UpdateHold.held(inOutput: "{\"held\":\"true\"}"))
    }
}
