package com.farcooler.model

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The worktree list's workspace level, as AgentKit's `ShellWorkspacesTests`
 * states it for the iPhone: which headings a runner is split into, what is
 * under each, and which pane is a heading's own row rather than a worktree's.
 */
class FleetLayoutTest {
    private val json = Json { ignoreUnknownKeys = true }

    /**
     * Two repositories: `r1` split into Main and Billing, Billing's
     * orchestrator running in Main's checkout, one worktree unclaimed, and
     * Main's orchestrator a terminal the fleet does not list; `r2` with Main
     * alone.
     */
    private val fleet = json.decodeFromString(
        Fleet.serializer(),
        """
        {
          "runtime_healthy": true, "live_panes": 3,
          "workspaces": [
            {"id": "b1", "repository": "r1", "name": "Billing", "task_prefix": "bil",
             "is_main": false, "ordinal": 1, "orchestrator": "orch-b"},
            {"id": "m1", "repository": "r1", "name": "Main", "task_prefix": "ov",
             "is_main": true, "ordinal": 0, "orchestrator": "gone"},
            {"id": "m2", "repository": "r2", "name": "Main", "task_prefix": "sc",
             "is_main": true, "ordinal": 0}
          ],
          "worktrees": [
            {"id": "checkout", "short": "c", "repository": "r1", "task": "overnight", "branch": "main",
             "state": "active", "workspace": "m1",
             "terminals": [
               {"id": "shell", "short": "s", "title": "", "preset": "zsh", "state": "running",
                "workspace": "m1", "role": "shell"},
               {"id": "orch-b", "short": "o", "title": "", "preset": "claude", "state": "running",
                "workspace": "b1", "role": "orchestrator"}
             ]},
            {"id": "loose", "short": "l", "repository": "r1", "task": "loose", "branch": "l",
             "state": "active", "workspace": null, "terminals": []},
            {"id": "invoices", "short": "i", "repository": "r1", "task": "invoices", "branch": "i",
             "state": "active", "workspace": "b1", "terminals": []},
            {"id": "scratch", "short": "x", "repository": "r2", "task": "scratch", "branch": "x",
             "state": "active", "workspace": "m2", "terminals": []}
          ]
        }
        """.trimIndent(),
    )

    @Test
    fun eachRepositorysWorkspacesThenItsUnclaimed() {
        val headings = FleetLayout.of(fleet, mapOf("r1" to "overnight", "r2" to "scratch"))!!
        assertEquals(listOf("Main", "Billing", "Unclaimed", "Main"), headings.map { it.name })
        assertEquals(listOf("m1", "b1", "unclaimed/r1", "m2"), headings.map { it.id })
        assertEquals(
            listOf(listOf("checkout"), listOf("invoices"), listOf("loose"), listOf("scratch")),
            headings.map { it.worktrees },
        )
        assertEquals(listOf("overnight", "overnight", "overnight", "scratch"), headings.map { it.repository })
        assertTrue(headings[2].isUnclaimed)
    }

    /**
     * An orchestrator is its workspace's own row, wherever its pane runs, and
     * only when that pane is in the fleet — and it is off every worktree's rows.
     */
    @Test
    fun anOrchestratorIsItsWorkspacesRowWhereverItsPaneRuns() {
        val headings = FleetLayout.of(fleet, emptyMap())
        assertEquals(listOf(null, "orch-b", null, null), headings!!.map { it.orchestrator })
        assertEquals(setOf("orch-b"), FleetLayout.orchestrators(headings))
    }

    /**
     * Each workspace's heading has its own Board row, as on the iPhone and the
     * Mac; one whose board has no row yet has none, and Unclaimed never does.
     */
    @Test
    fun eachWorkspaceHeadingHasItsOwnBoardRow() {
        val headings = FleetLayout.of(fleet, emptyMap())
        fun row(workspace: WorkspaceSummary) =
            BoardRow(hostId = "h", repository = "r1", name = workspace.name, decisions = 0, agents = 0, workspace = workspace)
        val main = row(fleet.workspaces!!.first { it.id == "m1" })
        val billing = row(fleet.workspaces!!.first { it.id == "b1" })
        // A row the fleet has no heading for, and one keyed like Unclaimed's.
        val stray = row(WorkspaceSummary(id = "unclaimed/r1", name = "x", repository = "r1"))
        val placed = FleetLayout.boardRows(headings, listOf(billing, main, stray))
        assertEquals(mapOf("m1" to main, "b1" to billing), placed)
        assertEquals(emptyMap<String, BoardRow>(), FleetLayout.boardRows(null, listOf(main)))
    }

    /** An orchestrator's tab is called after its workspace, and only a listed one's. */
    @Test
    fun anOrchestratorsTabIsNamedForItsWorkspace() {
        val headings = FleetLayout.of(fleet, emptyMap())
        assertEquals(mapOf("orch-b" to "Billing orchestrator"), FleetLayout.orchestratorTitles(headings))
    }

    /**
     * A worktree whose only pane is an orchestrator says whose it is, where
     * its rows would otherwise say "No terminals" over a strip with a tab in it.
     */
    @Test
    fun aCheckoutWithOnlyAnOrchestratorSaysWhose() {
        val titles = FleetLayout.orchestratorTitles(FleetLayout.of(fleet, emptyMap()))
        val checkout = fleet.worktrees.first { it.id == "checkout" }
        assertNull(FleetLayout.noTerminalsNote(checkout, titles))
        val managed = checkout.copy(terminals = checkout.terminals.filter { it.id == "orch-b" })
        assertEquals("Billing orchestrator runs here", FleetLayout.noTerminalsNote(managed, titles))
        val two = managed.copy(terminals = managed.terminals + managed.terminals[0].copy(id = "orch-c"))
        assertEquals(
            "2 orchestrators run here",
            FleetLayout.noTerminalsNote(two, titles + ("orch-c" to "Search Orchestrator")),
        )
        assertEquals("No terminals", FleetLayout.noTerminalsNote(checkout.copy(terminals = emptyList()), titles))
    }

    /** One repository is not named on every heading; an empty workspace keeps its heading. */
    @Test
    fun oneRepositoryIsNotNamedOnEveryHeading() {
        val one = fleet.copy(
            workspaces = fleet.workspaces!!.filter { it.repository == "r1" },
            worktrees = fleet.worktrees.filter { it.repository == "r1" && it.id != "invoices" },
        )
        val headings = FleetLayout.of(one, mapOf("r1" to "overnight"))!!
        assertTrue(headings.all { it.repository == null })
        assertEquals(listOf("Main", "Billing", "Unclaimed"), headings.map { it.name })
        assertEquals(emptyList<String>(), headings[1].worktrees)
    }

    /** A runner without `workstreams` keeps the flat list. */
    @Test
    fun aRunnerWithoutWorkspacesHasNoLayout() {
        assertNull(FleetLayout.of(fleet.copy(workspaces = null), emptyMap()))
    }
}
