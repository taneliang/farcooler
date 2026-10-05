import Foundation
import Testing

@testable import AgentKit

/// Decided for you (ov-304), as the Mac and the iPhone read it: the rulings
/// in `test/fixtures/plan.json`, which the CLI's test writes from its output
/// and the client core's test holds to the wire's bytes.
struct PlanRulingsTests {
    /// The fixture's two rulings read in the runner's order, R-2 standing
    /// first, with every part a row draws.
    @Test func theFixtureReadsStandingFirst() throws {
        let plan = try PlanModelTests.fixture()
        #expect(plan.rulings.map(\.short) == ["R-2", "R-1"])
        let standing = try #require(plan.standingRulings.first)
        #expect(standing.decision == "The inbox is amber.")
        #expect(standing.why == "It's the one attention color, so the inbox reads as needing you.")
        #expect(standing.reversal == "One token; every surface follows.")
        #expect(standing.cards.map(\.key) == ["ov-1"])
        #expect(standing.theme == "Visual language")
        #expect(standing.state == .standing)
        #expect(standing.settledAt == nil)
        let settled = try #require(plan.settledRulings.first)
        #expect((settled.short, settled.state, settled.note) == ("R-1", .confirmed, "Keep it."))
        #expect(settled.settledBy == "manager")
        #expect(plan.settledRulings.count == 1)
    }

    /// Copy Reference copies what the owner says to the orchestrator.
    @Test func theReferenceNamesTheShortIdAndTheDecision() throws {
        let plan = try PlanModelTests.fixture()
        #expect(plan.rulings[0].reference == "ruling R-2: The inbox is amber.")
        #expect(PlanRuling(id: "x", number: 12, decision: "Unread stays.", why: "", reversal: "").reference
            == "ruling R-12: Unread stays.")
    }

    /// The line under a decision says what it touches, and nothing when it
    /// touches nothing.
    @Test func touchesNameTheCardsThenTheTheme() throws {
        let plan = try PlanModelTests.fixture()
        #expect(PlanWords.rulingTouches(plan.rulings[0]) == "ov-1 · Visual language")
        #expect(PlanWords.rulingTouches(plan.rulings[1]) == nil)
        #expect(PlanWords.rulingAccessibility(plan.rulings[1]).hasSuffix("Confirmed"))
    }

    /// An answer without `rulings`, as an older CLI prints, still reads, with
    /// none. Rulings alone don't make a board planned: they never switch its
    /// layout (review 1005a F1), and `showsNothing` is what hides the notice.
    @Test func aPlanWithoutRulingsStillReads() throws {
        let json = #"{"now_ms": 1, "themes": [], "lanes": [], "order": [], "cards": []}"#
        let plan = try PlanModel.decode(Data(json.utf8))
        #expect(plan.rulings.isEmpty)
        #expect(plan.isEmpty)
        let ruled = PlanModel(rulings: [PlanRuling(id: "x", number: 1, decision: "A", why: "B", reversal: "C")])
        #expect(ruled.isEmpty, "nothing planned")
        #expect(!ruled.showsNothing, "but something to show")
        #expect(plan.showsNothing)
    }

    /// A state this build has no word for reads as unknown, not as a plan
    /// that can't be read.
    @Test func anUnknownStateStillReads() throws {
        var text = try String(contentsOf: Self.fixtureURL(), encoding: .utf8)
        text = text.replacingOccurrences(of: #""state": "confirmed""#, with: #""state": "withdrawn""#)
        let plan = try PlanModel.decode(Data(text.utf8))
        #expect(plan.rulings[1].state == .unknown)
    }

    /// The signature moves with anything a row shows, so the row washes.
    @Test func theSignatureMovesWithWhatTheRowShows() throws {
        let plan = try PlanModelTests.fixture()
        var moved = plan.rulings[0]
        let before = PlanWords.rulingSignature(moved)
        moved.state = .reversed
        #expect(PlanWords.rulingSignature(moved) != before)
        moved = plan.rulings[0]
        moved.why = "Another reason."
        #expect(PlanWords.rulingSignature(moved) != before)
    }

    static func fixtureURL() -> URL {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan.json")
    }
}
