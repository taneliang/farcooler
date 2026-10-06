import Foundation
import Testing

@testable import AgentKit

/// The plan strip on the phones (ov-300): the orchestrator's state as a mark,
/// what needs you, what's moving and what's next, in the Mac's strip's words.
/// Plans go through `PlanModel.decode`, the parser the apps read with.
struct PlanStripTests {
    typealias P = PlanModelTests

    static func plan() throws -> PlanModel {
        try P.plan(
            themes: [],
            lanes: [
                P.lane("mac-vis", "building", cards: ["t1"]),
                P.lane("phones-b", "review", cards: ["t2"]),
                P.lane("mac-rel", "fixing", cards: ["t3"]),
                P.lane("plan-phones", "queued", cards: ["t4"], rank: 1),
                P.lane("vis-lint", "queued", cards: ["t5"], rank: 2),
                P.lane("land-129", "landed", cards: ["t6"]),
            ],
            order: ["plan-phones", "vis-lint"])
    }

    @Test("it says what needs you, two lanes in Now, how many more, and the next lane")
    func wordsInOrder() throws {
        let strip = PlanStrip(plan: try Self.plan(), needsYou: 2, orchestrator: .working)
        #expect(strip.parts == ["2 need you", "mac-vis Building", "phones-b In Review", "+1", "next: plan-phones"])
        #expect(strip.text == "2 need you · mac-vis Building · phones-b In Review · +1 · next: plan-phones")
    }

    @Test("one ask is singular, and none says nothing rather than zero")
    func needsYouWords() throws {
        let plan = try Self.plan()
        #expect(PlanStrip(plan: plan, needsYou: 1, orchestrator: .idle).parts.first == "1 needs you")
        #expect(PlanStrip(plan: plan, needsYou: 0, orchestrator: .idle).parts.first == "mac-vis Building")
    }

    @Test("with no plan, no asks and no lanes there's nothing to draw")
    func emptyIsNotDrawn() throws {
        let strip = PlanStrip(plan: .empty, needsYou: 0, orchestrator: .working)
        #expect(strip.isEmpty)
        #expect(!PlanStrip(plan: .empty, needsYou: 1, orchestrator: .working).isEmpty)
    }

    @Test("a queued-only plan says only what's next")
    func queuedOnly() throws {
        let plan = try P.plan(themes: [], lanes: [P.lane("a", "queued", cards: ["t1"], rank: 1)], order: ["a"])
        #expect(PlanStrip(plan: plan, needsYou: 0, orchestrator: .idle).parts == ["next: a"])
    }

    @Test("VoiceOver hears the orchestrator's state first, since the mark draws it")
    func accessibilityLabel() throws {
        let strip = PlanStrip(plan: try Self.plan(), needsYou: 2, orchestrator: .needsYou)
        #expect(strip.accessibilityLabel.hasPrefix("Orchestrator needs you, 2 need you, mac-vis Building"))
        #expect(PlanStrip(plan: .empty, needsYou: 1, orchestrator: .none).accessibilityLabel == "No orchestrator, 1 needs you")
    }

    @Test("color only for a state that wants the owner")
    func tones() {
        #expect(PlanStripOrchestrator.needsYou.tone == .attention)
        #expect(PlanStripOrchestrator.failed.tone == .failure)
        #expect(PlanStripOrchestrator.stopped.tone == .failure)
        for quiet: PlanStripOrchestrator in [.none, .starting, .working, .done, .idle] { #expect(quiet.tone == .quiet) }
    }

    @Test("the line is kept for the peek, never part of the strip's own words")
    func lineIsNotAPart() throws {
        let strip = PlanStrip(plan: try Self.plan(), needsYou: 0, orchestrator: .working, line: "Dispatching ov-321")
        #expect(strip.line == "Dispatching ov-321")
        #expect(!strip.text.contains("Dispatching"))
    }
}
