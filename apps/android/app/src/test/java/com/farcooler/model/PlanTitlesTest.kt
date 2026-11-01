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
    fun `a landing reads Landing N, what it carries, and the agent landing it`() {
        val landing = plan.landings.first()
        assertEquals("Landing 9", landing.label)
        assertNull("its title is its label already", landing.subtitle)
        assertEquals("Title of ov-4", landing.carries)
        assertEquals("00000000-0000-0000-0000-000000007004", landing.lander)
        assertEquals("a landing the plan lists no agent for has no lander to name", null, PlanSections.of(plan).landings.first().lander)
        val earlier = plan.landings[1]
        assertEquals("Landing 8", earlier.label)
        assertEquals("Agents message the plan", earlier.subtitle)
    }

    @Test
    fun `a landing's title is its subtitle, never its name, and a number is all its name needs`() {
        val bare = PlanLanding("i", 72, "", null, LandingState.GATING, 0, tickets = emptyList(), ciSubject = "")
        assertEquals("Landing 72", bare.label)
        assertNull(bare.subtitle)
        val titled = bare.copy(number = 3, title = "Phones catch up")
        assertEquals("Landing 3", titled.label)
        assertEquals("the title is the subtitle, never the name", "Phones catch up", titled.subtitle)
        assertEquals("a title that repeats the label says no more", null, bare.copy(title = "Landing 72").subtitle)
        assertEquals("the runner's own prefix is not said twice", "Phones", bare.copy(title = "Landing 72 \u00b7 Phones").subtitle)
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
