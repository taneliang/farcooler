package com.farcooler.ui

import com.farcooler.model.Destination
import com.farcooler.model.DestinationPayloads
import com.farcooler.model.DestinationResolver
import com.farcooler.model.DestinationResolver.Arrival
import com.farcooler.model.Fleet
import com.farcooler.model.TaskBoard
import com.farcooler.model.TaskBoardColumn
import com.farcooler.model.TaskRow
import com.farcooler.model.TaskStatus
import com.farcooler.model.Terminal
import com.farcooler.model.Worktree
import com.farcooler.model.WorkspaceSummary
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * How a notification, and a relaunch, reach a screen (ov-182, ov-183): the
 * payload each kind carries, through [DestinationPayloads], the driver, the
 * resolver and [DestinationRoutes], to the stack that opens. Every step is
 * the one [AppModel] runs; only the clock and the connections are made here.
 */
class DestinationRoutingTest {
    private var now = 0L

    private fun driver() = DestinationDriver { now }

    private val workspace = WorkspaceSummary(id = "ws-1", name = "Billing", repository = "repo", orchestrator = "orch")

    private fun row(id: String, key: String) =
        TaskRow(id = id, key = key, title = key, status = TaskStatus.IN_PROGRESS, statusSince = 0)

    private fun board(vararg rows: TaskRow) = TaskBoard(listOf(TaskBoardColumn(TaskStatus.IN_PROGRESS, rows.toList())))

    private val fleet = Fleet(
        worktrees = listOf(
            Worktree(
                id = "wt-1",
                workspace = "ws-1",
                terminals = listOf(
                    Terminal(id = "t-agent", role = "agent", taskId = "task-1", workspace = "ws-1"),
                    Terminal(id = "t-shell", role = "shell"),
                ),
            ),
            Worktree(
                id = "wt-orch",
                workspace = "ws-1",
                terminals = listOf(Terminal(id = "t-orch", role = "orchestrator", workspace = "ws-1")),
            ),
        ),
        workspaces = listOf(workspace),
    )

    /** One runner that has answered: its daemon read, its fleet and its board in. */
    private fun ready(host: String = "h1", runnerId: String? = "runner-a") = DestinationWorld.Source(
        hostId = host, runnerId = runnerId, ready = true, idle = false, fleet = fleet,
        boardList = listOf(workspace), boards = mapOf("ws-1" to board(row("task-1", "ov-1"))),
    )

    private fun dialing(host: String = "h1") =
        DestinationWorld.Source(hostId = host, runnerId = null, ready = false, idle = false, fleet = null)

    private fun tap(vararg extras: Pair<String, String>) = DestinationPayloads.from(mapOf(*extras)::get)

    /** Taps `extras` on a runner that's ready, and the stack it opens. */
    private fun opened(vararg extras: Pair<String, String>, sources: List<DestinationWorld.Source> = listOf(ready())): DestinationRoutes.Landing? {
        val destination = assertNotNull(tap(*extras))
        val driver = driver()
        driver.request(destination, Arrival.NOTIFICATION)
        val step = driver.step(DestinationWorld.world(sources), moved = false)
        val open = step as? DestinationDriver.Step.Open ?: return null
        return DestinationRoutes.stack(open.destination, taskOf = { _, id -> if (id == "t-agent") "task-1" else null })
    }

    private fun <T> assertNotNull(value: T?): T {
        org.junit.Assert.assertNotNull(value)
        return value!!
    }

    // ---- each notification kind opens its subject ----

    @Test
    fun aTaskNoticeOpensItsTaskOverItsBoard() {
        val landing = opened(
            "kind" to "task", "task" to "ov-1", "runner" to "runner-a", "event" to "review", "noticeId" to "t:runner-a:ov-1",
        )
        assertEquals(
            listOf(Route.NeedsYou, Route.Workspace("h1", "ws-1", WorkspaceTab.BOARD), Route.BoardTask("h1", "ws-1", "task-1")),
            landing?.stack,
        )
    }

    @Test
    fun aBackgroundedDecisionPushWithAnEmptyTerminalOpensTheTask() {
        // The relay sends terminal "" beside a task, and Firebase copies it into the intent.
        val landing = opened("terminal" to "", "kind" to "decision", "task" to "ov-1", "runner" to "runner-a")
        assertEquals(Route.BoardTask("h1", "ws-1", "task-1"), landing?.stack?.last())
    }

    @Test
    fun anAgentPushOpensItsPaneOverItsTask() {
        val landing = opened("terminal" to "t-agent", "runner" to "runner-a")
        assertEquals(
            listOf(
                Route.NeedsYou,
                Route.Workspace("h1", "ws-1", WorkspaceTab.BOARD),
                Route.BoardTask("h1", "ws-1", "task-1"),
                Route.Terminal("h1", "wt-1"),
            ),
            landing?.stack,
        )
        assertEquals("t-agent", landing?.pane)
    }

