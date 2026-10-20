package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The agents at work (ov-619, ov-641): Far Cooler's panes and Claude's subagents read as one list, scoped to a
 * workspace for the Agents card. The twin of AgentKit's `RunningAgentsTests`, case for case. The fleet decides how many
 * (the larger of a pane's count and its names; a reconnect's fresh fleet is the whole of keeping it right); a pane's
 * conversation adds detail, joined on call ids, and fills in the unnamed rest.
 */
class RunningAgentsTest {
    private val runner = "RUNNER-A"
    private val main = "ws-main"
    private val billing = "ws-billing"

    private fun terminal(
        id: String,
        title: String = "claude",
        activity: String? = "working",
        workspace: String? = null,
        role: String? = "agent",
        state: String = "running",
        line: String? = null,
        turn: Double? = null,
        subagents: List<String>? = null,
        running: Int? = null,
        ids: List<String>? = null,
        failed: Boolean? = null,
    ) = Terminal(
        id = id, short = id, title = title, preset = "claude", state = state, activity = activity,
        turnStartedAt = turn, line = line, subagents = subagents, subagentsRunning = running, subagentIds = ids,
        workspace = workspace, role = role, turnFailed = failed,
    )

    private fun worktree(id: String, workspace: String?, vararg terminals: Terminal) =
        Worktree(id = id, short = id, repository = "repo", task = id, branch = "b/$id", state = "ready", terminals = terminals.toList(), workspace = workspace)

    private fun entry(id: String, title: String, asked: String, running: Boolean = true, started: Long? = null, call: String? = null) =
        SubagentEntry(id = id, title = title, description = asked, running = running, startedMs = started, toolUseId = call)

    /** The orchestrator alone, with [subagents] named under the call ids [ids] and [running] running. */
    private fun orchestrating(subagents: List<String>, running: Int?, ids: List<String>? = null) = Fleet(
        runtimeHealthy = true, livePanes = 1,
        worktrees = listOf(
            worktree("checkout", main, terminal("orch", workspace = main, role = "orchestrator", subagents = subagents, running = running, ids = ids)),
        ),
    )

    /**
     * Main's orchestrator with two subagents (named, as the runner names them, by what they were asked), a lane agent with
     * one, a shell, an exited agent; Billing's own agent; an unclaimed worktree's agent.
     */
    private val fleet = Fleet(
        runtimeHealthy = true, livePanes = 6,
        worktrees = listOf(
            worktree(
                "checkout", main,
                terminal("orch", workspace = main, role = "orchestrator", subagents = listOf("Survey the repo", "Review ov-616"), running = 2, ids = listOf("toolu_s", "toolu_r")),
                terminal("shell", activity = null, workspace = main, role = "shell"),
            ),
            worktree(
                "lane", main,
                terminal("builder", title = "Build ov-619", workspace = main, line = "Edit RunningAgents.swift", turn = 1_000.0, subagents = listOf("Survey the repo")),
                terminal("gone", workspace = main, state = "exited"),
            ),
            worktree("bill", billing, terminal("biller", activity = "blocked", workspace = billing)),
            worktree("scratch", null, terminal("loose", activity = "idle")),
        ),
    )

    @Test
    fun `a workspace lists its orchestrator's subagents, then each agent pane with its own, never the orchestrator, a shell or an exited pane`() {
        val agents = RunningAgents.workspace(main, runner, fleet)
        assertEquals(listOf("Subagent", "Subagent", "Build ov-619", "Subagent"), agents.agents.map { it.name })
        assertEquals(listOf("Survey the repo", "Review ov-616", "Edit RunningAgents.swift", "Survey the repo"), agents.agents.map { it.activity })
        assertEquals(
            listOf(RunningAgent.Kind.SUBAGENT, RunningAgent.Kind.SUBAGENT, RunningAgent.Kind.PANE, RunningAgent.Kind.SUBAGENT),
            agents.agents.map { it.kind },
        )
        assertEquals(4, agents.count)
        assertEquals("4 agents", agents.summary)
        assertEquals(RunningAgents.DOT_LIMIT, agents.dots.size)
        val builder = agents.agents[2]
        assertEquals(1_000L, builder.startedMs)
        assertEquals("lane", builder.worktree)
        assertEquals("builder", builder.terminal)
        // A subagent known only by name opens its parent's pane.
        assertEquals("orch", agents.agents[0].terminal)
        assertNull(agents.agents[0].row)
        assertEquals("every row has its own id", agents.agents.size, agents.agents.map { it.id }.toSet().size)
    }

