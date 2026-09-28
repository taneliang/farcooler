package com.farcooler.ui

import com.farcooler.model.TaskBoard
import com.farcooler.model.TaskBoardColumn
import com.farcooler.model.TaskRow
import com.farcooler.model.TaskStatus
import com.farcooler.model.Terminal
import com.farcooler.model.Worktree
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The workspace screen's rules (spec §6): its tabs, its Board tab's list
 * form, a task's links to its worktree, its Orchestrator tab's states, and
 * where a launch lands. The screen draws these; the JVM holds them.
 */
class WorkspaceScreenTest {
    private fun task(id: String, status: TaskStatus, worktree: String? = null) =
        TaskRow(id, "bil-$id", "Task $id", status, 0L, worktreeId = worktree)

    /** The tab row reads Orchestrator, Board, Worktrees — "Board", not the old row's "Main board". */
    @Test
    fun `the board tab's title is Board`() {
        assertEquals("Board", WorkspaceTab.BOARD.title)
        assertEquals(listOf("Orchestrator", "Board", "Worktrees"), WorkspaceTab.entries.map { it.title })
    }

    /**
     * Owner decision 3: an empty status is never hidden. It's a header
     * reading "Backlog 0" that can't be expanded, and every status is there,
     * Needs Decision first. Done starts collapsed with its count showing, and
     * opens when tapped.
     */
    @Test
    fun `an empty status is a collapsed zero header`() {
        val board = TaskBoard(
            listOf(
                TaskBoardColumn(TaskStatus.IN_PROGRESS, listOf(task("1", TaskStatus.IN_PROGRESS))),
                TaskBoardColumn(TaskStatus.DONE, listOf(task("2", TaskStatus.DONE))),
            )
        )
        val entries = BoardList.entries(board, toggled = emptySet())
        val headers = entries.filterIsInstance<BoardListEntry.Header>()
        assertEquals(TaskStatus.ORDER, headers.map { it.status })

        val backlog = headers.first { it.status == TaskStatus.BACKLOG }
        assertEquals(0, backlog.count)
        assertFalse(backlog.expandable)
        assertFalse(backlog.expanded)

        val done = headers.first { it.status == TaskStatus.DONE }
        assertEquals(1, done.count)
        assertFalse("Done starts collapsed", done.expanded)
        assertEquals(listOf("1"), entries.filterIsInstance<BoardListEntry.Card>().map { it.row.id })

        val opened = BoardList.entries(board, toggled = setOf(TaskStatus.DONE, TaskStatus.BACKLOG))
        assertEquals(listOf("1", "2"), opened.filterIsInstance<BoardListEntry.Card>().map { it.row.id })
        assertFalse("an empty status can't be opened", opened.filterIsInstance<BoardListEntry.Header>().first { it.status == TaskStatus.BACKLOG }.expanded)
    }

    /**
     * Spec §3.2, item 3: a task reaches its worktree and its changes through
     * its own `worktree_id`, with or without a live agent. A task in review
     * whose agent has gone was a dead end on every platform.
     */
    @Test
    fun `a task with a worktree but no agent reaches its changes`() {
        val worktree = Worktree(id = "w", task = "fc-3-webhooks", terminals = emptyList())
        val reviewing = task("9", TaskStatus.IN_REVIEW, worktree = "w")
        assertEquals(
            listOf(TaskLinkRow.Changes("w", null, null), TaskLinkRow.Worktree("w", "fc-3-webhooks")),
            TaskLinks.rows(reviewing, listOf(worktree)),
        )
        // No worktree named, or one removed since: nothing to push.
        assertTrue(TaskLinks.rows(task("8", TaskStatus.TODO), listOf(worktree)).isEmpty())
        assertTrue(TaskLinks.rows(task("7", TaskStatus.IN_REVIEW, worktree = "gone"), listOf(worktree)).isEmpty())
    }

    /** Spec §8's states, and the seat that sticks: Replace… after thirty seconds, or at once when this phone didn't ask. */
    @Test
    fun `an orchestrator that never arrives offers Replace`() {
        val pane = Terminal(id = "o", state = "running", role = "orchestrator")
        val lost = pane.copy(state = "lost")
        val worktrees = listOf(Worktree(id = "main", terminals = listOf(pane)))
        assertEquals(OrchestratorSeat.Empty(canStart = true), OrchestratorSeat.of(null, emptyList(), null, 0, true))
        assertEquals(OrchestratorSeat.Live(pane, "main"), OrchestratorSeat.of("o", worktrees, null, 0, true))
        assertEquals(
            OrchestratorSeat.Lost(lost, "main"),
            OrchestratorSeat.of("o", listOf(Worktree(id = "main", terminals = listOf(lost))), null, 0, true),
        )
        assertEquals(OrchestratorSeat.Starting(slow = false), OrchestratorSeat.of(null, emptyList(), 1_000, 30_999, true))
        assertEquals(OrchestratorSeat.Starting(slow = true), OrchestratorSeat.of(null, emptyList(), 1_000, 31_000, true))
        assertEquals(OrchestratorSeat.Starting(slow = true), OrchestratorSeat.of("o", emptyList(), null, 0, true))
    }

    /** Ruling 4: nothing waiting, and the app opens on the last workspace, over Needs You. */
    @Test
    fun `with nothing waiting, launch pushes the last workspace over Needs You`() {
        val last = Route.Workspace("h", "billing", WorkspaceTab.BOARD)
        assertEquals(listOf(Route.NeedsYou, last), Backstack.launch(listOf(Route.NeedsYou), waiting = 0, last = last))
        // Nothing remembered: the front door.
        assertEquals(listOf(Route.NeedsYou), Backstack.launch(listOf(Route.NeedsYou), waiting = 0, last = null))
        // A stack somebody already has — restored, or moved — is left alone.
        val restored = listOf(Route.NeedsYou, Route.Terminal("h", "w"))
        assertEquals(restored, Backstack.launch(restored, waiting = 0, last = last))
    }

    @Test
    fun `launch lands on Needs You when it has items`() {
        val last = Route.Workspace("h", "billing")
        assertEquals(listOf(Route.NeedsYou), Backstack.launch(listOf(Route.NeedsYou), waiting = 2, last = last))
    }
}
