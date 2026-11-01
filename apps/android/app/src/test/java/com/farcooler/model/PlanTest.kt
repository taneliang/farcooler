package com.farcooler.model

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.util.TimeZone

/**
 * The plan layer on Android (ov-274), against the bytes the Rust client makes of
 * a plan: `test/fixtures/plan.json` is `plan_json`'s output, held there by the
 * client crate's own test and the CLI's, and AgentKit's `PlanModelTests` read
 * the same file, so a key renamed on the wire fails here.
 */
class PlanTest {
    private val fixture = repositoryFile("test/fixtures/plan.json")
    private val now = 1_800_000_000_000L
    private val hour = 3_600_000L

    @Test
    fun `the Rust client's plan decodes whole`() {
        val plan = Plan.decode(fixture)
        assertEquals(now, plan.nowMs)
        val theme = plan.themes.single()
        assertEquals("Visual language", theme.name)
        assertEquals("Should the sidebar tint follow the terminal theme?", theme.ownerAsk)
        assertEquals(listOf("ov-1", "ov-2", "ov-3"), theme.cards.map { it.key })
        assertEquals(1, theme.counts.done)
        assertEquals(2, theme.counts.backlog)
        assertEquals(listOf("ov-2", "ov-1", "ov-4", "ov-3"), plan.tickets.map { it.key })
        assertEquals(listOf(TicketStage.WAITING, TicketStage.REVIEWING, TicketStage.LANDING, TicketStage.LANDED), plan.tickets.map { it.stage })
        val review = plan.tickets[1]
        assertEquals(1, review.fixRounds)
        assertEquals("Title of ov-1", review.title)
        assertEquals("In Review", review.status)
        assertEquals("ov-1-mac-polish", review.branch)
        assertEquals(listOf(WorkRole.BUILD, WorkRole.REVIEW), review.work.map { it.role })
        assertEquals("finished", review.work[0].verdict)
        assertEquals(now - hour, review.work[0].endedAt)
        assertNull(review.work[1].endedAt)
        assertEquals(470_000L, review.spend.totalTokens)
        assertEquals(31_000_000L, review.spend.costMicros)
        assertEquals(500_000L, review.budgetTokens)
        assertEquals(9, plan.tickets[2].landing)
        assertEquals(31, plan.tickets[2].prNumber)
        assertEquals("ov-1", plan.tickets[2].stackParent?.key)
        assertEquals("4d3c8cb1e2", plan.tickets[3].landedSha)
        assertEquals(listOf("ov-2"), plan.nextUp.map { it.key })
        assertEquals(listOf("ov-1", "ov-2", "ov-3", "ov-4", "ov-5"), plan.cards.map { it.key })
    }

    @Test
    fun `the agents carry their tickets and what they are doing now`() {
        val plan = Plan.decode(fixture)
        assertEquals(listOf(AgentRole.ORCHESTRATOR, AgentRole.REVIEWER, AgentRole.BUILDER), plan.agents.map { it.role })
        val reviewer = plan.agents[1]
        assertEquals("Reading PlanColumn.swift", reviewer.doingNow)
        assertEquals("Review ov-1", reviewer.title)
        assertEquals(listOf("ov-1"), reviewer.tickets.map { it.key })
        assertEquals(WorkRole.REVIEW, reviewer.tickets.single().role)
        assertEquals("a3", reviewer.subagentId)
        assertNull(reviewer.terminal)
        val builder = plan.agents[2]
        assertFalse(builder.working)
        assertEquals("gpt-5.6", builder.model)
        assertEquals(1_799_999_940_000L, builder.stopAskedAt)
        assertEquals("Stopping", builder.state(now))
        assertEquals("2/5 · Reviewing the plan", plan.agents[0].doingNow)
        assertEquals(listOf(reviewer), plan.agentsOn(plan.tickets[1]))
    }

    @Test
    fun `the seeded board the captures draw decodes, with a record for every theme and ticket`() {
        val seeded = Json.parseToJsonElement(repositoryFile("test/fixtures/plan-seeded.json")).jsonObject
        val plan = Plan.decode(seeded["plan"]!!.jsonObject)
        val records = seeded["records"]!!.jsonObject
        assertEquals(5, plan.shownThemes.size)
        assertEquals(5, plan.nextUp.size)
        for (id in plan.themes.map { it.id } + plan.tickets.map { it.task }) {
            PlanRecord.decode(records[id]!!.jsonObject)
        }
    }

