package com.farcooler.net

import com.farcooler.model.AgentRow
import com.farcooler.model.AgentRowPage
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A subagent's own conversation, open in its parent's place (ov-641, folding ov-657): the iPhone's `AgentDrill` and the Mac's.
 * Opened from the agents view or its row; followed only while the conversation is on screen; given up with the pane.
 */
class NativeSubagentDrillTest {
    private val running = TestScope()
    private val source = FakeRowSource()
    private val child = FakeRowSource()
    private val memory = InMemoryPaneViews()

    @After
    fun tearDown() = running.close()

    private fun model() = NativePaneModel(
        terminal = "t1",
        store = AgentRowStore(running.scope, retryDelayMs = 1, followWaitMs = 1),
        source = source,
        sink = ConversationSink { _, _, _ -> false },
        memory = memory,
        scope = running.scope,
    ).also { source.children["agent-a1"] = child }

    @Test
    fun `a subagent opens only where the runner serves its rows, and only once it is named`() {
        val model = model()
        assertFalse("no subagent_rows", model.openAgent("sub:toolu_1", "agent-a1"))
        assertNull(model.opened)
        model.offer(rich = false, interrupts = false, subagents = true)
        assertFalse("not named yet", model.openAgent("sub:toolu_1", null))
        assertFalse("a source with no such agent", model.openAgent("sub:toolu_9", "agent-zz"))
        assertTrue(model.openAgent("sub:toolu_1", "agent-a1"))
        assertEquals("agent-a1", model.opened?.agentId)
        assertEquals("sub:toolu_1", model.opened?.row)
        // The runner stops serving them: back to the pane's own.
        model.offer(rich = false, interrupts = false, subagents = false)
        assertNull(model.opened)
    }

    @Test
    fun `its own rows are followed only while the conversation is on screen, and Back lets go`() {
        val model = model()
        model.offer(rich = false, interrupts = false, subagents = true)
        source.answerPage(RowJson.page(4, 2, RowJson.prose(9, "The pane's own words.")))
        child.answerPage(RowJson.page(4, 3, RowJson.prose(1, "Found the join.")))
        assertTrue(model.openAgent("sub:toolu_1", "agent-a1"))
        // Nothing is read until the pane is on screen.
        Thread.sleep(50)
        assertEquals(0, child.pageCalls.get())
        model.setOnScreen(true)
        eventually("the agent's own rows") { model.opened?.store?.shown?.value?.rows?.isNotEmpty() == true }
        val rows = model.opened!!.store.shown.value.rows
        assertTrue("its rows, not the pane's: ${rows.map { it.id }}", rows.any { it.id == "prose:1" })
        eventually("the pane's own rows") { model.store.shown.value.rows.any { it.id == "prose:9" } }
        assertNull("the pane's own rows are untouched", model.store.shown.value.rows.firstOrNull { it.id == "prose:1" })
        // Off screen, the follow is let go.
        child.answerNothing()
        model.setOnScreen(false)
        eventually("the follow to end") { child.followsOut.get() == 0 }
        model.setOnScreen(true)
        model.closeAgent()
        assertNull(model.opened)
        eventually("the follow to end") { child.followsOut.get() == 0 }
    }

    @Test
    fun `an open asked for before the runner's offer arrives is done once it has`() {
        val model = model()
        model.requestOpen("sub:toolu_1", "agent-a1")
        assertNull(model.opened)
        model.offer(rich = false, interrupts = false, subagents = true)
        assertEquals("agent-a1", model.opened?.agentId)
    }

    @Test
    fun `asking the connection to open one reaches a pane that is made later`() {
        val panes = NativePanes(
            running.scope, ClientCall { _, _ -> throw AssertionError("no call goes to a core here") },
            sourceFor = { source.also { it.children["agent-a1"] = child } },
        )
        // The agents view opens a lane's pane and its subagent together: the pane isn't made yet.
        panes.open("t1", "sub:toolu_1", "agent-a1")
        assertNull(panes.ifMade("t1"))
        val model = panes.model("t1", memory)
        model.offer(rich = false, interrupts = false, subagents = true)
        assertEquals("agent-a1", model.opened?.agentId)
        assertTrue("an agent's conversation shows where the terminal did", model.showing)
    }

    @Test
    fun `a terminal the runner no longer lists lets go of its model, and an unread fleet lets go of none`() {
        val panes = NativePanes(running.scope, ClientCall { _, _ -> throw AssertionError("no call goes to a core here") }, sourceFor = { FakeRowSource() })
        panes.model("t1", memory)
        panes.model("t2", memory)
        panes.retain(emptySet())
        assertNotNull("not read yet", panes.ifMade("t1"))
        panes.retain(setOf("t2", "t9"))
        assertNull(panes.ifMade("t1"))
        assertNotNull(panes.ifMade("t2"))
    }

    @Test
    fun `a subagent row decodes the id its own rows are asked for by`() {
        val page = AgentRowPage.decode(RowJson.page(4, 3, RowJson.subagent(1, "toolu_1", "agent-a1"), RowJson.subagent(2, "toolu_2", null)))
        val subs = page.rows.map { (it.kind as AgentRow.Kind.OfSubagent).subagent }
        assertEquals(listOf("agent-a1", null), subs.map { it.agentId })
    }
}