    @Test
    fun aShellPaneOpensItsWorktreeOverTheWorktreesTab() {
        val landing = opened("com.farcooler.terminal" to "t-shell")
        assertEquals(
            listOf(Route.NeedsYou, Route.Workspace("h1", "ws-1", WorkspaceTab.WORKTREES), Route.Terminal("h1", "wt-1")),
            landing?.stack,
        )
    }

    @Test
    fun anOrchestratorPaneOpensItsWorkspaceOnTheOrchestratorTab() {
        val landing = opened("terminal" to "t-orch")
        assertEquals(listOf(Route.NeedsYou, Route.Workspace("h1", "ws-1", WorkspaceTab.ORCHESTRATOR)), landing?.stack)
    }

    @Test
    fun aNeedsYouDestinationOpensTheFrontDoor() {
        val landing = opened("destination" to Destination.NEEDS_YOU.encoded())
        assertEquals(listOf(Route.NeedsYou), landing?.stack)
    }

    @Test
    fun anEncodedDestinationOpensWhatItNames() {
        val d = Destination(place = Destination.Place.Task("ws-1", Destination.TaskRef(key = "ov-1")), question = true)
        val landing = opened("destination" to d.encoded())
        assertEquals(Route.BoardTask("h1", "ws-1", "task-1"), landing?.stack?.last())
    }

    @Test
    fun aPaneThatIsGoneAndATaskThatIsGoneOpenNothing() {
        assertNull(opened("terminal" to "t-gone"))
        assertNull(opened("kind" to "task", "task" to "ov-404", "runner" to "runner-a"))
    }

    @Test
    fun aTaskNoticeForAnotherRunnerOpensNothing() {
        assertNull(opened("kind" to "task", "task" to "ov-1", "runner" to "runner-b"))
    }

    // ---- cold launch: held for its runner ----

    @Test
    fun aTapBeforeItsRunnerComesUpIsHeldThenOpened() {
        val driver = driver()
        driver.request(assertNotNull(tap("terminal" to "t-agent", "runner" to "runner-a")), Arrival.NOTIFICATION)
        // Cold launch: the runner is still connecting.
        assertEquals(DestinationDriver.Step.Wait, driver.step(DestinationWorld.world(listOf(dialing())), moved = false))
        assertTrue(driver.isPending)
        now = 20_000
        val step = driver.step(DestinationWorld.world(listOf(ready())), moved = false)
        assertTrue(step is DestinationDriver.Step.Open)
        assertEquals(false, driver.isPending)
    }

    @Test
    fun aTapNeverOpensPastItsDeadline() {
        val driver = driver()
        driver.request(assertNotNull(tap("terminal" to "t-agent", "runner" to "runner-a")), Arrival.NOTIFICATION)
        now = DestinationResolver.Deadline.NOTIFICATION_MS + 1
        // What it names has just turned up: too late, the phone has moved on.
        val step = driver.step(DestinationWorld.world(listOf(ready())), moved = false)
        assertEquals(DestinationDriver.Step.Stay(DestinationResolver.Note.NOT_FOUND, Arrival.NOTIFICATION), step)
    }

    @Test
    fun aRunnerNothingIsDialingIsToldToConnect() {
        val driver = driver()
        val idle = DestinationWorld.Source(hostId = "h2", runnerId = null, ready = false, idle = true, fleet = null)
        driver.request(Destination(runner = Destination.Runner(host = "h2"), place = Destination.Place.Terminal("t-x")), Arrival.NOTIFICATION)
        assertEquals(DestinationDriver.Step.Connect("h2"), driver.step(DestinationWorld.world(listOf(ready(), idle)), moved = false))
    }

    // ---- relaunch: where you were, falling back when it's gone ----

    private fun restored(destination: Destination, sources: List<DestinationWorld.Source> = listOf(ready()), moved: Boolean = false): DestinationRoutes.Landing? {
        val driver = driver()
        driver.request(destination, Arrival.RESTORE)
        val step = driver.step(DestinationWorld.world(sources), moved)
        return (step as? DestinationDriver.Step.Open)?.let { DestinationRoutes.stack(it.destination) }
    }

    @Test
    fun aRelaunchReturnsToTheTaskItWasOn() {
        val kept = DestinationRoutes.destination(
            listOf(Route.NeedsYou, Route.Workspace("h1", "ws-1", WorkspaceTab.BOARD), Route.BoardTask("h1", "ws-1", "task-1")),
        )
        assertEquals(Route.BoardTask("h1", "ws-1", "task-1"), restored(assertNotNull(kept))?.stack?.last())
    }

    @Test
    fun aRelaunchReturnsToTheWorktreeAndItsPane() {
        val stack = listOf(Route.NeedsYou, Route.Workspace("h1", "ws-1", WorkspaceTab.WORKTREES), Route.Terminal("h1", "wt-1"))
        val kept = DestinationRoutes.destination(stack) { _, _ -> "t-shell" }
        assertEquals("t-shell", kept?.pane)
        val landing = restored(assertNotNull(Destination.decode(kept!!.encoded())))
        assertEquals(stack, landing?.stack)
        assertEquals("t-shell", landing?.pane)
    }

