import Foundation
import Testing

@testable import AgentKit

/// What `PlanRulingsUITests` pinned over `plan-rulings-seeded.json`, a real
/// board's four rulings (ov-538): the open ones newest first, the settled one
/// folded into Past Decisions, what Keep and Keep All do to the plan on screen,
/// and who is offered the marks at all. The swipe, the confirmation and what
/// reaches the orchestrator stay in the app (and in `PlanRulingActionsTests`).
struct PlanPhoneRulingsTests {
    typealias F = PlanPhoneFixtures

    static func plan() throws -> PlanModel { try F.plan("plan-rulings-seeded.json") }

    @Test("open rulings come first, newest first, and the settled one folds into Past Decisions")
    func openFirstAndPastFolds() throws {
        let plan = try Self.plan()
        let shown = PlanRulingsShown(plan: plan, keeps: true, canMark: true)
        #expect(shown.open.map(\.short) == ["R-4", "R-3", "R-2"], "newest first")
        #expect(shown.past.map(\.short) == ["R-1"])
        #expect(PlanWords.rulingAccessibility(shown.open[0]).contains("Why:"), "the reason is read")
        let settled = PlanWords.rulingAccessibility(shown.past[0])
        #expect(settled.hasSuffix("Kept"), "\(settled)")
        #expect(!PlanWords.rulingAccessibility(shown.open[0]).hasSuffix("Kept"))
        #expect(PlanWords.pastDecisions == "Past Decisions" && PlanWords.decidedForYou == "Decided For You")
    }

    @Test("Keep moves a ruling from Decided For You into Past Decisions, and leaves the others open")
    func keepMovesOne() throws {
        let plan = try Self.plan()
        let r2 = try #require(plan.rulings.first { $0.short == "R-2" })
        let kept = plan.showingKept([r2.id])
        let shown = PlanRulingsShown(plan: kept, keeps: true, canMark: true)
        #expect(shown.open.map(\.short) == ["R-4", "R-3"], "the others are still open")
        #expect(shown.past.map(\.short).sorted() == ["R-1", "R-2"])
        let now = try #require(shown.past.first { $0.short == "R-2" })
        #expect(PlanWords.rulingAccessibility(now).hasSuffix("Kept"))
        #expect(now.settledBy == "user" && now.settledAt == plan.nowMs, "the owner's own mark, now")
    }

    @Test("Keep All keeps every open ruling: the section and its Keep All are gone, and nothing is left open")
    func keepAll() throws {
        let plan = try Self.plan()
        let kept = plan.showingKept(Set(plan.openRulings.map(\.id)))
        let shown = PlanRulingsShown(plan: kept, keeps: true, canMark: true)
        #expect(shown.open.isEmpty && !shown.showsKeepAll)
        #expect(Set(shown.past.map(\.short)) == ["R-4", "R-3", "R-2", "R-1"])
        for ruling in shown.past { #expect(PlanWords.rulingAccessibility(ruling).hasSuffix("Kept"), "\(ruling.short)") }
    }

    @Test("a ruling that is not open is not touched by Keep")
    func keepLeavesSettledAlone() throws {
        let plan = try Self.plan()
        let r1 = try #require(plan.rulings.first { $0.short == "R-1" })
        let before = r1.settledAt
        let again = plan.showingKept([r1.id]).rulings.first { $0.short == "R-1" }
        #expect(again?.settledAt == before && again?.settledBy == "manager", "kept by the manager stays that")
    }

    @Test("Keep All is offered while more than one is open, and only to a phone that may mark")
    func keepAllIsOffered() throws {
        let plan = try Self.plan()
        #expect(PlanRulingsShown(plan: plan, keeps: true, canMark: true).showsKeepAll)
        #expect(!PlanRulingsShown(plan: plan, keeps: true, canMark: false).showsKeepAll)
        let one = plan.showingKept(Set(plan.openRulings.dropFirst().map(\.id)))
        #expect(PlanRulingsShown(plan: one, keeps: true, canMark: true).open.count == 1)
        #expect(!PlanRulingsShown(plan: one, keeps: true, canMark: true).showsKeepAll, "one open ruling has nothing to keep all of")
    }

    @Test("a phone with a Read grant is offered no marks: Keep would flip and silently put itself back")
    func readGrantOffersNoMarks() {
        #expect(!RulingActions.canMark(runnerTakesMarks: true, grantedScope: "read"))
        #expect(RulingActions.canMark(runnerTakesMarks: true, grantedScope: "control"))
        #expect(RulingActions.canMark(runnerTakesMarks: true, grantedScope: nil))
        #expect(!RulingActions.canMark(runnerTakesMarks: false, grantedScope: "control"), "a runner that takes no marks gets none")
    }

    @Test("Copy Reference copies what the owner says to the orchestrator, on an open ruling")
    func copyReference() throws {
        let plan = try Self.plan()
        let r3 = try #require(plan.openRulings.first { $0.short == "R-3" })
        #expect(PlanWords.copyReference == "Copy Reference")
        #expect(r3.reference == "ruling R-3: \(r3.decision)")
    }

    @Test("a runner without board_rulings, or a board with none, shows nothing at all")
    func nothingWithoutRulings() throws {
        let plan = try Self.plan()
        let without = PlanRulingsShown(plan: plan, keeps: false, canMark: true)
        #expect(without.isEmpty && without.open.isEmpty && without.past.isEmpty && !without.showsKeepAll)
        var none = plan
        none.rulings = []
        #expect(PlanRulingsShown(plan: none, keeps: true, canMark: true).isEmpty)
    }
}
