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
        assertEquals(listOf(LaneState.QUEUED, LaneState.REVIEW, LaneState.LANDED), plan.lanes.map { it.state })
        val review = plan.lanes[1]
        assertEquals("integ-9", review.train)
        assertEquals(1, review.fixRounds)
        assertEquals("Mac", review.cards.single().slice)
        assertEquals(listOf("build", "review"), review.agents.map { it.role })
        assertEquals(now - hour, review.agents[0].endedAt)
        assertNull(review.agents[1].endedAt)
        assertEquals(470_000L, review.spend.totalTokens)
        assertEquals(31_000_000L, review.spend.costMicros)
        assertEquals("4d3c8cb1e2", plan.lanes[2].landedSha)
        assertEquals(listOf("mac-fu3"), plan.nextUp.map { it.name })
        assertEquals(listOf("ov-1", "ov-2", "ov-3"), plan.cards.map { it.key })
    }

    @Test
    fun `the seeded board the captures draw decodes, with a record for every theme and lane`() {
        val seeded = Json.parseToJsonElement(repositoryFile("test/fixtures/plan-seeded.json")).jsonObject
        val plan = Plan.decode(seeded["plan"]!!.jsonObject)
        val records = seeded["records"]!!.jsonObject
        assertEquals(5, plan.shownThemes.size)
        assertEquals(4, plan.nextUp.size)
        for (id in plan.themes.map { it.id } + plan.lanes.map { it.id }) {
            PlanRecord.decode(records[id]!!.jsonObject)
        }
    }

    @Test
    fun `next up is the plan's order and now is every live lane past queued`() {
        val plan = Plan.decode(fixture)
        assertEquals(listOf("mac-fu3"), plan.nextUp.map { it.name })
        assertEquals(listOf("mac-ux"), plan.working.map { it.name })
        assertTrue(plan.unranked.isEmpty())
        assertEquals(listOf("fix-ac84"), plan.landedToday(TimeZone.getTimeZone("UTC")).map { it.name })
        assertEquals(plan.copy(order = emptyList()).unranked.map { it.name }, listOf("mac-fu3"))
    }

    @Test
    fun `a lane serves the theme most of its cards are in, and each lane names it`() {
        val plan = Plan.decode(fixture)
        assertEquals("Visual language", plan.themeOf(plan.lanes[0])?.name)
        assertNull(plan.themeOf(plan.lanes[0].copy(cards = listOf(PlanCardRef("elsewhere", "ov-9")))))
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
    fun `a lane's status says its round, its place, its commit and its train`() {
        val plan = Plan.decode(fixture)
        assertEquals("Queued · 1st", PlanWords.status(plan.lanes[0]))
        assertEquals("In review · in integ-9", PlanWords.status(plan.lanes[1]))
        assertEquals("Landed · 4d3c8cb1", PlanWords.status(plan.lanes[2]))
        assertEquals("Fixing · round 2", PlanWords.status(plan.lanes[1].copy(state = LaneState.FIXING, fixRounds = 2, train = null)))
        assertEquals(listOf("1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st"), listOf(1, 2, 3, 4, 11, 12, 13, 21).map(PlanWords::ordinal))
    }

    @Test
    fun `a state this build has no word for is unknown, not a failed read`() {
        assertEquals(LaneState.UNKNOWN, LaneState.parse("teleporting"))
        assertEquals(LaneState.UNKNOWN, LaneState.parse(null))
    }

    @Test
    fun `only a stale lane or one waiting on the owner is a warning`() {
        val plan = Plan.decode(fixture)
        val review = plan.lanes[1]
        assertNull(PlanWords.stale(review, now))
        assertEquals("No move in 3 h", PlanWords.stale(review.copy(stale = true, stateSince = now - 3 * hour), now))
        val statuses = mapOf(review.cards[0].task to TaskStatus.NEEDS_DECISION)
        assertTrue(plan.waitsOnOwner(review, statuses))
        assertFalse(plan.waitsOnOwner(plan.lanes[2], mapOf(plan.lanes[2].cards[0].task to TaskStatus.NEEDS_DECISION)))
    }

    @Test
    fun `spend is tokens first, a price is an estimate, and nothing measured is not reported`() {
        val review = Plan.decode(fixture).lanes[1]
        val dollars = TaskUsageFormat.dollars(31_000_000, java.util.Locale.US)
        assertEquals(
            "470K tokens · about $dollars estimated · 1 agent not reported · 1 agent's spend split with other lanes",
            PlanWords.spend(review.spend, java.util.Locale.US),
        )
        assertEquals("Not reported", PlanWords.spend(PlanSpend()))
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
    fun `every read lands in a state, and one nobody answers is unavailable, never loading for good`() = runBlocking {
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
        assertEquals(PlanReadState.Unavailable, hung)
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
