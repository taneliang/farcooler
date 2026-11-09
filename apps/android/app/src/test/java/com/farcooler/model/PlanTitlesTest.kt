package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Tickets and landings named by what they do (ov-462, ov-872), and a landing's own lander (ov-461), against
 * `test/fixtures/plan.json`. AgentKit's `PlanTitlesTests` holds the Mac and the iPhone to the same file and words.
 */
class PlanTitlesTest {
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
    fun `a ticket shows its card's title, and a card with none shows its key`() {
        assertEquals(listOf("Title of ov-2", "Title of ov-1", "Title of ov-4", "Title of ov-3"), plan.tickets.map { it.heading })
        assertEquals("ov-7", plan.tickets[0].copy(title = "", key = "ov-7").heading)
    }

    @Test
    fun `a landing is Landing N, with the agent landing it and its tickets' titles on one line`() {
        val landing = plan.landings.first()
        assertEquals("Landing 9", landing.label)
        assertEquals("Title of ov-4", landing.summary)
        assertEquals("00000000-0000-0000-0000-000000007004", landing.lander)
        assertEquals("Landing 72", PlanLanding("i", 72, "", null, LandingState.GATING, 0, tickets = emptyList(), ciSubject = "").label)
    }

    @Test
    fun `a lander is told what it landed and what it spent`() {
        val lander = PlanAgent(
            "i1", AgentRole.LANDER, "claude", "claude-haiku-4", "", null, "i1", emptyList(), true, "", null, 0, 0,
            PlanSpend(inputTokens = 120_000, runs = 1),
        )
        assertTrue(lander.working)
        assertEquals("Lander working \u00b7 120K tokens", LandingWords.lander(lander))
        assertEquals("Lander finished \u00b7 120K tokens", LandingWords.lander(lander.copy(working = false)))
        assertEquals("Lander finished", LandingWords.lander(lander.copy(working = false, spend = PlanSpend())))
    }
}
