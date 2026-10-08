import Foundation
import Testing

@testable import AgentKit

/// Cost on the plan (ov-307) as the Mac and the phones read it:
/// `test/fixtures/plan.json`, which the CLI's and the client's tests write from
/// the wire, and the words, the trend and the comparison drawn from it. The
/// dates are the runner's own clock in the fixture, so nothing here reads
/// "today" (it holds with `TZ=UTC` and without).
struct PlanCostTests {
    @Test("plan --json carries a theme's budget and trend, a lane's budget, the week and the comparison")
    func decodesTheFixture() throws {
        let plan = try PlanModelTests.fixture()
        #expect(plan.themes[0].budgetTokens == 250_000)
        #expect(plan.themes[0].trendTokens == [0, 0, 0, 40_000, 120_000, 0, 160_000])
        #expect(plan.lanes[1].budgetTokens == 500_000)
        #expect(plan.lanes[0].budgetTokens == nil)
        let cost = try #require(plan.cost)
        #expect(cost.weekTokens == 34_200_000 && cost.compareHeldBack == 2)
        #expect(cost.compare.map(\.cardShareMilli) == [5000, 3400])
        #expect(cost.inFlightTokens == 1_200_000 && cost.inFlightCostMicros == nil)
        #expect(cost.compare[1].costMicros == nil, "an unpriced pair has no dollars, never zero")
    }

    @Test("an older answer, from a runner without board_cost, reads and draws nothing")
    func anOlderAnswerReads() throws {
        let plan = try PlanModelTests.plan(
            themes: [PlanModelTests.theme("Visual", cards: ["t1"], ordinal: 1)],
            lanes: [PlanModelTests.lane("a", "building", cards: ["t1"])])
        #expect(plan.cost == nil && plan.themes[0].budgetTokens == nil && plan.themes[0].trendTokens == nil)
        #expect(!PlanThemeSpend.hasSomething(plan.themes[0]))
        #expect(PlanWords.overBudget(plan.lanes[0]) == nil && PlanWords.trend(plan.themes[0].trendTokens) == nil)
    }

    @Test("over budget is past it, and equal is within")
    func overBudget() throws {
        let plan = try PlanModelTests.fixture()
        let theme = plan.themes[0]
        let over = try #require(PlanWords.overBudget(theme))
        #expect(over == .over(used: 320_000, budget: 250_000))
        #expect(PlanWords.budgetLine(over, locale: Locale(identifier: "en_US")) == "Over budget: 320K of 250K tokens")
        #expect(PlanWords.overBudget(plan.lanes[1]) == nil, "470K of 500K is within")
        let within = try #require(PlanWords.budget(plan.lanes[1].spend, against: plan.lanes[1].budgetTokens))
        #expect(PlanWords.budgetLine(within, locale: Locale(identifier: "en_US")) == "470K of 500K tokens budgeted")
        var spend = PlanSpend()
        spend.inputTokens = 500
        #expect(PlanWords.budget(spend, against: 500) == .within(used: 500, budget: 500), "equal is within")
        spend.inputTokens = 501
        #expect(PlanWords.budget(spend, against: 500)?.isOver == true)
        #expect(PlanWords.budget(spend, against: nil) == nil, "no budget, nothing to flag")
    }

