package com.farcooler.model

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Bring here (ov-369, R-28), AgentKit's `BringHereTests`: read, placed, then cleared of exactly that; never in neither place. */
class BringHereTest {
    private fun build(vararg caps: Capability) = DaemonBuild("1", true, "linux", caps.map { it.wire }.toSet(), grantedScope = "control")

    @Test
    fun `offered for claude on a runner that serves it`() {
        assertTrue(BringHere.offered("claude", build(Capability.BRING_DRAFT)))
        assertFalse(BringHere.offered("claude", build(Capability.COMPOSE)))
        assertFalse(BringHere.offered("codex", build(Capability.BRING_DRAFT)))
        assertFalse(BringHere.offered("claude", null))
    }

    @Test
    fun `the box comes first, then the composer's text`() {
        assertEquals("fix the login\nthen the tests\nand the docs", BringHere.merged("fix the login\nthen the tests", "and the docs"))
        assertEquals("from the box", BringHere.merged("from the box\n", ""))
        assertEquals("mine", BringHere.merged("", "mine"))
    }

    @Test
    fun `read, placed, then cleared of exactly that`() = runBlocking {
        val steps = mutableListOf<String>()
        var composer = "and the docs"
        val issue = BringHere.run(
            read = { steps += "read"; BringHere.Answer.Took("fix the login") },
            place = { steps += "place"; composer = BringHere.merged(it, composer) },
            clear = { steps += "clear $it"; BringHere.Answer.Took(true) },
        )
        assertNull(issue)
        assertEquals(listOf("read", "place", "clear fix the login"), steps)
        assertEquals("fix the login\nand the docs", composer)
    }

    @Test
    fun `a refused read moves nothing`() = runBlocking {
        val steps = mutableListOf<String>()
        val issue = BringHere.run(
            read = { BringHere.Answer.Failed(AgentConversation.SendFailure.Refused("pasted")) },
            place = { steps += "place" },
            clear = { steps += "clear"; BringHere.Answer.Took(true) },
        )
        assertEquals(emptyList<String>(), steps)
        assertTrue((issue as AgentConversation.SendIssue.Said).words.contains("pasted block"))
    }

    @Test
    fun `a failed clear keeps the text placed and says it's in both`() = runBlocking {
        for (failure in listOf(
            AgentConversation.SendFailure.Refused("changed"), AgentConversation.SendFailure.Refused("partly"),
            AgentConversation.SendFailure.TimedOut,
        )) {
            var placed: String? = null
            val issue = BringHere.run(
                read = { BringHere.Answer.Took("from the box") },
                place = { placed = it },
                clear = { BringHere.Answer.Failed(failure) },
            )
            assertEquals("$failure", "from the box", placed)
            assertTrue("$failure", (issue as AgentConversation.SendIssue.DraftLeftInTerminal).words.startsWith("The draft is here"))
        }
    }

    @Test
    fun `an empty box brings nothing`() = runBlocking {
        val steps = mutableListOf<String>()
        val issue = BringHere.run(
            read = { BringHere.Answer.Took("") },
            place = { steps += "place" },
            clear = { steps += "clear"; BringHere.Answer.Took(false) },
        )
        assertNull(issue)
        assertEquals(emptyList<String>(), steps)
    }
}
