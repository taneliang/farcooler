package com.farcooler.model

import java.util.Locale
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The Agents list holds the agents that still exist (ov-518): the runner leaves out a finished subagent and an exited
 * pane, so the fleet has no Finished group, and the header's total is the runner's, which still counts what the gone
 * ones spent. The twin of AgentKit's `PlanFleetTests`.
 */
class PlanFleetTest {
    private val now = 1_800_000_000_000L
    private val hour = 3_600_000L
    private val en = Locale.US

    private fun agent(id: String, working: Boolean, startedAgo: Long, lastAgo: Long, tokens: Long, role: AgentRole = AgentRole.AGENT) =
        PlanAgent(
            id = id, role = role, harness = "claude", model = "sonnet", title = "", terminal = null, subagentId = "", tickets = emptyList(),
            working = working, doingNow = "", stopAskedAt = null, startedAt = now - startedAgo, lastAt = now - lastAgo,
            spend = PlanSpend(tokens, 0, 0, 0, null, 1, 0, 0),
        )

    @Test fun `an agent asked to stop reads Stopping until it ends, and what it works for is its tickets`() {
        val stopping = agent("agent:a", working = true, startedAgo = hour, lastAgo = 60_000, tokens = 10).copy(
            stopAskedAt = now - 1_000, tickets = listOf(PlanAgentTicket("t1", "ov-1", WorkRole.BUILD, 0), PlanAgentTicket("t2", "ov-2", WorkRole.BUILD, 0)),
        )
        assertEquals("Stopping", stopping.state(now))
        assertEquals("ov-1, ov-2", stopping.worksFor)
        assertEquals(PlanWords.NOT_ON_A_TICKET, agent("agent:b", working = true, startedAgo = hour, lastAgo = 0, tokens = 1).worksFor)
    }

    @Test fun `a subagent resumed for another round reads Working, an idle pane reads Idle`() {
        val resumed = agent("agent:a", working = true, startedAgo = 3 * hour, lastAgo = 60_000, tokens = 10)
        val idle = agent("pane:p", working = false, startedAgo = 5 * hour, lastAgo = 3 * hour, tokens = 10, role = AgentRole.AGENT)
        assertEquals("Working · 3 h", resumed.state(now))
        assertEquals("Idle · active 3 h ago", idle.state(now))
        assertFalse(idle.state(now).contains("Done"))
        assertEquals("no turn in the window: no invented age", "Idle", agent("pane:o", working = false, startedAgo = now, lastAgo = now, tokens = 0, role = AgentRole.ORCHESTRATOR).state(now))
    }

    @Test fun `the list is exactly the agents the runner sent, working ones first however little they spent`() {
        val idle = agent("pane:big", working = false, startedAgo = 9_000_000, lastAgo = 7_000_000, tokens = 5_000_000, role = AgentRole.AGENT)
        val small = agent("agent:small", working = true, startedAgo = 600_000, lastAgo = 1_000, tokens = 1_000)
        val fleet = PlanFleet.of(listOf(idle, small))
        assertEquals(listOf("agent:small", "pane:big"), fleet.agents.map { it.id })
        assertEquals(1, fleet.workingCount)
        assertFalse(fleet.isEmpty)
        assertEquals(PlanWords.NO_AGENTS, PlanFleet.of(emptyList()).header(en))
        // With none alive the day's spend still shows, in the live header's format.
        assertEquals("No agents running · 1.2M tokens in the last 24 hours · ≈$12.00", PlanFleet.of(emptyList(), PlanSpend(1_200_000, 0, 0, 0, 12_000_000, 5, 0, 0)).header(en))
    }

    @Test fun `the header total is the runner's, which counts agents that are gone, else the listed agents' sum`() {
        val live = agent("agent:live", working = true, startedAgo = 600_000, lastAgo = 1_000, tokens = 1_000_000)
        val gone = PlanSpend(4_000_000, 0, 0, 0, 12_000_000, 5, 0, 0)
        assertEquals("1 agent · 1 working · 4M tokens in the last 24 hours · ≈$12.00", PlanFleet.of(listOf(live), gone).header(en))
        val summed = PlanFleet.of(listOf(live)).header(en)
        assertEquals("1 agent · 1 working · 1M tokens in the last 24 hours", summed)
        assertNull(PlanFleet.of(listOf(live)).totalCostMicros)
    }

    /** ov-660: the plan's list counts the orchestrator, the Agents card does not, and the header says so. */
    @Test fun `the count says it includes the orchestrator when it does, so it can differ from the card's`() {
        val listed = PlanFleet.of(
            listOf(
                agent("pane:orch", working = true, startedAgo = 600_000, lastAgo = 1_000, tokens = 10, role = AgentRole.ORCHESTRATOR),
                agent("agent:s1", working = true, startedAgo = 600_000, lastAgo = 1_000, tokens = 10),
                agent("pane:builder", working = false, startedAgo = 600_000, lastAgo = 100_000, tokens = 10, role = AgentRole.BUILDER),
            ),
        )
        assertTrue(listed.includesOrchestrator)
        assertEquals("3 agents, including the orchestrator", listed.countWords)
        assertTrue(listed.header(en), listed.header(en).startsWith("3 agents, including the orchestrator · 2 working"))
        val without = PlanFleet.of(listed.agents.filter { it.role != AgentRole.ORCHESTRATOR })
        assertEquals("2 agents", without.countWords)
        assertFalse(without.header(en).contains("orchestrator"))
        assertEquals("1 agent, including the orchestrator", PlanFleet.of(listOf(listed.agents.first { it.role == AgentRole.ORCHESTRATOR })).countWords)
    }

    @Test fun `agents_spend decodes, and a runner without it leaves null`() {
        val base = """{"now_ms":1,"themes":[],"lines":[],"cards":[]"""
        assertEquals(7L, Plan.decode(base + ""","agents_spend":{"input_tokens":7,"cost_micros":9}}""").agentsSpend?.totalTokens)
        assertNull(Plan.decode("$base}").agentsSpend)
        assertTrue(Plan.decode("$base}").agents.isEmpty())
    }
}
