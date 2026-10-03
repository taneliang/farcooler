package com.farcooler.ui

import com.farcooler.model.BoardDone
import com.farcooler.model.BoardHistory
import com.farcooler.model.BoardReads
import com.farcooler.model.BoardSectionCut
import com.farcooler.model.TaskBoard
import com.farcooler.model.TaskBoardColumn
import com.farcooler.model.TaskRow
import com.farcooler.model.TaskStatus
import java.time.ZoneOffset
import java.time.temporal.WeekFields
import java.util.Locale
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Done shows the unread and today's with a floor of three (ov-103), a long
 * section ten, and the History page groups and searches: AgentKit's
 * BoardDoneTests and BoardUnreadTests, in Kotlin.
 */
class BoardDoneTest {
    // 2027-01-15 08:00 UTC, a Friday.
    private val now = 1_800_000_000_000L
    private val hour = 60L * 60 * 1000
    private val day = 24 * hour
    private val utc = ZoneOffset.UTC

    private fun row(key: String, agoMs: Long, status: TaskStatus = TaskStatus.DONE, title: String = "Task $key") =
        TaskRow("id-$key", key, title, status, now - agoMs, updatedAt = now - agoMs)

    @Test
    fun `done keeps the unread and today's, newest first, with a floor of three`() {
        val reads = BoardReads(now - 2 * day)
        val today = (0 until 3).map { row("t$it", (it + 1) * 600_000L) }
        val unread = (0 until 2).map { row("u$it", 20 * hour + it) }
        val old = (0 until 10).map { row("o$it", 30 * day + it) }
        val shown = BoardDone.shown((old + unread + today).reversed(), reads, now, utc)
        assertEquals(listOf("t0", "t1", "t2", "u0", "u1"), shown.map { it.key })

        val opened = reads.open(unread[0], now)
        assertEquals(listOf("t0", "t1", "t2", "u1"), BoardDone.shown(today + unread + old, opened, now, utc).map { it.key })
        assertEquals(listOf("o0", "o1", "o2"), BoardDone.shown(old, reads, now, utc).map { it.key })
        assertEquals("All Done", BoardDone.historyTitle(TaskStatus.DONE))
        assertEquals("All Canceled", BoardDone.historyTitle(TaskStatus.CANCELLED))
    }

    @Test
    fun `opening reads a ticket, and a runner clock ahead can't leave it unread`() {
        val reads = BoardReads(now - day)
        val fin = row("f", hour)
        assertTrue(reads.finishedUnread(fin))
        assertFalse(reads.open(fin, now).finishedUnread(fin))
        assertTrue(reads.open(fin, now).finishedUnread(row("g", hour)))
        val ahead = row("a", -10 * 60_000L)
        assertFalse(reads.open(ahead, now).finishedUnread(ahead))
        assertEquals(emptyMap<String, Long>(), BoardReads(now, mapOf("x" to now - 1)).pruned().opened)
    }

    @Test
    fun `the list cuts Done by its rule, a long section at ten, and offers History`() {
        val reads = BoardReads(now - 2 * day)
        val done = (0 until 15).map { row("d$it", 40 * day + it) }
        val todo = (0 until 14).map { row("t$it", hour, TaskStatus.TODO) }
        val board = TaskBoard(listOf(TaskBoardColumn(TaskStatus.TODO, todo), TaskBoardColumn(TaskStatus.DONE, done)))
        val open = setOf(TaskStatus.DONE)
        val entries = BoardList.entries(board, open, now, reads)
        assertEquals(13, entries.filterIsInstance<BoardListEntry.Card>().size)
        assertEquals("Show 4 More", entries.filterIsInstance<BoardListEntry.ShowMore>().single().title)
        val history = entries.filterIsInstance<BoardListEntry.History>().single()
        assertEquals(15, history.total)
        assertEquals("All Done", history.title)
        val more = BoardList.entries(board, open, now, reads, showingMore = setOf(TaskStatus.TODO))
        assertEquals(17, more.filterIsInstance<BoardListEntry.Card>().size)
        assertTrue(more.none { it is BoardListEntry.ShowMore })
        assertNull(BoardSectionCut.cut(TaskBoardColumn(TaskStatus.DONE, emptyList()), reads, now).history)
        // No count in parentheses anywhere.
        assertFalse(BoardSectionCut.showMoreTitle(4).contains("("))
        assertFalse(history.title.contains("("))
    }

    @Test
    fun `history groups by when it landed, and searches and filters by area`() {
        val rows = listOf(
            row("old", 40 * day, title = "Mac: an old one"),
            row("tue", 3 * day, title = "Daemon: tuesday"),
            row("yday", day, title = "Mac: yesterday"),
            row("now", hour, title = "Phones: just now"),
        )
        val groups = BoardHistory.groups(rows, now, utc, WeekFields.ISO)
        assertEquals(
            listOf(BoardHistory.Period.TODAY, BoardHistory.Period.YESTERDAY, BoardHistory.Period.THIS_WEEK, BoardHistory.Period.EARLIER),
            groups.map { it.period },
        )
        assertEquals(listOf("Today", "Yesterday", "This Week", "Earlier"), groups.map { it.period.title })
        assertEquals("Mac", BoardHistory.area("Mac: x"))
        assertNull(BoardHistory.area("No area here"))
        assertEquals(listOf("Mac", "Daemon", "Phones"), BoardHistory.areas(rows))
        assertEquals(listOf("old", "yday"), BoardHistory.filter(rows, "", "Mac").map { it.key })
        assertEquals(listOf("tue"), BoardHistory.filter(rows, "TUESDAY daemon").map { it.key })
        assertEquals("7:00 AM", BoardHistory.landed(rows[3], now, utc, Locale.US, WeekFields.ISO))
        assertEquals("Dec 6, 2026", BoardHistory.landed(rows[0], now, utc, Locale.US, WeekFields.ISO))
    }
}