    @Test
    fun `with none at work the card has nothing to count`() {
        val quiet = Fleet(
            runtimeHealthy = true, livePanes = 1,
            worktrees = listOf(worktree("checkout", main, terminal("orch", workspace = main, role = "orchestrator"))),
        )
        val agents = RunningAgents.workspace(main, runner, quiet)
        assertEquals(0, agents.count)
        assertTrue(agents.dots.isEmpty())
        assertEquals("1 agent", RunningAgents.summary(1))
    }

    @Test
    fun `a conversation the phone holds adds each named subagent's type, newest work and start, and opens it`() {
        val rows = listOf(entry("sub:a1", "General purpose", "Review ov-616", started = 5_000, call = "toolu_r"))
        val agents = RunningAgents.workspace(main, runner, fleet) { if (it == "orch") rows else null }
        val review = agents.agents[1]
        assertEquals("General purpose", review.name)
        assertEquals("Review ov-616", review.activity)
        assertEquals(5_000L, review.startedMs)
        assertEquals("sub:a1", review.row)
        assertEquals(4, agents.count)
    }

    @Test
    fun `the fleet decides the count, a row stuck at running adds nothing, and an empty conversation hides nothing`() {
        // The conversation still says an agent the runner has finished runs.
        val stuck = listOf(
            entry("sub:a1", "General purpose", "Review ov-616", call = "toolu_r"),
            entry("sub:old", "General purpose", "An agent that ended", call = "toolu_old"),
        )
        val withStuck = RunningAgents.workspace(main, runner, fleet) { if (it == "orch") stuck else null }
        assertEquals("an ended agent was counted", 4, withStuck.count)
        assertFalse(withStuck.agents.any { it.activity == "An agent that ended" })

        // A conversation that hasn't loaded its subagents yet.
        val empty = RunningAgents.workspace(main, runner, fleet) { emptyList() }
        assertEquals("a running agent was hidden", 4, empty.count)
        assertEquals(listOf("Survey the repo", "Review ov-616"), empty.agents.map { it.activity }.take(2))
    }

    @Test
    fun `a lane pane's own conversation adds detail to its subagents too`() {
        val rows = listOf(entry("sub:b1", "Explore", "Survey the repo for the card"))
        val agents = RunningAgents.workspace(main, runner, fleet) { if (it == "builder") rows else null }
        val sub = agents.agents[3]
        assertEquals("builder", sub.terminal)
        assertEquals("Explore", sub.name)
        assertEquals("sub:b1", sub.row)
    }

    @Test
    fun `two subagents of one type are told apart by number`() {
        val rows = listOf(
            entry("sub:a1", "General purpose", "Survey the repo", call = "toolu_s"),
            entry("sub:a2", "General purpose", "Review ov-616", call = "toolu_r"),
        )
        val agents = RunningAgents.workspace(main, runner, fleet) { if (it == "orch") rows else null }
        assertEquals(listOf("General purpose", "General purpose 2"), agents.agents.take(2).map { it.name })
        assertEquals(listOf("Survey the repo", "Review ov-616"), agents.agents.take(2).map { it.activity })
    }

    @Test
    fun `a reconnect's fresh fleet is the whole of the count, the same fleet twice counts the same, and a new one recounts`() {
        val before = RunningAgents.workspace(main, runner, fleet)
        assertEquals(before, RunningAgents.workspace(main, runner, fleet))
        val after = fleet.copy(
            worktrees = fleet.worktrees.mapIndexed { i, w ->
                when (i) {
                    0 -> w.copy(terminals = listOf(w.terminals[0].copy(subagents = emptyList(), subagentsRunning = 0), w.terminals[1]))
                    1 -> w.copy(terminals = listOf(w.terminals[0].copy(state = "exited"), w.terminals[1]))
                    else -> w
                }
            },
        )
        assertEquals(0, RunningAgents.workspace(main, runner, after).count)
    }

    @Test
    fun `agents a turn left running in the background are counted after it ends, no names, a count of 3, counts 3`() {
        val agents = RunningAgents.workspace(main, runner, orchestrating(emptyList(), running = 3))
        assertEquals("the card collapsed while three agents ran", 3, agents.count)
        assertEquals("3 agents", agents.summary)
        assertEquals(3, agents.dots.size)
        assertEquals(listOf("3 more agents"), agents.agents.map { it.name })
        assertEquals("orch", agents.agents.first().terminal)
        assertNull(agents.agents.first().row)
        assertEquals("1 more agent", RunningAgents.more(1))
    }

