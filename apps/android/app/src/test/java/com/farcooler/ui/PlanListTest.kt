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

/** Plan chosen switches only the task sections of the Board tab (ov-274): Unread stays. */
class PlanListTest {
    private val now = 1_800_000_000_000L
    private fun row(n: Int, status: TaskStatus) =
        TaskRow(id = "id-$n", key = "k-$n", title = "Task $n", status = status, statusSince = now - n * 60_000L, updatedAt = now - n * 60_000L)

    @Test
    fun `Unread entries are the ones that stay under the Plan view`() {
        val rows = listOf(row(1, TaskStatus.DONE), row(2, TaskStatus.NEEDS_DECISION), row(3, TaskStatus.BACKLOG))
        val board = TaskBoard(TaskStatus.ORDER.map { s -> TaskBoardColumn(s, rows.filter { it.status == s }) })
        val reads = BoardReads(now - 24 * 3_600_000L)
        val entries = BoardList.entries(board, emptySet(), now, reads, unread = BoardSummary.make(board.rows, reads = reads))
        val staying = entries.filter { it.isUnread }
        assertTrue("the Unread header stays", staying.first() is BoardListEntry.UnreadHeader)
        assertTrue("no status header stays", staying.none { it is BoardListEntry.Header || it is BoardListEntry.Card })
        assertEquals("everything but Unread is a task section", entries.size - staying.size, entries.count { it is BoardListEntry.Header || it is BoardListEntry.Card || it is BoardListEntry.ShowMore || it is BoardListEntry.History })
        assertTrue(entries.any { it is BoardListEntry.Header })
    }
}
