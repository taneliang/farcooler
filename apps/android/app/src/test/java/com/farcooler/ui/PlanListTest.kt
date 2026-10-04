package com.farcooler.ui

import com.farcooler.model.BoardReads
import com.farcooler.model.BoardSummary
import com.farcooler.model.TaskBoard
import com.farcooler.model.TaskBoardColumn
import com.farcooler.model.TaskRow
import com.farcooler.model.TaskStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Plan chosen stands in for the whole task list on the Board tab (ov-274), Unread with it, as on the Mac. */
class PlanListTest {
    private val now = 1_800_000_000_000L
    private fun row(n: Int, status: TaskStatus) =
        TaskRow(id = "id-$n", key = "k-$n", title = "Task $n", status = status, statusSince = now - n * 60_000L, updatedAt = now - n * 60_000L)

    private val rows = listOf(row(1, TaskStatus.DONE), row(2, TaskStatus.NEEDS_DECISION), row(3, TaskStatus.BACKLOG))
    private val board = TaskBoard(TaskStatus.ORDER.map { s -> TaskBoardColumn(s, rows.filter { it.status == s }) })
    private val reads = BoardReads(now - 24 * 3_600_000L)

    @Test
    fun `Tasks lists Unread first and then the statuses`() {
        val entries = BoardList.entries(board, emptySet(), now, reads, unread = BoardSummary.make(board.rows, reads = reads), showsPlan = false)
        assertTrue(entries.first() is BoardListEntry.UnreadHeader)
        assertTrue(entries.any { it is BoardListEntry.Header })
    }

    @Test
    fun `Plan lists none of it, Unread included, so the plan is the first thing seen`() {
        val entries = BoardList.entries(board, emptySet(), now, reads, unread = BoardSummary.make(board.rows, reads = reads), showsPlan = true)
        assertEquals(emptyList<BoardListEntry>(), entries)
    }
}