    @Test
    fun `next up is the plan's rank and now is every live ticket past waiting`() {
        val plan = Plan.decode(fixture)
        assertEquals(listOf("ov-2"), plan.nextUp.map { it.key })
        assertEquals(listOf("ov-1", "ov-4"), plan.working.map { it.key })
        assertTrue(plan.unranked.isEmpty())
        assertEquals(listOf("ov-3"), plan.landedToday(TimeZone.getTimeZone("UTC")).map { it.key })
        val unranked = plan.copy(tickets = plan.tickets.map { it.copy(planRank = null) })
        assertEquals(listOf("ov-2"), unranked.unranked.map { it.key })
        assertTrue(unranked.nextUp.isEmpty())
    }

    @Test
    fun `a ticket serves the theme its card is in, and a card elsewhere serves none`() {
        val plan = Plan.decode(fixture)
        assertEquals("Visual language", plan.themeOf(plan.tickets[0])?.name)
        assertNull(plan.themeOf(plan.tickets[0].copy(task = "elsewhere")))
        assertEquals(listOf("ov-1", "ov-2", "ov-3"), plan.ticketsIn(plan.themes.single()).map { it.key })
    }

    @Test
    fun `progress leaves canceled cards out`() {
        assertEquals("4 of 18 done", PlanWords.progress(PlanCounts(done = 4, backlog = 14, cancelled = 5)))
        assertEquals(18, PlanWords.total(PlanCounts(done = 4, backlog = 14, cancelled = 5)))
        assertFalse(PlanWords.breakdown(PlanCounts(done = 1, cancelled = 3)).contains("3"))
    }

    @Test
    fun `an outcome gets three lines`() {
        assertEquals(3, PlanWords.OUTCOME_LINES)
    }

    @Test
    fun `a ticket's status says its round, its place, its commit and its landing`() {
        val plan = Plan.decode(fixture)
        assertEquals("Waiting · 1st", PlanWords.status(plan.tickets[0]))
        assertEquals("In review", PlanWords.status(plan.tickets[1]))
        assertEquals("Landing · in Landing 9", PlanWords.status(plan.tickets[2]))
        assertEquals("Landed · 4d3c8cb1", PlanWords.status(plan.tickets[3]))
        val fixing = plan.tickets[1].copy(
            stage = TicketStage.BUILDING, fixRounds = 2, landing = null,
            work = listOf(PlanWork("a", WorkRole.FIX, now - hour, null)),
        )
        assertEquals("Fixing · round 2", PlanWords.status(fixing))
        assertEquals(listOf("1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st"), listOf(1, 2, 3, 4, 11, 12, 13, 21).map(PlanWords::ordinal))
    }

    @Test
    fun `a stage this build has no word for is unknown, not a failed read`() {
        assertEquals(TicketStage.UNKNOWN, TicketStage.parse("teleporting"))
        assertEquals(TicketStage.UNKNOWN, TicketStage.parse(null))
        assertEquals(WorkRole.UNKNOWN, WorkRole.parse("juggle"))
    }

    @Test
    fun `only a stale ticket or one waiting on the owner is a warning`() {
        val plan = Plan.decode(fixture)
        val review = plan.tickets[1]
        assertNull(PlanWords.stale(review, now))
        assertEquals("No move in 3 h", PlanWords.stale(review.copy(stale = true, stageSince = now - 3 * hour), now))
        val statuses = mapOf(review.task to TaskStatus.NEEDS_DECISION)
        assertTrue(plan.waitsOnOwner(review, statuses))
        assertTrue(plan.waitsOnOwner(review.copy(stage = TicketStage.NEEDS_YOU), emptyMap()))
        assertFalse(plan.waitsOnOwner(plan.tickets[3], mapOf(plan.tickets[3].task to TaskStatus.NEEDS_DECISION)))
    }

    @Test
    fun `spend is tokens first, a price is an estimate, and nothing measured is not reported`() {
        val review = Plan.decode(fixture).tickets[1]
        val dollars = TaskUsageFormat.dollars(31_000_000, java.util.Locale.US)
        assertEquals(
            "470K tokens · about $dollars estimated · 1 agent not reported · 1 agent's spend split with other tickets",
            PlanWords.spend(review.spend, java.util.Locale.US),
        )
        assertEquals("Not reported", PlanWords.spend(PlanSpend()))
    }

