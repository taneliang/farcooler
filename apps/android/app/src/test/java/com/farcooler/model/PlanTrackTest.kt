package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import org.junit.Test
import java.io.File

/**
 * The track line on Android (ov-331): the same phrases and rules as AgentKit's `PlanThemeTrackTests`, so the
 * three platforms say the same thing about the same theme.
 */
class PlanTrackTest {
    private val now = 1_800_000_000_000L
    private val hour = 3_600_000L
    private val day = 24 * hour

    private fun counts(todo: Int = 1, done: Int = 0) = PlanCounts(0, todo, 0, 0, 0, done, 0)

    private fun theme(
        state: String = "active", counts: PlanCounts = counts(), storyAt: Long = 0, story: String = "", lastMovedAt: Long? = null,
        budget: Long? = null, spent: Long = 0,
    ) = PlanTheme(
        id = "t", name = "T", outcome = "", story = story, storyAt = storyAt, next = "", ownerAsk = "", state = state, ordinal = 0,
        cards = listOf(PlanCardRef("c1", "ov-1"), PlanCardRef("c2", "ov-2")), counts = counts,
        spend = PlanSpend(inputTokens = spent), budgetTokens = budget, lastMovedAt = lastMovedAt,
    )

    private fun lane(
        name: String, state: LaneState, rank: Int? = null, since: Long = now - 10 * 60_000, rounds: Int = 0, stale: Boolean = false,
    ) = PlanLane(
        id = "lane-$name", name = name, state = state, reason = "", planRank = rank, worktreePath = "", branch = name,
        harness = "claude", model = "sonnet", train = null, landedSha = null, stateSince = since, stale = stale,
        fixRounds = rounds, cards = listOf(PlanCardRef("c1", "ov-1")), agents = emptyList(), spend = PlanSpend(),
    )

    private fun plan(theme: PlanTheme, vararg lanes: PlanLane) = Plan(nowMs = now, themes = listOf(theme), lanes = lanes.toList())
    private fun words(plan: Plan) = PlanWords.track(plan.track(plan.themes[0]), now)

    @Test
    fun `moving names one lane with its state and counts several`() {
        assertEquals("skill-323 is building", words(plan(theme(), lane("skill-323", LaneState.BUILDING))))
        assertEquals("a is in review", words(plan(theme(), lane("a", LaneState.REVIEW))))
        assertEquals("fix-gestures is fixing, round 1", words(plan(theme(), lane("fix-gestures", LaneState.FIXING, rounds = 1))))
        assertEquals("a is fixing", words(plan(theme(), lane("a", LaneState.FIXING))))
        assertEquals("a is landing", words(plan(theme(), lane("a", LaneState.LANDING))))
        assertEquals("2 lanes moving", words(plan(theme(), lane("a", LaneState.BUILDING), lane("b", LaneState.REVIEW))))
    }

    @Test
    fun `a lone stale lane reads stuck, and among moving ones it is named after them`() {
        assertEquals("s: no move in an hour", words(plan(theme(), lane("s", LaneState.BUILDING, since = now - 70 * 60_000, stale = true))))
        assertEquals("s: no move in 3 h", words(plan(theme(), lane("s", LaneState.BUILDING, since = now - 3 * hour, stale = true))))
        assertEquals(
            "2 lanes moving · slow no move in 4 h",
            words(plan(theme(), lane("fine", LaneState.BUILDING), lane("slow", LaneState.REVIEW, since = now - 4 * hour, stale = true))),
        )
    }

    @Test
    fun `queued says the best rank, and a working lane beats it`() {
        assertEquals("Queued, 1st up", words(plan(theme(), lane("a", LaneState.QUEUED, rank = 1))))
        assertEquals("Queued, 2nd up", words(plan(theme(), lane("a", LaneState.QUEUED, rank = 3), lane("b", LaneState.QUEUED, rank = 2))))
        assertEquals("Queued", words(plan(theme(), lane("a", LaneState.QUEUED))))
        assertEquals("b is building", words(plan(theme(), lane("a", LaneState.QUEUED, rank = 1), lane("b", LaneState.BUILDING))))
    }

    @Test
    fun `quiet after a day with open cards and no lane, idle before it`() {
        assertEquals("No lane · quiet for 3 days", words(plan(theme(storyAt = now - 3 * day - hour))))
        assertEquals("No lane · quiet for 1 day", words(plan(theme(storyAt = now - 25 * hour))))
        assertTrue(plan(theme(storyAt = now - day)).let { it.track(it.themes[0]).isQuiet })
        assertEquals("No lane yet", words(plan(theme(storyAt = now - 23 * hour))))
        assertEquals("No lane yet", words(plan(theme(storyAt = 0))))
    }

