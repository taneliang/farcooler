package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Decided for you (ov-304) on Android, against the bytes the Rust client makes
 * of a plan: `test/fixtures/plan.json`'s `rulings`, held there by the client
 * crate's test (from wire bytes) and the CLI's, and read by AgentKit's
 * `PlanRulingsTests` too.
 */
class PlanRulingsTest {
    private val fixture = repositoryFile("test/fixtures/plan.json")

    @Test
    fun `the fixture's rulings read standing first, with every part a row draws`() {
        val plan = Plan.decode(fixture)
        assertEquals(listOf("R-2", "R-1"), plan.rulings.map { it.short })
        val standing = plan.standingRulings.single()
        assertEquals("The inbox is amber.", standing.decision)
        assertEquals("It's the one attention color, so the inbox reads as needing you.", standing.why)
        assertEquals("One token; every surface follows.", standing.reversal)
        assertEquals(listOf("ov-1"), standing.cards.map { it.key })
        assertEquals("Visual language", standing.theme)
        assertNull(standing.settledAt)
        val settled = plan.settledRulings.single()
        assertEquals(RulingState.CONFIRMED, settled.state)
        assertEquals("Keep it.", settled.note)
    }

    @Test
    fun `Copy reference copies what the owner tells the orchestrator`() {
        val plan = Plan.decode(fixture)
        assertEquals("ruling R-2: The inbox is amber.", plan.rulings[0].reference)
    }

    @Test
    fun `what a ruling touches, and nothing when it touches nothing`() {
        val plan = Plan.decode(fixture)
        assertEquals("ov-1 · Visual language", RulingWords.touches(plan.rulings[0]))
        assertNull(RulingWords.touches(plan.rulings[1]))
        assertTrue(RulingWords.accessibility(plan.rulings[1]).endsWith("Confirmed"))
    }

    @Test
    fun `a plan without rulings reads with none, and rulings alone plan nothing but still show`() {
        val bare = Json.parseToJsonElement(fixture).jsonObject.filterKeys { it != "rulings" }
        val plan = Plan.decode(JsonObject(bare))
        assertTrue(plan.rulings.isEmpty())
        val only = Plan(rulings = listOf(PlanRuling("x", "R-1", 1, "A", "B", "C")))
        assertTrue("rulings alone don't make a board planned (review 1005a F1)", only.isEmpty)
        assertFalse(only.showsNothing)
        assertTrue(Plan().showsNothing)
    }

    @Test
    fun `a state this build has no word for reads as unknown`() {
        val o = Json.parseToJsonElement(fixture).jsonObject
        val ruling = o["rulings"]!!.let { (it as kotlinx.serialization.json.JsonArray)[1].jsonObject }
        val moved = JsonObject(ruling + ("state" to JsonPrimitive("withdrawn")))
        assertEquals(RulingState.UNKNOWN, PlanRuling.decode(moved).state)
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
