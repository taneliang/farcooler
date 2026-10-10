import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The folded window's plan strip says what the phones' strip says (ov-343):
/// for one plan and one count, the Mac row's words are `PlanStrip.text`, the
/// line the phones draw (`Connection.planStrip` in apps/ios), whatever state
/// the orchestrator is in. Plans are `plan --json`'s bytes, through
/// `PlanModel.decode`, the parser both apps read with.
@MainActor
struct PlanStripWordsTests {
    /// The CLI's fixture with its lanes replaced by `lanes` (name, state),
    /// each a copy of the fixture's first lane, and `order` naming the queued.
    static func plan(_ lanes: [(String, String)], order: [String] = []) throws -> PlanModel {
        var object = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        let template = try #require((object["lanes"] as? [[String: Any]])?.first)
        object["lanes"] = lanes.map { name, state in
            template.merging(["id": "lane-\(name)", "name": name, "short": name, "state": state, "title": ""]) { $1 }
        }
        object["order"] = order.map { "lane-\($0)" }
        return try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
    }

    /// What the phone's strip says for `plan`: AgentKit's model, built as
    /// `Connection.planStrip` builds it, with a state and a line of its own.
    static func phone(_ plan: PlanModel, needsYou: Int, _ state: PlanStripOrchestrator) -> String {
        PlanStrip(plan: plan, needsYou: needsYou, orchestrator: state, line: "Dispatching ov-321").text
    }

    @Test("One plan, the same words on the Mac and the phone, in every orchestrator state")
    func sameWords() throws {
        let plan = try Self.plan(
            [
                ("mac-vis", "building"), ("phones-b", "review"), ("mac-rel", "fixing"), ("plan-phones", "queued"),
                ("vis-lint", "queued"), ("land-129", "landed"),
            ], order: ["plan-phones", "vis-lint"])
        for needsYou in [0, 1, 2] {
            let mac = try #require(PlanStripRow.words(plan, needsYou: needsYou))
            for state in [PlanStripOrchestrator.none, .working, .needsYou, .failed, .idle, .stopped] {
                #expect(mac == Self.phone(plan, needsYou: needsYou, state), "\(needsYou) asks, \(state)")
            }
        }
        #expect(
            PlanStripRow.words(plan, needsYou: 2)
                == "2 need you · mac-vis Building · phones-b In Review · +1 · next: plan-phones")
        #expect(PlanStripRow.words(plan, needsYou: 1)?.hasPrefix("1 needs you · ") == true)
    }

    @Test("The fixture's plan, as the CLI wrote it, reads the same on both")
    func theCLIsPlan() throws {
        let plan = try PlanModel.decode(PlanViewTests.fixture())
        let mac = try #require(PlanStripRow.words(plan, needsYou: 1))
        #expect(mac == Self.phone(plan, needsYou: 1, .working))
        #expect(mac.hasPrefix("1 needs you · "), Comment(rawValue: mac))
    }

    @Test("With no plan, no asks and no lanes, there are no words")
    func nothingToSay() throws {
        #expect(PlanStripRow.words(.empty, needsYou: 0) == nil)
        #expect(PlanStripRow.words(.empty, needsYou: 3) == "3 need you")
    }
}
