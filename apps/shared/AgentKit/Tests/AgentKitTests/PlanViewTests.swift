import Foundation
import Testing

@testable import AgentKit

/// The Plan's view as the Rust core answers it (ov-1016): the golden
/// `test/fixtures/plan-view-apple.json` decodes into every slot the Mac and
/// the phones draw, and the bridge this package's tests link answers the
/// golden's request with the golden's view, through the same request AgentKit
/// builds for the apps.
struct PlanViewTests {
    static func golden() throws -> (request: [String: Any], answer: Data) {
        let data = try Data(contentsOf: PlanPhoneFixtures.root.appendingPathComponent("test/fixtures/plan-view-apple.json"))
        let object = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        let request = try #require(object["request"] as? [String: Any])
        return (request, try JSONSerialization.data(withJSONObject: try #require(object["answer"])))
    }

    static func view() throws -> PlanView {
        try #require(PlanView.decode(answer: try golden().answer))
    }

    @Test("the golden decodes into every slot: sections, cards, notes, runs, landings, agents and tiles")
    func decodesTheGolden() throws {
        let view = try Self.view()
        #expect(view.sections.map(\.kind) == [.needsYou, .landing, .inReview, .building, .waiting, .agents, .themes, .landedToday])
        #expect(view.sections.map(\.title) == ["Needs You", "Landing", "In Review", "Building", "Waiting", "Agents", "Themes", "Landed Today"])
        #expect(view.landedToday.map(\.key) == ["ov-460"] && view.section(.landedToday)?.folded == true)
        #expect(view.section(.landing)?.help == "Reviewed tickets land on main together, after one round of checks.")
        #expect(view.needsYou == 2 && view.now == 1_800_000_000_000)

        let built = try #require(view.building.first { $0.key == "ov-310" })
        #expect(built.mark == .building && built.markAttention)
        #expect(built.second == "Building \u{00B7} ov-310 \u{00B7} Builder \u{00B7} sonnet \u{00B7} 380K \u{00B7} 1 h")
        #expect(built.theme == "Trustworthy agent view" && built.agent?.who == "Builder \u{00B7} sonnet")
        let run = try #require(built.notes.first { $0.kind == .run })
        #expect(run.text == "Prepare Workspace \u{00B7} Approval Required" && run.attention && run.help == "Waiting for approval from owner.")
        #expect(built.run?.gate?.label == "Approval Required" && built.run?.phaseLabel == "Awaiting Approval")
        #expect(view.inReview.first { $0.key == "ov-312" }?.mark == .fixing)
        #expect(view.inReview.first { $0.key == "ov-482" }?.notes.map(\.kind) == [.warning])

        let landing = try #require(view.landings.first)
        #expect(landing.label == "Landing 86" && landing.phaseWord == "Waiting for Tickets" && landing.ladder.at == 0)
        #expect(landing.ladder.steps.map(\.label) == ["Waiting for Tickets", "Checking on CI", "Landed"])
        #expect(landing.landerWords == "Lander \u{00B7} opus \u{00B7} 1.4M" && landing.lander?.model == "opus")
        let aboard = try #require(landing.tickets.first)
        #expect(aboard.note == nil && aboard.mark == .landing && aboard.branch == "team-dir")

        #expect(view.waiting.map(\.kind) == [.notRanked] && !view.namesWaitingGroups)
        #expect(view.waiting.first?.id == "plan-not-ranked")
        #expect(view.agents.rows.map(\.id) == ["pane:orch", "pane:i1", "pane:b1"])
        #expect(view.agents.rows.first?.cost == "\u{2248}$14.20")
        let tile = try #require(view.themes.tiles.first { $0.name == "Marketing" })
        #expect(tile.ownerAsk == "How do we ship the site?" && tile.cards.map(\.glyph) == [.notStarted])
        #expect(view.themes.finishedTitle == "Finished" && view.themes.finishedWords == "1 theme")
    }

    @Test("the bridge answers the golden's request with the golden's view, as the apps ask it")
    func theBridgeAnswersTheGolden() throws {
        let (request, _) = try Self.golden()
        let plan = try PlanModel.decode(JSONSerialization.data(withJSONObject: try #require(request["plan"])))
        let context = try #require(request["context"] as? [String: Any])
        let statuses = (context["statuses"] as? [String: String] ?? [:]).compactMapValues { TaskStatus(rawValue: $0) }
        let workflow = try JSONSerialization.data(withJSONObject: try #require(request["workflow"]))
        let built = PlanView.build(
            plan: plan, context: PlanViewContext(needsYou: context["needs_you"] as? Int ?? 0, statuses: statuses, utcOffset: 0),
            workflow: workflow)
        #expect(built == (try Self.view()))
    }

    @Test("a plan made in code has no wire form, so no view; the same plan read from the runner has one")
    func aPlanInCodeHasNoView() throws {
        #expect(PlanView.build(plan: PlanModel(nowMs: 1)) == nil)
        let read = try PlanModel.decode(Data(#"{"now_ms": 1, "themes": [], "lines": [], "cards": []}"#.utf8))
        let view = try #require(PlanView.build(plan: read))
        #expect(view.section(.landing)?.empty == "Nothing is landing right now.")
        #expect(view.section(.waiting)?.empty == "Nothing is waiting.")
    }
}
