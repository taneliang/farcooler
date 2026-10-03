package com.farcooler.ui

import com.farcooler.model.Fleet
import com.farcooler.model.Repository
import com.farcooler.model.TaskBoard
import com.farcooler.model.TaskBoardColumn
import com.farcooler.model.TaskRow
import com.farcooler.model.TaskStatus
import com.farcooler.model.Terminal
import com.farcooler.model.WorkspaceSummary
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

    /**
     * A stack saved on the Board tab — or an old board route, which becomes
     * one — comes back with the tab row selecting the tab labeled "Board".
     * Goes red if the saved word, the row's order or its labels drift apart.
     */
    @Test
    fun `the board tab's title is Board`() {
        for (saved in listOf(
            """[{"type":"workspace","hostId":"h","workspaceId":"b","tab":"board"}]""",
            """[{"type":"board","hostId":"h","workspaceId":"b"}]""",
        )) {
            val route = Backstack.decodeStack(saved)!!.single() as Route.Workspace
            val (labels, selected) = WorkspaceTab.row(route.tab)
            assertEquals("Board", labels[selected])
        }
        assertEquals(listOf("Orchestrator", "Board", "Worktrees"), WorkspaceTab.row(WorkspaceTab.BOARD).first)
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

    /**
     * A reader below Control scope sees the seat's state and no button the
     * runner would refuse: not Restart, not Replace…, not Start.
     */
    @Test
    fun `a read-scoped phone is offered no orchestrator actions`() {
        val lost = OrchestratorSeat.Lost(Terminal(id = "o", state = "lost"), "main")
        assertEquals(setOf(SeatAction.RESTART, SeatAction.REPLACE), seatActions(lost, mayControl = true))
        assertEquals(emptySet<SeatAction>(), seatActions(lost, mayControl = false))
        assertEquals(emptySet<SeatAction>(), seatActions(OrchestratorSeat.Starting(slow = true), mayControl = false))
        assertEquals(emptySet<SeatAction>(), seatActions(OrchestratorSeat.Empty(canStart = true), mayControl = false))
        assertEquals(setOf(SeatAction.REPLACE), seatActions(OrchestratorSeat.Starting(slow = true), mayControl = true))
    }

    /**
     * A refusal is read from the runner's `what`, never from its prose: a
     * sentence that happens to mention a folder is not a missing folder.
     */
    @Test
    fun `an orchestrator refusal reads the runner's word, not its sentence`() {
        assertEquals(
            "Billing already has an orchestrator. Choose Replace to start a new one.",
            orchestratorRefusal("invalid-argument", "orchestrator_taken", "Billing", replace = false),
        )
        assertEquals(
            "The runner couldn’t make Billing’s folder, so no orchestrator started.",
            orchestratorRefusal("invalid-argument", "orchestrator_home", "Billing", replace = false),
        )
        assertEquals(
            "This runner couldn’t start an orchestrator for Billing. That’s a problem in the app, not in anything you did.",
            orchestratorRefusal("invalid-argument", "harness", "Billing", replace = false),
        )
    }

    /**
     * A route whose workspace was deleted — or a stale saved stack — is Gone
     * once the runner has answered, not a screen called "Main" with an empty
     * board. Before it answers, it's Loading.
     */
    @Test
    fun `a workspace that no longer exists is gone, not Main`() {
        val billing = WorkspaceSummary(id = "ws-b", name = "Billing", repository = "repo")
        val main = WorkspaceSummary(id = "ws-m", name = "Main", isMain = true, repository = "repo")
        val fleet = Fleet(workspaces = listOf(main, billing))
        val repos = listOf(Repository(id = "repo"))
        assertEquals(WorkspacePresence.Found(billing), WorkspacePresence.of("ws-b", fleet, repos, answered = true))
        assertEquals(WorkspacePresence.Gone, WorkspacePresence.of("ws-deleted", fleet, repos, answered = true))
        assertEquals(WorkspacePresence.Loading, WorkspacePresence.of("ws-deleted", Fleet(), emptyList(), answered = false))
        // A board saved by repository, before workspaces: its Main.
        assertEquals(WorkspacePresence.Found(main), WorkspacePresence.of("repo", fleet, repos, answered = true))
        // A runner without workspaces: the repository's implicit one, and an unknown id is gone.
        assertEquals(WorkspacePresence.Found(WorkspaceSummary.implicit("repo")), WorkspacePresence.of("repo", Fleet(), repos, answered = true))
        assertEquals(WorkspacePresence.Gone, WorkspacePresence.of("other", Fleet(), repos, answered = true))
    }

    /** A sheet made on a runner without workspaces names the repository, as the row and the screen do, not "Main". */
    @Test
    fun `a sheet names an implicit workspace by its repository`() {
        val repos = listOf(Repository(id = "repo", displayName = "overnight"))
        assertEquals("overnight", workspacePlace(WorkspaceSummary.implicit("repo"), repos))
        assertEquals("Billing", workspacePlace(WorkspaceSummary(id = "b", name = "Billing", repository = "repo"), repos))
    }
}