    @Test
    fun aGoneTaskFallsBackToItsWorkspaceQuietly() {
        val kept = Destination(
            runner = Destination.Runner(host = "h1"),
            place = Destination.Place.Task("ws-1", Destination.TaskRef(id = "task-gone")),
        )
        assertEquals(
            listOf(Route.NeedsYou, Route.Workspace("h1", "ws-1", WorkspaceTab.ORCHESTRATOR)),
            restored(kept)?.stack,
        )
    }

    @Test
    fun aGoneWorktreeFallsBackToItsWorkspace() {
        val kept = Destination(
            runner = Destination.Runner(host = "h1"),
            place = Destination.Place.Worktree("wt-gone", "ws-1"),
            pane = "t-gone",
        )
        assertEquals(Route.Workspace("h1", "ws-1", WorkspaceTab.ORCHESTRATOR), restored(kept)?.stack?.last())
    }

    @Test
    fun aGoneWorkspaceFallsBackToTheRunnersFirst() {
        val kept = Destination(runner = Destination.Runner(host = "h1"), place = Destination.Place.Workspace("ws-gone"))
        assertEquals(Route.Workspace("h1", "ws-1", WorkspaceTab.ORCHESTRATOR), restored(kept)?.stack?.last())
    }

    @Test
    fun aRelaunchIsHeldForItsRunnerAndThenFallsBackToNeedsYou() {
        val kept = Destination(runner = Destination.Runner(host = "h1"), place = Destination.Place.Workspace("ws-1"))
        val driver = driver()
        driver.request(kept, Arrival.RESTORE)
        assertEquals(DestinationDriver.Step.Wait, driver.step(DestinationWorld.world(listOf(dialing())), moved = false))
        now = DestinationResolver.Deadline.RESTORE_MS + 1
        val step = driver.step(DestinationWorld.world(listOf(dialing())), moved = false)
        assertEquals(Destination.NEEDS_YOU, (step as DestinationDriver.Step.Open).destination)
    }

    @Test
    fun aRelaunchWithNoRunnersWaitsForNothing() {
        val driver = driver()
        val kept = Destination(runner = Destination.Runner(host = "h1"), place = Destination.Place.Workspace("ws-1"))
        assertEquals(false, driver.restore(kept, runners = 0))
        assertEquals(false, driver.isPending)
        assertEquals(true, driver.restore(kept, runners = 1))
        assertEquals(true, driver.isPending)
    }

    @Test
    fun aRelaunchYieldsToSomebodyWhoMovedFirst() {
        val kept = Destination(runner = Destination.Runner(host = "h1"), place = Destination.Place.Workspace("ws-1"))
        assertNull(restored(kept, moved = true))
    }

    @Test
    fun aTapOutranksARelaunchStillWaiting() {
        val driver = driver()
        driver.request(Destination(runner = Destination.Runner(host = "h1"), place = Destination.Place.Workspace("ws-1")), Arrival.RESTORE)
        driver.request(assertNotNull(tap("terminal" to "t-agent", "runner" to "runner-a")), Arrival.NOTIFICATION)
        val step = driver.step(DestinationWorld.world(listOf(ready())), moved = false) as DestinationDriver.Step.Open
        assertEquals(Arrival.NOTIFICATION, step.arrival)
        assertEquals("t-agent", step.destination.pane)
    }

    // ---- the stack as a destination ----

    @Test
    fun aStackIsWhereItsDeepestPlaceIs() {
        assertEquals(Destination.NEEDS_YOU, DestinationRoutes.destination(listOf(Route.NeedsYou)))
        // Settings over a task is the task.
        val over = listOf(Route.NeedsYou, Route.Workspace("h", "w", WorkspaceTab.BOARD), Route.BoardTask("h", "w", "t"), Route.Settings)
        assertEquals(Destination.Place.Task("w", Destination.TaskRef(id = "t")), DestinationRoutes.destination(over)?.place)
        assertNull(DestinationRoutes.destination(listOf(Route.Onboarding)))
        val history = listOf(Route.NeedsYou, Route.Workspace("h", "w", WorkspaceTab.BOARD), Route.BoardHistory("h", "w", "done"))
        assertEquals(Destination.Place.History("w", "done"), DestinationRoutes.destination(history)?.place)
    }

    @Test
    fun aWorkspaceKeepsItsTab() {
        for (tab in WorkspaceTab.entries) {
            val stack = listOf(Route.NeedsYou, Route.Workspace("h", "w", tab))
            val landing = DestinationRoutes.stack(assertNotNull(DestinationRoutes.destination(stack)))
            assertEquals(stack, landing.stack)
        }
    }
}
