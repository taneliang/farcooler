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
            withdraw = { steps += "withdraw" },
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
            withdraw = { steps += "withdraw" },
            clear = { steps += "clear"; BringHere.Answer.Took(true) },
        )
        assertEquals(emptyList<String>(), steps)
        assertTrue((issue as AgentConversation.SendIssue.Said).words.contains("pasted block"))
    }

    @Test
    fun `a clear that may have taken text keeps it placed and says it's in both`() = runBlocking {
        for (failure in listOf(
            AgentConversation.SendFailure.Refused("partly"), AgentConversation.SendFailure.TimedOut,
            AgentConversation.SendFailure.Lost(notSent = false),
        )) {
            var placed: String? = null
            var withdrawn = false
            val issue = BringHere.run(
                read = { BringHere.Answer.Took("from the box") },
                place = { placed = it },
                withdraw = { withdrawn = true },
                clear = { BringHere.Answer.Failed(failure) },
            )
            assertEquals("$failure", "from the box", placed)
            assertFalse("$failure", withdrawn)
            assertTrue("$failure", (issue as AgentConversation.SendIssue.DraftLeftInTerminal).words.startsWith("The draft is here"))
        }
    }

    @Test
    fun `a clear refused with the box whole gives the text back and never says to clear the box`() = runBlocking {
        for (what in listOf("changed", "too_tall", "typing", "sending", "unfamiliar", "prompt")) {
            var composer = "mine"
            var withdrawn: String? = null
            val issue = BringHere.run(
                read = { BringHere.Answer.Took("from the box") },
                place = { composer = BringHere.merged(it, composer) },
                withdraw = { withdrawn = it; composer = BringHere.withdrawn(it, composer) },
                clear = { BringHere.Answer.Failed(AgentConversation.SendFailure.Refused(what)) },
            )
            assertEquals(what, "from the box", withdrawn)
            assertEquals(what, "mine", composer)
            assertTrue(what, issue != null && issue !is AgentConversation.SendIssue.DraftLeftInTerminal)
        }
        var gone: String? = null
        BringHere.run(
            read = { BringHere.Answer.Took("x") }, place = {}, withdraw = { gone = it },
            clear = { BringHere.Answer.Failed(AgentConversation.SendFailure.Lost(notSent = true)) },
        )
        assertEquals("a clear that never left", "x", gone)
    }

    @Test
    fun `a clear that cleared nothing gives the text back`() = runBlocking {
        var composer = ""
        val issue = BringHere.run(
            read = { BringHere.Answer.Took("fix the login") },
            place = { composer = BringHere.merged(it, composer) },
            withdraw = { composer = BringHere.withdrawn(it, composer) },
            clear = { BringHere.Answer.Took(false) },
        )
        assertEquals("", composer)
        assertTrue((issue as AgentConversation.SendIssue.Said).words.contains("emptied"))
    }

    @Test
    fun `withdrawn takes only the brought text, and merged keeps a first line's indent`() {
        val merged = BringHere.merged("fix the login\nthen the tests", "and the docs")
        assertEquals("and the docs", BringHere.withdrawn("fix the login\nthen the tests", merged))
        assertEquals("", BringHere.withdrawn("fix the login", "fix the login"))
        assertEquals("I rewrote it", BringHere.withdrawn("fix the login", "I rewrote it"))
        assertEquals("    indented\nmore", BringHere.merged("    indented\nmore\n", ""))
    }

    @Test
    fun `an empty box brings nothing`() = runBlocking {
        val steps = mutableListOf<String>()
        val issue = BringHere.run(
            read = { BringHere.Answer.Took("") },
            place = { steps += "place" },
            withdraw = { steps += "withdraw" },
            clear = { steps += "clear"; BringHere.Answer.Took(false) },
        )
        assertNull(issue)
        assertEquals(emptyList<String>(), steps)
    }
}