    @Test
    fun `the runner's last_moved_at keeps a theme awake, and a dropped lane doesn't`() {
        val old = now - 5 * day
        assertTrue(plan(theme(storyAt = old)).let { it.track(it.themes[0]).isQuiet })
        assertEquals(PlanTrack.Idle, plan(theme(storyAt = old, lastMovedAt = now - hour)).let { it.track(it.themes[0]) })
        assertTrue(plan(theme(storyAt = old), lane("a", LaneState.DROPPED, since = now - hour)).let { it.track(it.themes[0]).isQuiet })
        assertEquals(PlanTrack.Idle, plan(theme(storyAt = old), lane("a", LaneState.LANDED, since = now - hour)).let { it.track(it.themes[0]) })
    }

    @Test
    fun `all done, paused and done`() {
        assertEquals("Every card is done", words(plan(theme(counts = counts(todo = 0, done = 2)))))
        assertEquals("No lane yet", words(plan(theme(counts = counts(todo = 0, done = 0)))))
        assertEquals("Paused", words(plan(theme(state = "paused"), lane("a", LaneState.BUILDING))))
        assertEquals("Done", words(plan(theme(state = "done"), lane("a", LaneState.BUILDING))))
    }

    @Test
    fun `over budget is the one amber state and wins over a moving lane`() {
        val over = plan(theme(budget = 100, spent = 600), lane("a", LaneState.BUILDING))
        val track = over.track(over.themes[0])
        assertTrue(track.needsAttention)
        assertEquals("Over budget: 600 of 100 tokens", PlanWords.track(track, now))
        assertFalse(plan(theme(budget = 1000, spent = 600), lane("a", LaneState.BUILDING)).let { it.track(it.themes[0]).needsAttention })
        for (other in listOf(PlanTrack.Paused, PlanTrack.Done, PlanTrack.Idle, PlanTrack.AllDone, PlanTrack.Queued(1), PlanTrack.Quiet(1))) {
            assertFalse("$other is never amber", other.needsAttention)
        }
    }

    @Test
    fun `spoken, the middle dot is a comma, and the story's age is printed only with a story`() {
        assertEquals("No lane, quiet for 3 days", PlanWords.trackSpoken(PlanTrack.Quiet(now - 3 * day), now))
        assertEquals("Updated 3 h ago", PlanWords.storyAge(theme(storyAt = now - 3 * hour, story = "Going well."), now))
        assertNull(PlanWords.storyAge(theme(storyAt = now - hour), now))
    }

    @Test
    fun `the summary counts asks, moving themes and quiet ones, leaving out zeros`() {
        fun t(id: String, state: String, ask: String, card: String) = theme(state = state, storyAt = now - 5 * day).copy(
            id = id, name = id, ownerAsk = ask, cards = listOf(PlanCardRef(card, card)),
        )
        val moving = lane("w", LaneState.BUILDING).copy(cards = listOf(PlanCardRef("c0", "c0")))
        val plan = Plan(
            nowMs = now,
            themes = listOf(t("A", "active", "?", "c0"), t("B", "active", "", "c1"), t("C", "active", "?", "c2"), t("D", "paused", "", "c3")),
            lanes = listOf(moving),
        )
        assertEquals("2 waiting on you · 1 moving · 2 quiet", plan.trackSummary())
        assertNull(Plan(nowMs = now, themes = listOf(theme(counts = counts(todo = 0, done = 2)))).trackSummary())
    }

    @Test
    fun `last_moved_at decodes from the wire, and its absence leaves null`() {
        val json = """{"now_ms": 1, "themes": [{"id": "t", "name": "T", "last_moved_at": 42}, {"id": "u", "name": "U"}], "lanes": [], "order": [], "cards": []}"""
        val plan = Plan.decode(json)
        assertEquals(42L, plan.themes[0].lastMovedAt)
        assertNull(plan.themes[1].lastMovedAt)
        // The runner's own bytes (test/fixtures/plan.json, which the CLI's test writes).
        assertEquals(now - hour, Plan.decode(repositoryFile("test/fixtures/plan.json")).themes[0].lastMovedAt)
    }

    /** The cases `PlanThemeTrackTests` (Swift) reads too, so a phrase or a threshold can't change on one platform alone. */
    @Test
    fun `every case in plan-track-cases json reads as written`() {
        val cases = Json.parseToJsonElement(repositoryFile("test/fixtures/plan-track-cases.json")).jsonObject["cases"]!!.jsonArray
        assertTrue(cases.size >= 30)
        for (item in cases) {
            val c = item.jsonObject
            val name = (c["name"] as JsonPrimitive).content
            val plan = Plan.decode(c["plan"]!!.jsonObject)
            (c["track"] as? JsonPrimitive)?.takeIf { it !is JsonNull }?.let { assertEquals(name, it.content, words(plan)) }
            if (c.containsKey("summary")) {
                val want = (c["summary"] as? JsonPrimitive)?.takeIf { it !is JsonNull }?.content
                assertEquals("$name: the summary", want, plan.trackSummary())
            }
        }
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
