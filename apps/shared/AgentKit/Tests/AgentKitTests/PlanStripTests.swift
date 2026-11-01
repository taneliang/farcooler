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
            lines: [
                P.line("t1", "building", title: "Visual polish"),
                P.line("t2", "reviewing", title: "Phone strip"),
                P.line("t3", "building", fixing: true, title: "Release notes"),
                P.line("t4", "waiting", rank: 1, title: "Plan phones"),
                P.line("t5", "waiting", rank: 2, title: "Lint rules"),
                P.line("t6", "landed", title: "Landing report"),
            ])
    }

    @Test("it says what needs you, two tickets in Now, how many more, and the next ticket")
    func wordsInOrder() throws {
        let strip = PlanStrip(plan: try Self.plan(), needsYou: 2, orchestrator: .working)
        #expect(strip.parts == ["2 need you", "Visual polish Building", "Phone strip In Review", "+1", "next: Plan phones"])
        #expect(strip.text == "2 need you · Visual polish Building · Phone strip In Review · +1 · next: Plan phones")
    }

    @Test("one ask is singular, and none says nothing rather than zero")
    func needsYouWords() throws {
        let plan = try Self.plan()
        #expect(PlanStrip(plan: plan, needsYou: 1, orchestrator: .idle).parts.first == "1 needs you")
        #expect(PlanStrip(plan: plan, needsYou: 0, orchestrator: .idle).parts.first == "Visual polish Building")
    }

    @Test("with no plan, no asks and no tickets there's nothing to draw")
    func emptyIsNotDrawn() throws {
        let strip = PlanStrip(plan: .empty, needsYou: 0, orchestrator: .working)
        #expect(strip.isEmpty)
        #expect(!PlanStrip(plan: .empty, needsYou: 1, orchestrator: .working).isEmpty)
    }

    @Test("a waiting-only plan says only what's next")
    func waitingOnly() throws {
        let plan = try P.plan(themes: [], lines: [P.line("t1", "waiting", rank: 1, title: "Alpha")])
        #expect(PlanStrip(plan: plan, needsYou: 0, orchestrator: .idle).parts == ["next: Alpha"])
    }

    @Test("a ticket waiting on a decision is never next: not in the strip's next, and not in the band's")
    func decisionIsNotNext() throws {
        let plan = try P.plan(
            themes: [],
            lines: [
                P.line("t1", "waiting", rank: 1, title: "blocked"), P.line("t2", "waiting", rank: 2, title: "clear"),
            ])
        let without = PlanStrip(plan: plan, needsYou: 0, orchestrator: .idle)
        #expect(without.next == "blocked" && without.band?.next == "blocked, then clear", "blind to the board, as before")
        let strip = PlanStrip(plan: plan, needsYou: 0, orchestrator: .idle, statuses: ["t1": .needsDecision])
        #expect(strip.next == "clear" && strip.band?.next == "clear")
        #expect(strip.parts == ["next: clear"])
    }

    @Test("the band's Next names titles only, once each, cut at a word")
    func nextNamesTitles() throws {
        var first = PlanModelTests.line("t1", "waiting", rank: 1)
        first["title"] = "Reps: each person's asks reach the orchestrator, then skill-init writes the manager skill"
        var second = PlanModelTests.line("t2", "waiting", rank: 2)
        second["title"] = "Onboarding: skill-init explains what the manager skill is for, in the owner's words"
        let plan = try P.plan(themes: [], lines: [first, second])
        let next = try #require(PlanStrip(plan: plan, needsYou: 0, orchestrator: .idle).band?.next)
        #expect(next == "Reps: each person's asks reach the orchestrator, then skill-init\u{2026}, then Onboarding: skill-init explains what the manager skill is for, in the\u{2026}")
        #expect(!next.contains("\u{2014}") && next.components(separatedBy: "Reps").count == 2)
    }

    @Test("VoiceOver hears the orchestrator's state first, since the mark draws it, and each thing once")
    func accessibilityLabel() throws {
        let strip = PlanStrip(plan: try Self.plan(), needsYou: 2, orchestrator: .needsYou)
        #expect(strip.accessibilityLabel.hasPrefix("Orchestrator, waiting on you. 2 need you, Visual polish Building"))
        #expect(PlanStrip(plan: .empty, needsYou: 1, orchestrator: .none).accessibilityLabel == "No orchestrator. 1 needs you.")
        #expect(PlanStrip(plan: .empty, needsYou: 0, orchestrator: .working).accessibilityLabel == "Orchestrator, working.")
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
