import Foundation
import Testing

@testable import AgentKit

/// What `PlanUITests` pinned by launching the app (ov-538), over the files it
/// launched with: the lane and theme words on `plan-seeded.json`, the cost on
/// `plan-cost-seeded.json`, what a notice makes read again, and that Tasks is
/// what a board draws until Plan is chosen. That a notice reaches the phone and
/// a page opens stay in the app.
struct PlanPhoneBoardTests {
    typealias F = PlanPhoneFixtures

    @Test("a runner without board_plan draws its tasks whatever was chosen, and Tasks is the default")
    func tasksUnlessChosenAndKept() throws {
        let defaults = try #require(ScratchDefaults.suite("plan-phone-board-\(UUID().uuidString)"))
        #expect(!PlanChoice.shown(host: "h", workspace: "w", defaults: defaults), "Tasks is the default")
        #expect(!PlanChoice.showing(runnerKeepsPlan: true, chosen: false))
        #expect(PlanChoice.showing(runnerKeepsPlan: true, chosen: true))
        #expect(!PlanChoice.showing(runnerKeepsPlan: false, chosen: true), "Plan was chosen, then the runner stopped keeping one")
        #expect(!PlanChoice.showing(runnerKeepsPlan: false, chosen: false))
    }

    @Test("a refused or unanswered read says so in these words, and offers Try Again")
    func unavailableWords() {
        #expect(PlanWords.couldntRead == "Far Cooler couldn\u{2019}t read this board\u{2019}s plan.")
        #expect(PlanWords.tryAgain == "Try Again")
    }

    @Test("each Next Up lane names its theme and reason, and each theme tile reads on its own")
    func lanesAndTiles() throws {
        let plan = try F.plan("plan-seeded.json")
        let sections = PlanSections(plan: plan)
        let next = try #require(sections.waiting.next.first { $0.lane.name == "plan-phones" })
        #expect(plan.theme(of: next.lane)?.name == "Plan layer", "the lane doesn't name its theme")
        #expect(next.lane.reason.contains("Unblocked once the store"))
        let working = try #require(sections.building.first { $0.lane.name == "mac-vis" })
        #expect(plan.theme(of: working.lane)?.name == "Visual language", "a lane in Building doesn't name its theme")

        let tile = try #require(sections.themes.tiles.first { $0.name == "Visual language" })
        let said = tile.spoken()
        for part in ["Visual language", "4 of 18 done", "Needs you: Should the sidebar tint"] {
            #expect(said.contains(part), "the tile doesn't say \(part): \(said)")
        }
        #expect(!said.contains("Next: mac-vis finishes"), "the story's next line is back on the tile: \(said)")
    }

    @Test("work reads by what it does: the train as Train 9 with what it carries and its place, its lanes by title with no slug")
    func landingNamesTheTrain() throws {
        let plan = try F.plan("plan-seeded.json")
        let sections = PlanSections(plan: plan)
        let card = try #require(sections.trains.first { $0.train.name == "integ-9" })
        let said = card.spoken(ci: nil, now: sections.now)
        #expect(said.contains("Train 9"), "\(said)")
        #expect(said.contains("A read-only Files browser"), "the train doesn't say what it carries: \(said)")
        #expect(said.contains("Checking Here \u{00B7} step 1 of 3"), "no place on the ladder: \(said)")
        #expect(said.contains("Integrator \u{00B7} Opus"), "the train doesn't say who integrates: \(said)")
        #expect(!said.contains("integ-9"), "the slug is in the text: \(said)")
        let aboard = try #require(card.lanes.first { $0.name == "phones-b" })
        #expect(aboard.lane.heading.contains("A read-only Files browser"))
        #expect(aboard.note == nil, "a lane aboard says nothing of its state")
        #expect(!aboard.lane.heading.contains("integ-9") && !aboard.lane.heading.contains("phones-b"), "a slug is in the text")
    }

    @Test("a theme's record says what changed: the story it had before, when there was one")
    func whatChanged() throws {
        let plan = try F.plan("plan-seeded.json")
        let visual = try #require(plan.themes.first { $0.name == "Visual language" })
        let record = try F.record("plan-seeded.json", visual.id)
        let before = try #require(record.previousStory, "no What Changed: the record has no earlier story")
        #expect(!before.story.isEmpty)
        let none = try #require(plan.themes.first { try F.record("plan-seeded.json", $0.id).previousStory == nil })
        #expect(none.id != visual.id, "a theme with no earlier story offers no What Changed")
    }

    // MARK: Cost

    @Test("cost on the plan, from the CLI's own bytes: over budget flagged, the week, the split, the pair held back")
    @MainActor func cost() throws {
        let plan = try F.plan("plan-cost-seeded.json")
        let en = F.en
        let cost = try #require(plan.cost)
        #expect(cost.isWorthShowing)
        let theme = try #require(plan.themes.first { $0.name == "Invoices" })
        let over = try #require(PlanWords.overBudget(theme), "no theme over budget is flagged")
        #expect(PlanWords.budgetSpoken(over, locale: en) == "Over budget. 4.9 million of 2 million tokens used.")
        #expect(PlanWords.budgetLine(over, locale: en).hasPrefix("Over budget"))
        #expect(PlanWords.week(cost.weekTokens, locale: en).contains("tokens in the last 7 days"))
        #expect(PlanWords.weekDollars(cost, locale: en) == "about $37.80 API-equivalent", "the week's total in dollars leads")
        let rows = cost.week.map { PlanWords.weekRow($0, locale: en).detail }
        #expect(rows.contains { $0.contains("about $31.00 API-equivalent") }, "\(rows)")
        #expect(PlanWords.heldBack(cost.compareHeldBack)?.contains("held back until three cards have landed") == true)
        #expect(PlanWords.inFlight(cost, locale: en)?.contains("tokens on cards that haven\u{2019}t landed") == true)
        let share = cost.compare.map { PlanWords.compareRow($0, locale: en).detail }
        #expect(share.contains { $0.contains("3.6 finished cards") }, "\(share)")
        #expect(PlanThemeSpend.hasSomething(theme), "the theme page draws its spend")
        #expect(PlanWords.trend(theme.trendTokens) != nil, "and its trend")
        let lane = try #require(plan.lanes.first { PlanWords.overBudget($0) != nil })
        #expect(PlanWords.overBudget(lane) != nil, "\(lane.name) is over its budget")
    }

    // MARK: Which boards read again

    @Test("a plan notice makes the boards that asked for their plan and that it names read again, and no others")
    func planNoticeReadsTheNamedBoard() throws {
        let news = PlanNews(workspace: "Board-A")
        #expect(news.boardsToRead(asked: ["board-a", "Board-A", "board-b"], among: ["board-a", "board-b", "board-c"]) == ["board-a"])
        #expect(
            news.boardsToRead(asked: ["board-b"], among: ["board-a", "board-b"]) == [],
            "a board nobody asked the plan of never reads until someone picks Plan")
        #expect(news.boardsToRead(asked: ["board-a"], among: ["board-b"]).isEmpty)
    }

    @Test("a task notice moves counts the plan derives, so every moved board that asked reads again")
    func taskNoticeReadsAgain() {
        #expect(PlanNews.boardsToReadAfterTasks(moved: ["a", "b", "c"], asked: ["a", "c", "z"]) == ["a", "c"])
        #expect(PlanNews.boardsToReadAfterTasks(moved: ["a"], asked: []).isEmpty)
    }
}
