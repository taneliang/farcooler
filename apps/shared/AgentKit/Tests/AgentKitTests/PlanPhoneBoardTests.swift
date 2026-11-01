import Foundation
import Testing

@testable import AgentKit

/// What `PlanUITests` pinned by launching the app (ov-538), over the files it
/// launched with: the ticket and theme words on `plan-seeded.json`, the cost on
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

    @Test("each Next Up ticket names its theme and reason, and each theme tile reads on its own")
    func ticketsAndTiles() throws {
        let plan = try F.plan("plan-seeded.json")
        let sections = PlanSections(plan: plan)
        let next = try #require(sections.waiting.next.first { $0.line.key == "ov-274" })
        #expect(plan.theme(of: next.line)?.name == "Plan layer", "the ticket doesn't name its theme")
        #expect(next.line.reason.contains("Unblocked once the store"))
        let working = try #require(sections.building.first { $0.line.key == "ov-220" })
        #expect(plan.theme(of: working.line)?.name == "Visual language", "a ticket in Building doesn't name its theme")

        let tile = try #require(sections.themes.tiles.first { $0.name == "Visual language" })
        let said = tile.spoken()
        for part in ["Visual language", "4 of 18 done", "Needs you: Should the sidebar tint"] {
            #expect(said.contains(part), "the tile doesn't say \(part): \(said)")
        }
        #expect(!said.contains("Next: Mac finishes"), "the story's next line is back on the tile: \(said)")
    }

    @Test("work reads by what it does: the landing as Landing 9 with what it carries and its place, its tickets by title with no branch")
    func landingNamesTheLanding() throws {
        let plan = try F.plan("plan-seeded.json")
        let sections = PlanSections(plan: plan)
        let card = try #require(sections.landings.first { $0.landing.number == 9 })
        let said = card.spoken(ci: nil, now: sections.now)
        #expect(said.contains("Landing 9"), "\(said)")
        #expect(said.contains("A read-only Files browser"), "the landing doesn't say what it carries: \(said)")
        #expect(said.contains("Checking Here \u{00B7} step 1 of 3"), "no place on the ladder: \(said)")
        #expect(said.contains("Lander \u{00B7} opus"), "the landing doesn't say who lands it: \(said)")
        #expect(!said.contains("a-integ9"), "the lander's id is in the text: \(said)")
        let aboard = try #require(card.tickets.first { $0.key == "ov-259" })
        #expect(aboard.line.heading.contains("read-only Files browser"))
        #expect(aboard.note == nil, "a ticket aboard says nothing of its state")
        #expect(!aboard.line.heading.contains("Landing 9") && !aboard.line.heading.contains("phones-b"), "a slug is in the text")
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
        let line = try #require(plan.lines.first { PlanWords.overBudget($0) != nil })
        #expect(PlanWords.overBudget(line) != nil, "\(line.key) is over its budget")
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
