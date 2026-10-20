package com.farcooler.capture

import com.farcooler.model.Fleet
import com.farcooler.model.RunningAgents
import com.farcooler.model.SubagentEntry
import com.farcooler.model.Terminal
import com.farcooler.model.Worktree

/** Four agents at work in one workspace (ov-641): the orchestrator's subagents, a lane's pane, and one of its own. */
object AgentsFixtures {
    const val WORKSPACE = "ws"

    private val now get() = System.currentTimeMillis()

    private fun fleet(running: Boolean) = Fleet(
        runtimeHealthy = true,
        livePanes = 2,
        worktrees = listOf(
            Worktree(
                id = "checkout", workspace = WORKSPACE,
                terminals = listOf(
                    Terminal(
                        id = "o", state = "running", activity = "working", role = "orchestrator", workspace = WORKSPACE,
                        subagents = if (running) listOf("ov-452 conversation view hierarchy", "ov-454 image paste and attachments") else emptyList(),
                        subagentIds = if (running) listOf("toolu_1", "toolu_2") else emptyList(),
                        subagentsRunning = if (running) 3 else 0,
                    ),
                ),
            ),
            Worktree(
                id = "lane", workspace = WORKSPACE,
                terminals = if (running) {
                    listOf(
                        Terminal(
                            id = "b", title = "Build ov-619", state = "running", activity = "working", role = "agent", workspace = WORKSPACE,
                            line = "Edit RunningAgents.swift", turnStartedAt = (now - 185_000).toDouble(),
                        ),
                    )
                } else {
                    emptyList()
                },
            ),
        ),
    )

    /** Running: three of the orchestrator's (two named, one in the background) and a lane's pane. */
    fun running(): RunningAgents = RunningAgents.workspace(WORKSPACE, "runner", fleet(true)) { terminal ->
        if (terminal != "o") {
            null
        } else {
            listOf(
                SubagentEntry("s1", "General purpose", "ov-452 conversation view hierarchy", startedMs = now - 190_000, toolUseId = "toolu_1"),
                SubagentEntry("s2", "General purpose", "ov-454 image paste and attachments", startedMs = now - 185_000, toolUseId = "toolu_2"),
                SubagentEntry("s3", "Explore", "ov-453 subagent tray", startedMs = now - 120_000, toolUseId = "toolu_3"),
            )
        }
    }

    fun none(): RunningAgents = RunningAgents.workspace(WORKSPACE, "runner", fleet(false))
}

/** Nine agents at work: more than a sheet has room for. */
fun AgentsFixtures.many(): RunningAgents = RunningAgents.workspace(
    AgentsFixtures.WORKSPACE, "runner",
    Fleet(
        runtimeHealthy = true,
        livePanes = 9,
        worktrees = listOf(
            Worktree(
                id = "lane", workspace = AgentsFixtures.WORKSPACE,
                terminals = (1..9).map { n ->
                    Terminal(
                        id = "p$n", title = listOf("Build ov-619", "Review ov-616", "Fix ov-640", "Explore the agents tray", "Integrator for train 190", "Mac: the card", "Android: the sheet", "Relay: the retry", "Site: the page")[n - 1],
                        state = "running", activity = "working", role = "agent", workspace = AgentsFixtures.WORKSPACE,
                        line = "Edit RunningAgents.swift and the tests that go with it, then run the whole suite once",
                        turnStartedAt = (System.currentTimeMillis() - n * 61_000L).toDouble(),
                    )
                },
            ),
        ),
    ),
)
