import Foundation
import Testing

@testable import AgentKit

/// Tickets and landings named by what they do (ov-462, ov-872), as the apps
/// read them from the plan.
struct PlanTitlesTests {
    static func line(_ task: String, title: String? = nil, stage: String = "building") -> [String: Any] {
        var line = PlanModelTests.line(task, stage)
        if let title { line["title"] = title }
        return line
    }

    @Test("a ticket's title comes first and its key second; with none, the key stands alone")
    func aTicketHeadsWithItsTitle() throws {
        let plan = try PlanModelTests.plan(
            themes: [],
            lines: [
                Self.line("t1", title: "Agents message the orchestrator"), Self.line("t2"), Self.line("t3", title: ""),
            ])
        let (titled, bare, empty) = (plan.lines[0], plan.lines[1], plan.lines[2])
        #expect((titled.heading, titled.key) == ("Agents message the orchestrator", "ov-1"))
        #expect(bare.heading == "ov-2", "no title: the key")
        #expect(empty.heading == "ov-3", "an empty title is none")
    }

    @Test("the fixture: titles on the tickets, and Landing 9 with its summary and its lander")
    func theFixtureCarriesTitles() throws {
        let plan = try PlanModelTests.fixture()
        #expect(plan.lines.map(\.heading) == ["Title of ov-2", "Title of ov-1", "Title of ov-4", "Title of ov-3"])
        let nine = try #require(plan.landings.first)
        #expect((nine.label, nine.subtitle, nine.carries) == ("Landing 9", nil, "Title of ov-4"))
        let eight = plan.landings[1]
        #expect((eight.label, eight.subtitle, eight.carries) == ("Landing 8", "Agents message the plan", "Title of ov-3"))
        #expect(nine.lander == "00000000-0000-0000-0000-000000007004")
        #expect(eight.lander == nil)
    }

    @Test("a landing reads Landing N from its number with its title under it, and a runner's own \"Landing N ·\" is not said twice")
    func landingLabels() throws {
        func landing(_ number: Int, title: String? = nil, summary: String = "") -> [String: Any] {
            var landing = PlanModelTests.landing(number, "gating", title: title)
            landing["summary"] = summary
            return landing
        }
        let plan = try PlanModelTests.plan(
            themes: [], lines: [],
            landings: [
                landing(72), landing(5, title: "Phones catch up"), landing(3, title: "Landing 3 \u{00B7} Phones catch up"),
                landing(4, title: "Landing 4"), landing(6, title: "Landing 6 \u{00B7} ", summary: "Two fixes"),
            ])
        #expect(plan.landings.map(\.label) == ["Landing 72", "Landing 5", "Landing 3", "Landing 4", "Landing 6"])
        #expect(plan.landings.map(\.subtitle) == [nil, "Phones catch up", "Phones catch up", nil, nil], "the title is the subtitle, never the name")
        #expect(plan.landings.map(\.carries) == [nil, nil, nil, nil, "Two fixes"])
        #expect(plan.landings.allSatisfy { $0.lander == nil })
    }

    @Test("tidy words count cards")
    func tidyWords() {
        let card = PlanFlaggedCard(task: "t", key: "ov-1", status: "in_review")
        #expect(PlanWords.tidy(PlanOutside(tickets: [], openCards: 0, tidy: [card, card])) == "2 cards to tidy")
        #expect(PlanWords.tidy(PlanOutside(tickets: [], openCards: 0, tidy: [card])) == "1 card to tidy")
    }

    @Test("a task's line names the ticket's stage; the strip says its title, cut to fit")
    func titlesReachTheLineAndTheStrip() throws {
        let plan = try PlanModelTests.fixture()
        let line = try #require(plan.taskLine("00000000-0000-0000-0000-000000001001"))
        #expect(line.stageWords == "In Review")
        #expect(line.text == "In Review \u{00B7} Visual language")
        #expect(PlanStrip(plan: plan, needsYou: 0, orchestrator: .working).now.map(\.name) == ["Title of ov-1", "Title of ov-4"])
        #expect(PlanWords.short("Agents tell the orchestrator when they are done") == "Agents tell the\u{2026}")
        #expect(PlanWords.short("Short title") == "Short title")
        #expect(PlanWords.status(plan.lines[2]) == "Landing \u{00B7} in Landing 9", "a row says in Landing 9, not the lander's id")
    }
}
