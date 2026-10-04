package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The phone's Needs You before anything needs answering, and a blank board
 * (ov-205). The same table as AgentKit's `PhoneFirstRunTests` and
 * `BoardFormTests`, so a rule changed on one phone and not the other fails on
 * the side it was left out of.
 */
class PhoneFirstRunTest {
    private fun row(id: String, orchestrator: Terminal? = null, implicit: Boolean = false) = WorkspaceRow(
        hostId = "h",
        workspace = WorkspaceSummary(id = id, name = id, isImplicit = implicit),
        name = id,
        orchestrator = orchestrator,
        count = 0,
    )

    private fun section(repository: String, vararg rows: WorkspaceRow) = RepositoryWorkspaces(
        hostId = "h", repository = repository, name = repository, workspaces = rows.toList(),
        unclaimed = emptyList(), unclaimedCount = 0, hidden = emptyList(),
    )

    private val running = Terminal(id = "t1", preset = "claude", state = "running")

    @Test
    fun `a runner with no sections has no repositories`() {
        assertTrue(PhoneFirstRun.hasNoRepositories(emptyList()))
        assertFalse(PhoneFirstRun.hasNoRepositories(listOf(section("a", row("main")))))
    }

    @Test
    fun `no orchestrator anywhere needs a workspace and none running on any`() {
        assertTrue(PhoneFirstRun.noOrchestratorAnywhere(listOf(section("a", row("main")))))
        // One running anywhere, in another repository, is enough to say nothing.
        assertFalse(
            PhoneFirstRun.noOrchestratorAnywhere(
                listOf(section("a", row("main")), section("b", row("w", running))),
            ),
        )
        // Nothing at all is the other empty state, not this one.
        assertFalse(PhoneFirstRun.noOrchestratorAnywhere(emptyList()))
        // An implicit workspace can't run one, so it can't be missing one.
        assertFalse(PhoneFirstRun.noOrchestratorAnywhere(listOf(section("a", row("a", implicit = true)))))
    }

    @Test
    fun `a blank board's line depends on whether an orchestrator leads it and is running`() {
        val up = PhoneFirstRun.blankLine(ledByOrchestrator = true, orchestratorRunning = true)
        val down = PhoneFirstRun.blankLine(ledByOrchestrator = true, orchestratorRunning = false)
        val implicit = PhoneFirstRun.blankLine(ledByOrchestrator = false, orchestratorRunning = false)
        assertEquals(FirstRunCopy.BOARD_WITH_ORCHESTRATOR, up)
        assertEquals(FirstRunCopy.BOARD_NO_ORCHESTRATOR, down)
        assertTrue(down.startsWith("Start the orchestrator"))
        assertFalse(implicit.contains("orchestrator"))
        for (line in listOf(up, down, implicit)) {
            assertFalse(line.contains("grouped by", ignoreCase = true))
            assertTrue(line.endsWith("."))
        }
    }

    @Test
    fun `show orchestrator is offered only when one leads the board and none is running`() {
        assertTrue(PhoneFirstRun.offersOrchestrator(ledByOrchestrator = true, orchestratorRunning = false))
        assertFalse(PhoneFirstRun.offersOrchestrator(ledByOrchestrator = true, orchestratorRunning = true))
        assertFalse(PhoneFirstRun.offersOrchestrator(ledByOrchestrator = false, orchestratorRunning = false))
    }

    @Test
    fun `a board with no task in any status is blank`() {
        assertTrue(TaskBoard.EMPTY.isEmpty)
    }

    @Test
    fun `the explainer comes with the first runner only`() {
        assertTrue(NotificationAsk.explainsAfterFirstRunner(hadRunners = false, hasRunners = true))
        assertFalse(NotificationAsk.explainsAfterFirstRunner(hadRunners = true, hasRunners = true))
        assertFalse(NotificationAsk.explainsAfterFirstRunner(hadRunners = true, hasRunners = false))
    }

    /** The permission prompt must never be the first thing a new person sees. */
    @Test
    fun `a phone with no runner does not ask at launch`() {
        assertFalse(NotificationAsk.asksAtLaunch(hasRunners = false))
        assertTrue(NotificationAsk.asksAtLaunch(hasRunners = true))
    }

    @Test
    fun `a daemon build reads which harnesses the runner found`() {
        val said = DaemonBuild(version = "1", matches = true, platform = "linux", agentsFound = listOf("codex"))
        assertEquals(listOf(AgentHarness.CODEX), said.availability.installed)
        assertEquals(listOf(AgentHarness.CLAUDE, AgentHarness.CURSOR), said.availability.missing)
        val silent = DaemonBuild(version = "1", matches = true, platform = "linux")
        assertEquals(AgentHarness.entries, silent.availability.installed)
    }

    @Test
    fun `the front door says no agents are working when workspaces have no orchestrator`() {
        val idle = listOf(RunnerCount(RunnerLink.ANSWERING, 0, true))
        assertEquals(
            FirstRunCopy.NO_ORCHESTRATOR_RUNNING,
            reassurance(idle, "", worktrees = 1, noOrchestrator = true),
        )
        // Agents already working make that untrue.
        val busy = listOf(RunnerCount(RunnerLink.ANSWERING, 2, true))
        assertEquals("2 agents are working.", reassurance(busy, "", worktrees = 1, noOrchestrator = true))
        assertEquals("Nothing is running.", reassurance(idle, "", worktrees = 1, noOrchestrator = false))
    }

    @Test
    fun `the no orchestrator sentence says what a workspace is for`() {
        assertTrue(FirstRunCopy.NO_ORCHESTRATOR_RUNNING.contains("one line of work"))
    }
}
