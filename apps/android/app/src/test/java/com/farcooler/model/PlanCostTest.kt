package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.util.Locale

/**
 * Cost on the plan (ov-307) on Android, against the bytes the Rust client
 * makes of a plan: `test/fixtures/plan.json`, held there by the client crate's
 * and the CLI's tests and read by AgentKit's `PlanCostTests` too, so a key
 * renamed on the wire fails here. Nothing reads "today": the dates are the
 * runner's own clock in the fixture, so it holds with `TZ=UTC` and without.
 */
class PlanCostTest {
    private val us = Locale.US
    private val fixture = repositoryFile("test/fixtures/plan.json")

    @Test
    fun `the plan carries a theme's budget and trend, a lane's budget, the week and the comparison`() {
        val plan = Plan.decode(fixture)
        assertEquals(250_000L, plan.themes[0].budgetTokens)
        assertEquals(listOf(0L, 0, 0, 40_000, 120_000, 0, 160_000), plan.themes[0].trendTokens)
        assertEquals(500_000L, plan.lanes[1].budgetTokens)
        assertNull(plan.lanes[0].budgetTokens)
        val cost = plan.cost!!
        assertEquals(34_200_000L, cost.weekTokens)
        assertEquals(2, cost.compareHeldBack)
        assertEquals(listOf(5000, 3400), cost.compare.map { it.cardShareMilli })
        assertEquals(1_200_000L, cost.inFlightTokens)
        assertNull(cost.inFlightCostMicros)
        assertNull("an unpriced pair has no dollars, never zero", cost.compare[1].costMicros)
    }

    @Test
    fun `an older answer from a runner without board_cost reads and draws nothing`() {
        val plan = Plan.decode("""{"now_ms":1,"themes":[{"id":"t","name":"Visual","state":"active","ordinal":1,"counts":{}}],"lanes":[],"order":[],"cards":[]}""")
        assertNull(plan.cost)
        assertNull(plan.themes[0].budgetTokens)
        assertNull(plan.themes[0].trendTokens)
        assertNull(PlanCostWords.trend(plan.themes[0].trendTokens))
        assertNull(PlanCostWords.overBudget(plan.themes[0]))
    }

    @Test
    fun `over budget is past it and equal is within`() {
        val plan = Plan.decode(fixture)
        val over = PlanCostWords.overBudget(plan.themes[0])!!
        assertEquals(PlanBudget.Over(320_000, 250_000), over)
        assertEquals("Over budget: 320K of 250K tokens", PlanCostWords.budgetLine(over, us))
        assertNull("470K of 500K is within", PlanCostWords.overBudget(plan.lanes[1]))
        val within = PlanCostWords.budget(plan.lanes[1].spend, plan.lanes[1].budgetTokens)!!
        assertEquals("470K of 500K tokens budgeted", PlanCostWords.budgetLine(within, us))
        assertEquals(PlanBudget.Within(500, 500), PlanCostWords.budget(PlanSpend(inputTokens = 500), 500))
        assertTrue(PlanCostWords.budget(PlanSpend(inputTokens = 501), 500)!!.isOver)
        assertNull("no budget, nothing to flag", PlanCostWords.budget(PlanSpend(inputTokens = 501), null))
        assertEquals("Over budget: 470K of 100K tokens", PlanCostWords.overBudget(plan.lanes[1].copy(budgetTokens = 100_000), us))
    }

    @Test
    fun `TalkBack hears the counts as words, never a suffix letter`() {
        assertEquals("6.1 million", PlanCostWords.spokenTokens(6_100_000, us))
        assertEquals("340 thousand", PlanCostWords.spokenTokens(340_000, us))
        assertEquals("812", PlanCostWords.spokenTokens(812, us))
        assertEquals(
            "Over budget. 6.1 million of 5 million tokens used.",
            PlanCostWords.budgetSpoken(PlanBudget.Over(6_100_000, 5_000_000), us),
        )
        val trend = PlanCostWords.trend(listOf(0L, 0, 0, 40_000, 120_000, 0, 160_000))!!
        assertEquals(
            "Last 7 days by UTC day, oldest first: none, none, none, 40 thousand, 120 thousand, none, 160 thousand today. 320 thousand tokens in all.",
            PlanCostWords.trendSpoken(trend, us),
        )
    }

    @Test
    fun `a trend is seven days against the busiest and a day with something keeps a sliver`() {
        assertNull(PlanCostWords.trend(null))
        assertNull(PlanCostWords.trend(emptyList()))
        assertNull("a chart of nothing says nothing", PlanCostWords.trend(List(7) { 0L }))
        assertNull("seven days or none", PlanCostWords.trend(listOf(1L, 2, 3)))
        val t = PlanCostWords.trend(listOf(0L, 0, 0, 40_000, 120_000, 0, 160_000))!!
        assertEquals(listOf(0.0, 0.0, 0.0, 0.25, 0.75, 0.0, 1.0), t.heights())
        val tiny = PlanCostWords.trend(listOf(0L, 0, 0, 0, 0, 1, 1_000_000))!!
        assertEquals(0.08, tiny.heights()[5], 0.0)
        assertEquals(0.0, tiny.heights()[0], 0.0)
    }

