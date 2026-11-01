package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Landings (ov-309, ov-872) and the runner's CI reads (ov-306) on Android, against
 * `test/fixtures/plan.json`, the Rust client's own output; and a page's live
 * references against `test/fixtures/pages/normalized/live.json`, what the
 * runner stores. AgentKit's `PlanLandingsTests` and `PageLiveDataTests` hold
 * the Mac and the iPhone to the same files and the same words.
 */
class PlanLandingsTest {
    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}.")
    }

    private val plan = Plan.decode(repositoryFile("test/fixtures/plan.json"))

    @Test
    fun `the plan carries landings, CI reads, a theme's spend and the board's counts`() {
        val landing = plan.landings.first()
        assertEquals(9, landing.number)
        assertEquals("Landing 9", landing.label)
        assertEquals(LandingState.RED, landing.state)
        assertEquals(LandingRoute.PUSH, landing.route)
        assertEquals("c85bf83d", landing.pushedSha)
        assertEquals(listOf("ov-4"), landing.tickets.map { it.key })
        assertEquals("00000000-0000-0000-0000-000000007004", landing.lander)
        assertEquals(listOf(9, 8), plan.landings.map { it.number })
        assertEquals("Agents message the plan", plan.landings[1].title)
        val read = assertNotNullOf(plan.ciOf(landing))
        assertEquals(CiStatus.FAILED, read.status)
        assertEquals(3, read.jobs.size)
        assertEquals(320_000L, plan.themes.single().spend?.totalTokens)
        assertEquals(9, plan.cardCount("open"))
        assertEquals(3, plan.cardCount("in_review"))
    }

    @Test
    fun `a landing is its tickets' home while it is live, and a ticket says it's in it`() {
        val card = PlanSections.of(plan).landings.single()
        assertEquals(listOf("ov-4"), card.tickets.map { it.key })
        assertEquals(LandingPhase.CI_FAILED, card.phase)
        assertTrue(LandingWords.needsAttention(card.landing, null))
        assertEquals("Landing · in Landing 9", PlanWords.status(plan.tickets[2]))
        assertEquals(listOf(plan.tickets[2]), plan.ticketsOn(card.landing))
    }

    @Test
    fun `a read that stopped working says how old it is`() {
        val now = 1_800_000_000_000L
        val read = PlanCiRead("main", status = CiStatus.PASSED, fetchedAt = now - 3 * 3_600_000L)
        assertEquals("as of 3 h ago", LandingWords.ciStale(read, now))
        assertNull(LandingWords.ciStale(read.copy(fetchedAt = now - 60_000), now))
        val world = PageWorld(plan = Plan(nowMs = now, ci = listOf(read)))
        assertEquals("Passed · as of 3 h ago", world.resolve(PageRef(PageTarget.Ci("main"))).status)
    }

    @Test
    fun `an answer with no landings still reads`() {
        val old = Plan.decode("""{"now_ms": 1, "themes": [], "lines": [], "cards": []}""")
        assertTrue(old.landings.isEmpty() && old.ci.isEmpty() && old.tickets.isEmpty() && old.agents.isEmpty())
        assertNull(old.boardCounts)
        assertNull(old.cardCount("open"))
    }

    @Test
    fun `CI words, in sentence case`() {
        val jobs = listOf(PlanCiJob("a", "running"), PlanCiJob("b", "passed"))
        assertEquals("Running · 1 of 2 jobs done", LandingWords.ciSummary(PlanCiRead("main", status = CiStatus.RUNNING, jobs = jobs)))
        assertEquals("No runs yet", LandingWords.ciSummary(PlanCiRead("main", status = CiStatus.NONE)))
        assertEquals("CI unknown", LandingWords.ciSummary(PlanCiRead("main", status = CiStatus.UNKNOWN)))
    }

    private val live = PageDoc.decode(repositoryFile("test/fixtures/pages/normalized/live.json"))

    @Test
    fun `figures on the live page draw from the plan, failed CI in amber`() {
        val stats = live.blocks[1] as PageBlock.Stats
        // The runner gives a live figure the value and the reference the label an older app draws (review 1005c M4).
        assertEquals(PageRef(PageTarget.Ci("main"), "Main"), stats.items[0].ref)
        assertEquals("Main", stats.items[0].value)
        val world = PageWorld(plan = plan)
        val shown = stats.items.map(world::statText)
        // Main isn't read in the fixture: its name, as plain text.
        assertEquals("Main", shown[0].value)
        assertEquals(PageWorld.StatShown("Failed", "pushed at midnight", PageTone.ATTENTION), shown[1])
        assertEquals("3", shown[2].value)
        assertEquals("9", shown[3].value)
        assertEquals("320K", shown[4].value)
        // The runner no longer knows lanes: a page's `lane` reference to a name no ticket has draws as the plain word.
        assertEquals("mac-ux", shown[5].value)
    }

    @Test
    fun `a CI reference opens its run and says how it stands, and a count is a number`() {
        val world = PageWorld(plan = plan)
        val ci = world.resolve(PageRef(PageTarget.Ci("c85bf83d")))
        assertEquals("c85bf83d", ci.name)
        assertEquals("Failed · 1 of 3 jobs failed", ci.status)
        assertEquals(PageTone.ATTENTION, ci.statusTone)
        assertEquals(PageDestination.Url("https://github.com/taneliang/farcooler/actions/runs/37275435256"), ci.destination)
        assertEquals("11", world.cellText(PageCell(ref = PageRef(PageTarget.Cards("done")))))
        assertEquals("320K", world.cellText(PageCell(ref = PageRef(PageTarget.Theme("Visual language")), show = PageShow.SPEND)))
        val bare = PageWorld()
        assertEquals(PageResolved("Run 37275435256"), bare.resolve(PageRef(PageTarget.Ci("run:37275435256"))))
        assertEquals(PageResolved("In review"), bare.resolve(PageRef(PageTarget.Cards("in_review"))))
        assertFalse(bare.resolve(PageRef(PageTarget.Ci("main"))).destination != null)
    }

    private fun <T> assertNotNullOf(value: T?): T {
        assertNotNull(value)
        return value!!
    }
}