    @Test("a lane over its budget is the warning its row draws in amber")
    func aLaneWarning() throws {
        var lane = try PlanModelTests.fixture().lanes[1]
        #expect(PlanWords.overBudget(lane) == nil)
        lane.budgetTokens = 100_000
        #expect(PlanWords.overBudget(lane, locale: Locale(identifier: "en_US")) == "Over budget: 470K of 100K tokens")
        #expect(
            PlanWords.overBudgetSpoken(lane, locale: Locale(identifier: "en_US"))
                == "Over budget. 470 thousand of 100 thousand tokens used.", "VoiceOver hears words, not a suffix letter")
    }

    @Test("a screen reader hears the counts as words, never a suffix letter")
    func spoken() throws {
        let us = Locale(identifier: "en_US")
        #expect(PlanWords.spokenTokens(6_100_000, locale: us) == "6.1 million")
        #expect(PlanWords.spokenTokens(340_000, locale: us) == "340 thousand")
        #expect(PlanWords.spokenTokens(812, locale: us) == "812")
        let over = PlanBudget.over(used: 6_100_000, budget: 5_000_000)
        #expect(PlanWords.budgetSpoken(over, locale: us) == "Over budget. 6.1 million of 5 million tokens used.")
        let trend = try #require(PlanWords.trend([0, 0, 0, 40_000, 120_000, 0, 160_000]))
        #expect(
            PlanWords.trendSpoken(trend, locale: us)
                == "Last 7 days by UTC day, oldest first: none, none, none, 40 thousand, 120 thousand, none, 160 thousand today. 320 thousand tokens in all."
        )
    }

    @Test("a trend is seven days against the busiest, and a day with something keeps a sliver")
    func trendHeights() throws {
        #expect(PlanWords.trend(nil) == nil && PlanWords.trend([]) == nil)
        #expect(PlanWords.trend([0, 0, 0, 0, 0, 0, 0]) == nil, "a chart of nothing says nothing")
        #expect(PlanWords.trend([1, 2, 3]) == nil, "seven days or none")
        let t = try #require(PlanWords.trend([0, 0, 0, 40_000, 120_000, 0, 160_000]))
        #expect(t.heights() == [0, 0, 0, 0.25, 0.75, 0, 1])
        let tiny = try #require(PlanWords.trend([0, 0, 0, 0, 0, 1, 1_000_000]))
        #expect(tiny.heights()[5] == 0.08, "a day with anything never rounds away")
        #expect(tiny.heights()[0] == 0)
    }

    @Test("the week is a count with no percentage, and says why")
    func theWeek() {
        let us = Locale(identifier: "en_US")
        #expect(PlanWords.week(34_200_000, locale: us) == "34M tokens in the last 7 days on this runner")
        #expect(!PlanWords.week(34_200_000, locale: us).contains("%"))
        #expect(!PlanWords.weekNote.contains("%"))
        #expect(PlanWords.weekNote.contains("weekly limit"))
    }

    @Test("the week's total leads in tokens and API-equivalent dollars, split by harness and model, and the split adds up")
    func weekTotalAndSplit() throws {
        let us = Locale(identifier: "en_US")
        let cost = try #require(try PlanModelTests.fixture().cost)
        #expect(cost.week.map(\.tokens).reduce(0, +) == cost.weekTokens, "the parts are the total")
        #expect(cost.week.compactMap(\.costMicros).reduce(0, +) == cost.weekCostMicros)
        #expect(PlanWords.weekDollars(cost, locale: us) == "about $45.00 API-equivalent")
        let rows = cost.week.map { PlanWords.weekRow($0, locale: us) }
        #expect(rows.map(\.title) == ["Claude Code · opus", "Codex · gpt-5.6"])
        #expect(rows[0].detail == "28M tokens · about $41.20 API-equivalent")
        #expect(rows[1].detail == "6.2M tokens · about $3.80 API-equivalent")
        #expect(rows[0].spoken.contains("28 million tokens"))
        var unpriced = cost
        unpriced.weekCostMicros = nil
        #expect(PlanWords.weekDollars(unpriced, locale: us) == PlanWords.dollarsNotReported, "never $0 for dollars we can't count")
        #expect(PlanWords.weekDollars(PlanCostRead(weekTokens: 5), locale: us) == nil, "an older runner sent no split")
        let none = PlanWords.weekRow(PlanWeekSpend(harness: "cursor", model: "", tokens: 900), locale: us)
        #expect(none.detail == "900 tokens · API-equivalent dollars: Not reported")
    }

    @Test("each comparison row says n, tokens a card, and dollars only when every turn was priced")
    func compareRows() throws {
        let us = Locale(identifier: "en_US")
        let cost = try #require(try PlanModelTests.fixture().cost)
        let claude = PlanWords.compareRow(cost.compare[0], locale: us)
        #expect(claude.title == "Claude Code · opus")
        #expect(claude.detail == "5 finished cards · 1.5M tokens a card · about $2.50 a card API-equivalent")
        #expect(claude.spoken.contains("1.5 million tokens a card") && claude.spoken.contains("API-equivalent"))
        let codex = PlanWords.compareRow(cost.compare[1], locale: us)
        #expect(codex.title == "Codex · gpt-5.6")
        #expect(
            codex.detail == "3.4 finished cards · 300K tokens a card · API-equivalent dollars: Not reported",
            "unpriced reads Not reported, never nothing and never $0")
        let unnamed = PlanWords.compareRow(PlanHarnessCost(harness: "cursor", model: "", cardShareMilli: 3000, tokens: 30), locale: us)
        #expect(unnamed.title == "Cursor · no model named")
        #expect(PlanWords.heldBack(2) == "2 other harness and model pairs held back until three cards have landed")
        #expect(PlanWords.heldBack(1) == "1 other harness and model pair held back until three cards have landed")
        #expect(PlanWords.heldBack(0) == nil)
        #expect(planCompareMinimumCards == 3)
    }

    @Test("spend on cards that haven't landed is said apart, with its dollars API-equivalent or Not reported")
    func inFlight() throws {
        let us = Locale(identifier: "en_US")
        let cost = try #require(try PlanModelTests.fixture().cost)
        #expect(
            PlanWords.inFlight(cost, locale: us)
                == "1.2M tokens on cards that haven’t landed · API-equivalent dollars: Not reported")
        var priced = cost
        priced.inFlightCostMicros = 9_000_000
        #expect(PlanWords.inFlight(priced, locale: us) == "1.2M tokens on cards that haven’t landed · about $9.00 API-equivalent")
        #expect(PlanWords.inFlight(PlanCostRead(), locale: us) == nil)
        #expect(PlanCostRead(inFlightTokens: 1).isWorthShowing)
    }

    @Test("a share of the landed cards reads with its decimal only when it isn't whole")
    func cardShares() {
        let us = Locale(identifier: "en_US")
        #expect(PlanWords.cardShare(5000, locale: us) == "5 finished cards")
        #expect(PlanWords.cardShare(3600, locale: us) == "3.6 finished cards")
    }

    @Test("a harness is named as a person names it")
    func harnessNames() {
        #expect(PlanWords.harnessName("claude") == "Claude Code")
        #expect(PlanWords.harnessName("codex") == "Codex")
        #expect(PlanWords.harnessName("zed") == "Zed")
        #expect(PlanWords.harnessName("") == "Unknown")
    }

    @Test("a cost with nothing in it isn't worth a section")
    func worthShowing() {
        #expect(!PlanCostRead().isWorthShowing)
        #expect(PlanCostRead(weekTokens: 1).isWorthShowing)
        #expect(PlanCostRead(compareHeldBack: 1).isWorthShowing)
    }

    @Test("board_cost is a capability the app can name")
    func capability() {
        #expect(Capability.boardCost.rawValue == "board_cost")
    }
}
