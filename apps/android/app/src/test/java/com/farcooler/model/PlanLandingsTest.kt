package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Landings (ov-309, ov-872) and the runner's CI reads (ov-306) on Android, against
 * `test/fixtures/plan.json`, the Rust client's own output.
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
    fun `a CI run link opens only over HTTPS with a plain domain and no credentials`() {
        val run = "https://github.com/o/r/actions/runs/1"
        val read = PlanCiRead("main", status = CiStatus.PASSED, url = run)
        assertEquals(run, read.runUrl)

        val rejected = listOf(
            "http://github.com/",
            "javascript:alert(1)",
            "https://user@github.com/",
            "https://user:pw@github.com/",
            "file:///etc/passwd",
            "https://gіthub.com/", // Cyrillic і
            "/relative",
            "https://",
        )
        for (url in rejected) {
            assertNull("Rejected URL: $url", read.copy(url = url).runUrl)
        }
    }

    @Test
    fun `a landing is its tickets' home while it is live, and a ticket says it's in it`() {
        val landing = plan.landings.single { !it.state.isSettled }
        assertEquals(listOf("ov-4"), landing.tickets.map { it.key })
        assertTrue(LandingWords.needsAttention(landing, null))
        assertEquals("Landing · in Landing 9", PlanViews.status(plan, plan.tickets[2]))
    }

    @Test
    fun `a read that stopped working says how old it is`() {
        val now = 1_800_000_000_000L
        val read = PlanCiRead("main", status = CiStatus.PASSED, fetchedAt = now - 3 * 3_600_000L)
        assertEquals("as of 3 h ago", LandingWords.ciStale(read, now))
        assertNull(LandingWords.ciStale(read.copy(fetchedAt = now - 60_000), now))
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

    private fun <T> assertNotNullOf(value: T?): T {
        assertNotNull(value)
        return value!!
    }
}
