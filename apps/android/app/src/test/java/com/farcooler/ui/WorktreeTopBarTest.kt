package com.farcooler.ui

import com.farcooler.model.TaskBoard
import com.farcooler.model.TaskBoardColumn
import com.farcooler.model.TaskRef
import com.farcooler.model.TaskRow
import com.farcooler.model.TaskStatus
import com.farcooler.model.Terminal
import com.farcooler.model.WorkspaceSummary
import com.farcooler.model.Worktree
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** The task chip in a pane's top bar (spec §3.2): what it names, and where it opens. */
class WorktreeTopBarTest {
    private val invoice = TaskRef("t-9", "bil-9", "Invoice PDF export", "in_progress")

    @Test
    fun `the top bar names the pane's task`() {
        val claude = Terminal(id = "p", preset = "claude", state = "running")
        val worktree = Worktree(id = "w", terminals = listOf(claude), openTasks = listOf(invoice))
        assertEquals("bil-9 Invoice PDF export", topBarTask(claude, worktree)?.label)

        // Dispatched for a card that isn't open here: an id with no name is no chip.
        val dispatched = claude.copy(taskId = "t-done")
        assertNull(topBarTask(dispatched, worktree))
        // Two open tasks, and no pane of its own: none.
        assertNull(topBarTask(claude, worktree.copy(openTasks = listOf(invoice, invoice.copy(id = "t-4")))))
        assertNull(topBarTask(null, worktree))
    }

    @Test
    fun `the chip's board is the one that holds the card, never a guess`() {
        val row = TaskRow("t-9", "bil-9", "Invoice PDF export", TaskStatus.IN_PROGRESS, 0L)
        val boards = mapOf(
            "ws-other" to TaskBoard(listOf(TaskBoardColumn(TaskStatus.IN_PROGRESS, emptyList()))),
            "ws-held" to TaskBoard(listOf(TaskBoardColumn(TaskStatus.IN_PROGRESS, listOf(row)))),
        )
        assertEquals("ws-held", taskBoardOf("t-9", boards))
        // No board read so far holds it: nothing, rather than the pane's workspace.
        assertNull(taskBoardOf("t-9", boards - "ws-held"))
    }

    @Test
    fun `the boards read to find a card start with the pane's workspace`() {
        fun ws(id: String, repository: String) = WorkspaceSummary(id = id, repository = repository)
        val list = listOf(ws("ws-a", "repo"), ws("ws-owner", "repo"), ws("ws-other-repo", "elsewhere"), ws("ws-pane", "repo"))
        val pane = Terminal(id = "p", workspace = "ws-pane")
        val worktree = Worktree(id = "w", workspace = "ws-owner", repository = "repo")
        assertEquals(
            listOf("ws-pane", "ws-owner", "ws-a"),
            boardsToRead(emptyMap(), list, pane, worktree).map { it.id },
        )
        // A board already read would have held it: not read again.
        val read = mapOf("ws-owner" to TaskBoard(emptyList()))
        assertEquals(listOf("ws-pane", "ws-a"), boardsToRead(read, list, pane, worktree).map { it.id })
    }
}
