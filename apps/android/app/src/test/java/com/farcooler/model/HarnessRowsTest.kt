package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * ov-474: claude's injected turns arrive from the projector already read, decoded from the
 * page the client core writes from a redacted real transcript
 * (`test/fixtures/agent-rows-harness.json`; AgentKit's `HarnessRowsTests` read the same file).
 */
class HarnessRowsTest {
    private val turns: Map<String, AgentRow.Turn> by lazy {
        AgentRowPage.decode(AgentRowFixture.harnessPage).rows
            .mapNotNull { row -> (row.kind as? AgentRow.Kind.OfTurn)?.let { row.id to it.turn } }
            .toMap()
    }

    @Test
    fun `a hand-back is a report named by its agent's call, with its body clean`() {
        val turn = turns.getValue("turn:p2")
        val harness = turn.harness!!
        assertTrue(AgentConversation.isHarness(turn))
        assertEquals("Report", harness.kind)
        assertEquals("Report from review dead-layer removal", AgentConversation.harnessTitle(harness))
        assertTrue(harness.body.startsWith("**Verdict: ready.**"))
        assertFalse(harness.body.contains("[Subagent hand-back]") || harness.body.contains("<agent-message"))
        assertFalse(harness.body.split("\n").any { it.startsWith("  ") })
    }

    @Test
    fun `a peer no launch names is called by a short id, and with no id a subagent`() {
        val harness = turns.getValue("turn:p5").harness!!
        assertEquals("Message from agent a272e8", AgentConversation.harnessTitle(harness))
        assertEquals("Message from a subagent", AgentConversation.harnessTitle(harness.copy(agentId = null)))
    }

    @Test
    fun `a task notice is its status line, a reminder is a System note, a typed turn has none`() {
        assertEquals("Agent finished: review dead-layer removal", AgentConversation.harnessTitle(turns.getValue("turn:p3").harness!!))
        assertEquals(
            "Background command finished: gh run watch 123456 --exit-status (exit 0)",
            AgentConversation.harnessTitle(turns.getValue("turn:p4").harness!!),
        )
        assertEquals("System note", AgentConversation.harnessTitle(turns.getValue("turn:p6").harness!!))
        assertNull(turns.getValue("turn:p1").harness)
    }
}