    @Test
    fun `a work record says its role, the agent's model and how long it ran`() {
        val work = PlanWork("a", WorkRole.BUILD, 0, 3 * hour)
        assertEquals("GPT-6.1 Sol", PlanWords.model("gpt-6.1-sol"))
        assertEquals("GPT-6.1 Terra", PlanWords.model("gpt-6.1-terra"))
        assertEquals("GPT-6 Luna", PlanWords.model("gpt-6-luna"))
        assertEquals("Gemini 2.5 Pro", PlanWords.model("gemini-2.5-pro"))
        assertEquals("claude-opus-5-5-20261101", PlanWords.model("claude-opus-5-5-20261101"))
        assertEquals("Builder Claude Opus 5.5, 3 h", PlanWords.work(work, now, "claude-opus-5-5"))
        assertEquals("Reviewer, 3 h", PlanWords.work(work.copy(role = WorkRole.REVIEW), now))
        assertEquals("Fixer sonnet, 3 h", PlanWords.work(work.copy(role = WorkRole.FIX), now, "sonnet"))
    }

    @Test
    fun `a run of cards added in one write is one line, and a story rewrite is said once`() {
        val record = PlanRecord(
            listOf(
                PlanEvent(1_000, "manager", "cards", "Added ov-1."),
                PlanEvent(2_000, "manager", "cards", "Added ov-2."),
                PlanEvent(3_000, "manager", "cards", "Added ov-3."),
                PlanEvent(4_000, "manager", "story", "The old story."),
                PlanEvent(500_000, "manager", "state", "Moved to review."),
            ),
        )
        assertEquals(listOf("Added ov-1, ov-2 and ov-3.", "Rewrote where it stands.", "Moved to review."), record.timeline.map { it.text })
        assertEquals("The old story." to 4_000L, record.previousStory)
        assertNull(PlanRecord(emptyList()).previousStory)
    }

    @Test
    fun `a plan notice names its board, and no other line is plan news`() {
        val line = Json.parseToJsonElement("""{"event":"plan","workspace":"0192f3a4-0000-7000-8000-000000000001"}""").jsonObject
        assertEquals("0192f3a4-0000-7000-8000-000000000001", PlanNews.board(line))
        assertNull(PlanNews.board(Json.parseToJsonElement("""{"event":"task","workspace":"w"}""").jsonObject))
        assertNull(PlanNews.board(Json.parseToJsonElement("""{"event":"plan"}""").jsonObject))
    }

    @Test
    fun `Tasks is the default, and a runner without the layer draws tasks whatever was chosen`() {
        assertFalse(PlanChoice.showing(runnerKeepsPlan = true, chosen = false))
        assertTrue(PlanChoice.showing(runnerKeepsPlan = true, chosen = true))
        assertFalse(PlanChoice.showing(runnerKeepsPlan = false, chosen = true))
    }

    @Test
    fun `every read lands in a state, and one nobody answers is reconnecting, never loading for good`() = runBlocking {
        val data = fixture
        assertTrue(PlanReadState.read(runnerCan = true) { data } is PlanReadState.Loaded)
        assertEquals(PlanReadState.NeedsUpdate, PlanReadState.read(runnerCan = false) { error("asked") })
        assertEquals(PlanReadState.Unavailable, PlanReadState.read(runnerCan = true) { "not json" })
        assertEquals(PlanReadState.Unavailable, PlanReadState.read(runnerCan = null) { throw RuntimeException("refused") })
        class Unsupported : Exception()
        assertEquals(PlanReadState.NeedsUpdate, PlanReadState.read(runnerCan = null, isUnsupported = { it is Unsupported }) { throw Unsupported() })
        val started = System.currentTimeMillis()
        val hung = PlanReadState.read(runnerCan = true, timeoutMs = 80) {
            kotlinx.coroutines.delay(60_000)
            data
        }
        assertEquals(PlanReadState.Reconnecting, hung)
        assertTrue("it waited out the runner", System.currentTimeMillis() - started < 5_000)
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