    @Test
    fun `the week is a count with no percentage and says why`() {
        assertEquals("34M tokens in the last 7 days in this project", PlanCostWords.week(34_200_000, us))
        assertFalse(PlanCostWords.week(34_200_000, us).contains("%"))
        assertFalse(PlanCostWords.WEEK_NOTE.contains("%"))
        assertTrue(PlanCostWords.WEEK_NOTE.contains("weekly limit"))
    }

    @Test
    fun `the week's total leads in tokens and API-equivalent dollars, split by harness and model, and the split adds up`() {
        val cost = Plan.decode(fixture).cost!!
        assertEquals("the parts are the total", cost.weekTokens, cost.week.sumOf { it.tokens })
        assertEquals(cost.weekCostMicros, cost.week.sumOf { it.costMicros ?: 0L })
        assertEquals("about \$45.00 API-equivalent", PlanCostWords.weekDollars(cost, us))
        val rows = cost.week.map { PlanCostWords.weekRow(it, us) }
        assertEquals(listOf("Claude Code · opus", "Codex · gpt-5.6"), rows.map { it.title })
        assertEquals("28M tokens · about \$41.20 API-equivalent", rows[0].detail)
        assertEquals("6.2M tokens · about \$3.80 API-equivalent", rows[1].detail)
        assertTrue(rows[0].spoken.contains("28 million tokens"))
        assertEquals(PlanCostWords.DOLLARS_NOT_REPORTED, PlanCostWords.weekDollars(cost.copy(weekCostMicros = null), us))
        assertNull("an older runner sent no split", PlanCostWords.weekDollars(PlanCostRead(weekTokens = 5), us))
        assertEquals("900 tokens · API-equivalent dollars: Not reported", PlanCostWords.weekRow(PlanWeekSpend("cursor", "", 900), us).detail)
        assertEquals("900 tokens · No price listed for gpt-9", PlanCostWords.weekRow(PlanWeekSpend("codex", "gpt-9", 900), us).detail)
    }

    @Test
    fun `each comparison row says n, tokens a card, and dollars only when every turn was priced`() {
        val cost = Plan.decode(fixture).cost!!
        val claude = PlanCostWords.compareRow(cost.compare[0], us)
        assertEquals("Claude Code · opus", claude.title)
        assertEquals("5 finished cards · 1.5M tokens a card · about $2.50 a card API-equivalent", claude.detail)
        assertTrue(claude.spoken.contains("1.5 million tokens a card") && claude.spoken.contains("API-equivalent"))
        val codex = PlanCostWords.compareRow(cost.compare[1], us)
        assertEquals("Codex · gpt-5.6", codex.title)
        assertEquals("an unpriced model is named, never nothing", "3.4 finished cards · 300K tokens a card · No price listed for gpt-5.6", codex.detail)
        assertEquals("Cursor · no model named", PlanCostWords.compareRow(PlanHarnessCost("cursor", "", 3000, 30), us).title)
        assertEquals("2 other harness and model pairs held back until three cards have landed", PlanCostWords.heldBack(2))
        assertEquals("1 other harness and model pair held back until three cards have landed", PlanCostWords.heldBack(1))
        assertNull(PlanCostWords.heldBack(0))
        assertEquals(3, PlanCostWords.COMPARE_MINIMUM_CARDS)
    }

    @Test
    fun `spend on cards that haven't landed is said apart, with its dollars API-equivalent or Not reported`() {
        val cost = Plan.decode(fixture).cost!!
        assertEquals("1.2M tokens on cards that haven’t landed · API-equivalent dollars: Not reported", PlanCostWords.inFlight(cost, us))
        assertEquals(
            "1.2M tokens on cards that haven’t landed · about \$9.00 API-equivalent",
            PlanCostWords.inFlight(cost.copy(inFlightCostMicros = 9_000_000), us),
        )
        assertNull(PlanCostWords.inFlight(PlanCostRead(), us))
        assertFalse("spend in flight alone is no cost per landed card", PlanCostWords.showsCompareHeading(PlanCostRead(inFlightTokens = 1)))
        assertTrue(PlanCostWords.showsCompareHeading(PlanCostRead(compareHeldBack = 1, inFlightTokens = 1)))
        assertEquals("5 finished cards", PlanCostWords.cardShare(5000))
        assertEquals("3.6 finished cards", PlanCostWords.cardShare(3600))
    }

    @Test
    fun `a harness is named as a person names it, and board_cost is a capability`() {
        assertEquals("Claude Code", PlanCostWords.harnessName("claude"))
        assertEquals("Codex", PlanCostWords.harnessName("codex"))
        assertEquals("Zed", PlanCostWords.harnessName("zed"))
        assertEquals("Unknown", PlanCostWords.harnessName(""))
        assertEquals("board_cost", Capability.BOARD_COST.wire)
        assertFalse(PlanCostRead().isWorthShowing)
        assertTrue(PlanCostRead(weekTokens = 1).isWorthShowing)
        assertTrue(PlanCostRead(compareHeldBack = 1).isWorthShowing)
    }

    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}.")
    }
}
