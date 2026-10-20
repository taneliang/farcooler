package com.farcooler.net

import com.farcooler.model.Fleet
import com.farcooler.model.RunningAgents
import com.farcooler.model.SubagentEntry
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The Agents card's count stays right across reconnects (ov-641), driven as a connection drives it: each read is the client
 * core's JSON, decoded by `Fleet.decode` into the state the screens collect, which every read replaces whole. The Mac's
 * `acrossReconnects`.
 */
class AgentsAcrossReconnectsTest {
    private val running = TestScope()
    private val fleet = MutableStateFlow(Fleet.EMPTY)

    @After
    fun tearDown() = running.close()

    /** What `fleet` answers: one workspace's orchestrator, as `Session::fleet` writes it. */
    private fun wire(extra: String) = Json.parseToJsonElement(
        """{"runtime_healthy":true,"live_panes":1,"worktrees":[{"id":"checkout","workspace":"ws","terminals":[
            {"id":"orch","state":"running","activity":"working","role":"orchestrator","workspace":"ws"$extra}]}]}""",
    )

    /** A read, as `Connection.refresh` lands it. */
    private fun read(extra: String) {
        fleet.value = Fleet.decode(wire(extra))
    }

    private fun count(rows: (String) -> List<SubagentEntry>? = { null }) = RunningAgents.workspace("ws", "r", fleet.value, rows).count

    @Test
    fun `a fresh read replaces the fleet whole, so the count is right after every reconnect`() {
        read(""","subagents":["a","b"],"subagentsRunning":2,"subagentIds":["x","y"]""")
        assertEquals(2, count())
        // The turn ends; the two it started in the background run on, and the names went with the turn.
        read(""","activity":"done","subagents":[],"subagentsRunning":2,"subagentIds":[]""")
        assertEquals("the names went with the turn; the agents did not", 2, count())
        // The link drops and comes back, and one finished meanwhile.
        read(""","subagentsRunning":1""")
        assertEquals(1, count())
        read(""","subagentsRunning":0""")
        assertEquals(0, count())
        // A runner from before the keys: the names are the count.
        read(""","subagents":["a"]""")
        assertEquals(1, count())
    }

    @Test
    fun `a conversation joins its rows only while the pane is followed live, so a reconnect never hides or invents an agent`() {
        read(""","subagents":["Survey the repo"],"subagentsRunning":2,"subagentIds":["toolu_1"]""")
        val source = FakeRowSource()
        val panes = NativePanes(
            running.scope, ClientCall { _, _ -> throw AssertionError("no call goes to a core here") }, sourceFor = { source },
        )
        val model = panes.model("orch", InMemoryPaneViews())
        // Not followed yet: the fleet's names stand in, and the count is the runner's.
        assertEquals(mapOf<String, List<SubagentEntry>>(), runBlocking { panes.subagentEntries.first() })
        assertEquals(2, count())
        // Followed live: the row for toolu_1 is joined, and the count is still the runner's.
        source.answerPage(RowJson.page(4, 3, RowJson.subagent(1, "toolu_1", "agent-a1", description = "Survey the repo")))
        model.setOnScreen(true)
        eventually("the rows") { model.store.shown.value.phase == AgentRowStore.Phase.Live }
        val rows = runBlocking { panes.subagentEntries.first() }
        assertEquals(listOf("toolu_1"), rows["orch"]?.map { it.toolUseId })
        val agents = RunningAgents.workspace("ws", "r", fleet.value) { rows[it] }
        assertEquals(2, agents.count)
        assertEquals("agent-a1", agents.agents.first().agentId)
        // The link drops: the pane is not followed, the entries go, and the count holds.
        model.setOnScreen(false)
        source.answerNothing()
        eventually("the loop to end") { source.followsOut.get() == 0 }
        read(""","subagents":["Survey the repo"],"subagentsRunning":2,"subagentIds":["toolu_1"]""")
        assertEquals(2, count())
    }
}
