package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Where choosing an agent in the agents view goes (ov-641): the rule the screen carries out. */
class AgentOpeningTest {
    private val mark = GlanceMark(GlanceMark.Attention.QUIET, GlanceMark.Core.PRODUCING)

    private fun agent(
        kind: RunningAgent.Kind, terminal: String, worktree: String = "wt-$terminal", row: String? = null, agentId: String? = null,
    ) = RunningAgent(
        id = "r/$terminal", kind = kind, runner = "r", workspace = "ws", name = "x", activity = "", startedMs = null, mark = mark,
        worktree = worktree, terminal = terminal, row = row, agentId = agentId,
    )

    private val orchestratorSub = agent(RunningAgent.Kind.SUBAGENT, "orch", row = "sub:a", agentId = "agent-a")
    private val namedOnly = agent(RunningAgent.Kind.SUBAGENT, "orch")
    private val laneSub = agent(RunningAgent.Kind.SUBAGENT, "lane", row = "sub:b", agentId = "agent-b")
    private val lane = agent(RunningAgent.Kind.PANE, "lane")

    @Test
    fun `only a subagent with a row and a named agent opens its own conversation`() {
        assertTrue(orchestratorSub.opensOwnConversation)
        assertFalse(namedOnly.opensOwnConversation)
        assertFalse("the unnamed rest is one row for several", agent(RunningAgent.Kind.SUBAGENT, "orch", row = "sub:a").opensOwnConversation)
        assertFalse(lane.opensOwnConversation)
    }

    @Test
    fun `the orchestrator's subagent opens its conversation in the one already up, on a phone and beside the plan`() {
        assertEquals(
            AgentOpening.OpenOwn("orch", "sub:a", "agent-a", pane = null),
            AgentOpening.of(orchestratorSub, orchestrator = "orch", wide = false, ownOffered = true),
        )
        assertEquals(
            AgentOpening.OpenOwn("orch", "sub:a", "agent-a", pane = null),
            AgentOpening.of(orchestratorSub, orchestrator = "orch", wide = true, ownOffered = true),
        )
    }

    @Test
    fun `without the runner's subagent rows the orchestrator's agents open nothing on a phone and focus the chat beside the plan`() {
        assertEquals(AgentOpening.Stay, AgentOpening.of(orchestratorSub, "orch", wide = false, ownOffered = false))
        assertEquals(AgentOpening.Stay, AgentOpening.of(namedOnly, "orch", wide = false, ownOffered = true))
        assertEquals(AgentOpening.FocusChat, AgentOpening.of(orchestratorSub, "orch", wide = true, ownOffered = false))
    }

    @Test
    fun `a lane's subagent opens its own conversation in that pane, and a pane opens itself`() {
        assertEquals(
            AgentOpening.OpenOwn("lane", "sub:b", "agent-b", pane = "wt-lane"),
            AgentOpening.of(laneSub, "orch", wide = false, ownOffered = true),
        )
        assertEquals(AgentOpening.OpenPane("wt-lane", "lane"), AgentOpening.of(laneSub, "orch", wide = false, ownOffered = false))
        assertEquals(AgentOpening.OpenPane("wt-lane", "lane"), AgentOpening.of(lane, "orch", wide = true, ownOffered = true))
    }
}
