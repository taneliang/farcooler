package com.farcooler.ui

import com.farcooler.model.BoardReads
import com.farcooler.model.BoardSummary
import com.farcooler.model.TaskBoard
import com.farcooler.model.TaskBoardColumn
import com.farcooler.model.TaskNoteKind
import com.farcooler.model.TaskNoteRow
import com.farcooler.model.TaskRow
import com.farcooler.model.TaskStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The board's Unread section (ov-113): first in the list, each group cut at five with "and N more". */
class BoardUnreadListTest {
    private val now = 1_800_000_000_000L

    private fun row(n: Int, status: TaskStatus = TaskStatus.DONE) = TaskRow(
        id = "id-$n", key = "k-$n", title = "Task $n", status = status, statusSince = now - n * 60_000L, updatedAt = now - n * 60_000L,
    )

    private fun board(vararg rows: TaskRow) = TaskBoard(
        TaskStatus.ORDER.map { status -> TaskBoardColumn(status, rows.filter { it.status == status }) },
    )

    private val reads = BoardReads(now - 24 * 3_600_000L)

    @Test
    fun theSectionIsFirstAndAnEmptyOneSaysAllCaughtUp() {
        val b = board(row(1))
        val entries = BoardList.entries(b, emptySet(), now, reads, unread = BoardSummary.make(b.rows, reads = reads))
        assertTrue(entries.first() is BoardListEntry.UnreadHeader)
        assertEquals(listOf("Finished"), entries.filterIsInstance<BoardListEntry.UnreadGroup>().map { it.title })
        assertTrue("the status sections follow it", entries.indexOfFirst { it is BoardListEntry.Header } > entries.indexOfFirst { it is BoardListEntry.UnreadLine })

        val empty = BoardList.entries(b, emptySet(), now, BoardReads(now), unread = BoardSummary.make(b.rows, reads = BoardReads(now)))
        assertEquals(listOf(BoardListEntry.UnreadHeader(0), BoardListEntry.UnreadNothing), empty.take(2))
        assertFalse((empty.first() as BoardListEntry.UnreadHeader).offersMarkAll)
        assertEquals("You’re all caught up.", (empty[1] as BoardListEntry.UnreadNothing).text)
    }

    @Test
    fun aGroupIsCutAtFiveWithAndNMore() {
        val rows = (1..7).map { row(it) }
        val b = board(*rows.toTypedArray())
        val unread = BoardList.unreadEntries(BoardSummary.make(b.rows, reads = reads))
        assertEquals(5, unread.filterIsInstance<BoardListEntry.UnreadLine>().size)
        assertEquals(BoardListEntry.UnreadGroup("Finished", 7), unread.filterIsInstance<BoardListEntry.UnreadGroup>().single())
        assertEquals("and 2 more", unread.filterIsInstance<BoardListEntry.UnreadMore>().single().text)
        assertTrue((unread.first() as BoardListEntry.UnreadHeader).offersMarkAll)
        assertEquals(7, (unread.first() as BoardListEntry.UnreadHeader).count)
    }

    @Test
    fun theGroupsComeInTheMacsOrderAndAnActivityLineIsItsTicketsNewestNote() {
        val rows = listOf(row(1), row(2, TaskStatus.NEEDS_DECISION), row(3, TaskStatus.IN_PROGRESS))
        val notes = mapOf("id-3" to listOf(TaskNoteRow("n", TaskNoteKind.FINDING, now - 1000, "found it")))
        val unread = BoardList.unreadEntries(BoardSummary.make(rows, notes, reads))
        assertEquals(listOf("Finished", "Needs you or review", "Activity"), unread.filterIsInstance<BoardListEntry.UnreadGroup>().map { it.title })
        assertEquals("found it", unread.filterIsInstance<BoardListEntry.UnreadNote>().single().activity.text)
    }

    @Test
    fun withoutASummaryThereIsNoSection() {
        val b = board(row(1))
        assertTrue(BoardList.entries(b, emptySet(), now, reads).none { it is BoardListEntry.UnreadHeader })
    }
}