    @Test
    fun `a held conversation fills the unnamed rest from its newest running rows, and the count still caps it`() {
        val rows = listOf(
            entry("sub:old", "General purpose", "Stuck at running", call = "toolu_0"),
            entry("sub:b1", "Explore", "Survey the repo", call = "toolu_1"),
            entry("sub:b2", "General purpose", "Review ov-616", call = "toolu_2"),
            entry("sub:done", "General purpose", "Finished", running = false, call = "toolu_3"),
        )
        val agents = RunningAgents.workspace(main, runner, orchestrating(emptyList(), running = 2)) { if (it == "orch") rows else null }
        assertEquals(2, agents.count)
        assertEquals(listOf("Explore", "General purpose"), agents.agents.map { it.name })
        assertEquals(listOf("Survey the repo", "Review ov-616"), agents.agents.map { it.activity })
        assertTrue(agents.agents.all { it.row != null })

        // Fewer rows than the count: the rest is still said.
        val short = RunningAgents.workspace(main, runner, orchestrating(emptyList(), running = 4)) { if (it == "orch") listOf(rows[1]) else null }
        assertEquals(4, short.count)
        assertEquals(listOf("Explore", "3 more agents"), short.agents.map { it.name })
    }

    @Test
    fun `a name the runner cleaned still finds its row, by the call id behind it`() {
        val fleet = orchestrating(listOf("Read …/daemon/src/watch.rs closely"), running = 1, ids = listOf("toolu_x"))
        val rows = listOf(
            entry("sub:y", "Explore", "Something else", call = "toolu_y"),
            entry("sub:x", "General purpose", "Read /Users/someone/Dev/acme/crates/daemon/src/watch.rs closely", call = "toolu_x"),
        )
        val agents = RunningAgents.workspace(main, runner, fleet) { if (it == "orch") rows else null }
        assertEquals(1, agents.count)
        assertEquals("the row wasn't found by its call id", "sub:x", agents.agents.first().row)
        assertEquals("General purpose", agents.agents.first().name)
    }

    @Test
    fun `from a runner that sends no ids or count, names take rows by order, and the names are the count`() {
        val fleet = orchestrating(listOf("Survey the repo"), running = null)
        val rows = listOf(entry("sub:a", "Explore", "Survey the repo for the card"))
        val agents = RunningAgents.workspace(main, runner, fleet) { if (it == "orch") rows else null }
        assertEquals(1, agents.count)
        assertEquals("sub:a", agents.agents.first().row)
    }

    @Test
    fun `the conversation's subagent rows become entries by their call id`() {
        fun row(id: String, call: String, status: AgentRow.Status) = AgentRow(
            id = id, ord = 1, rev = 1,
            kind = AgentRow.Kind.OfSubagent(
                AgentRow.Subagent(toolUseId = call, agentType = "general-purpose", description = "Review ov-616", background = true, status = status, startedMs = 9L),
            ),
        )
        val entries = SubagentEntry.of(listOf(row("sub:a", "toolu_r", AgentRow.Status.Running), row("sub:b", "toolu_s", AgentRow.Status.Done)))
        assertEquals(listOf("General purpose", "General purpose"), entries.map { it.title })
        assertEquals(listOf(true, false), entries.map { it.running })
        assertEquals("toolu_r", entries.first().toolUseId)
        assertEquals(9L, entries.first().startedMs)
    }

    @Test
    fun `the fleet decode carries how many run and the call id behind each name`() {
        val t = kotlinx.serialization.json.Json { ignoreUnknownKeys = true }.decodeFromString(
            Terminal.serializer(),
            """{"id":"t","subagents":["a"],"subagentsRunning":3,"subagentIds":["toolu_1"]}""",
        )
        assertEquals(3, t.subagentsRunning)
        assertEquals(listOf("toolu_1"), t.subagentIds)
        val bare = kotlinx.serialization.json.Json { ignoreUnknownKeys = true }.decodeFromString(Terminal.serializer(), """{"id":"t"}""")
        assertNull(bare.subagentsRunning)
        assertNull(bare.subagentIds)
    }

    @Test
    fun `a pane whose last turn died is marked failed, and it is the dots' too`() {
        val fleet = Fleet(
            runtimeHealthy = true, livePanes = 2,
            worktrees = listOf(
                worktree(
                    "lane", main,
                    terminal("broke", activity = "done", workspace = main, failed = true),
                    terminal("fine", activity = "done", workspace = main, failed = false),
                    terminal("busy", workspace = main),
                ),
            ),
        )
        val agents = RunningAgents.workspace(main, runner, fleet)
        assertEquals(listOf(true, false, false), agents.agents.map { it.failed })
        assertEquals("failed ones lead the dots when they lead the list", listOf(true, false, false), agents.dotAgents.map { it.failed })
        // The same weight as an agent waiting on a person; the ring's hue is the view's.
        assertEquals(GlanceMark.Attention.NEEDS_YOU, agents.agents.first().mark.attention)
        assertEquals(GlanceMark.Attention.TO_REVIEW, agents.agents[1].mark.attention)
    }
}
