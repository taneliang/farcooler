package com.farcooler.ui

import com.farcooler.model.BoardDone
import com.farcooler.model.TaskBoard
import com.farcooler.model.TaskBoardColumn
import com.farcooler.model.TaskRow
import com.farcooler.model.TaskStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Done shows the work finished lately, newest first (ov-80); AgentKit's BoardDoneTests, in Kotlin. */
class BoardDoneTest {
    private val now = 1_800_000_000_000L
    private val day = 24L * 60 * 60 * 1000

    private fun done(key: String, agoMs: Long, status: TaskStatus = TaskStatus.DONE) =
        TaskRow(key, key, "Task $key", status, now - agoMs)

    private fun board(vararg rows: TaskRow) =
        TaskBoard(listOf(TaskBoardColumn(TaskStatus.DONE, rows.toList())))

    @Test
    fun `done keeps the last seven days and at least ten, newest first`() {
        val recent = (0 until 12).map { done("r$it", it * 3_600_000L) }
        val old = (0 until 5).map { done("o$it", 30 * day + it) }
        val shown = BoardDone.visible((old + recent).reversed(), showingAll = false, nowMs = now)
        assertEquals((0 until 12).map { "r$it" }, shown.map { it.key })

        val quiet = listOf(done("a", 3_600_000L)) + (0 until 20).map { done("o$it", 30 * day + it) }
        val topped = BoardDone.visible(quiet, showingAll = false, nowMs = now)
        assertEquals(10, topped.size)
        assertEquals("a", topped.first().key)
        assertEquals("o8", topped.last().key)
        assertEquals(21, BoardDone.visible(quiet, showingAll = true, nowMs = now).size)
        assertEquals("Show All Done (21)", BoardDone.showAllTitle(21))
    }

    @Test
    fun `the list offers show all under an open done that hides work`() {
        val rows = (0 until 15).map { done("d$it", 40 * day + it) }
        val toggled = setOf(TaskStatus.DONE)
        val short = BoardList.entries(board(*rows.toTypedArray()), toggled, nowMs = now)
        assertEquals(10, short.filterIsInstance<BoardListEntry.Card>().size)
        val button = short.filterIsInstance<BoardListEntry.ShowAllDone>().single()
        assertEquals("Show All Done (15)", button.title)

        val all = BoardList.entries(board(*rows.toTypedArray()), toggled, nowMs = now, showAllDone = true)
        assertEquals(15, all.filterIsInstance<BoardListEntry.Card>().size)
        assertEquals("Show Recent Done Only", all.filterIsInstance<BoardListEntry.ShowAllDone>().single().title)

        val few = BoardList.entries(board(done("x", 40 * day)), toggled, nowMs = now)
        assertTrue(few.none { it is BoardListEntry.ShowAllDone })
        val canceled = TaskBoard(
            listOf(TaskBoardColumn(TaskStatus.CANCELLED, rows.map { it.copy(status = TaskStatus.CANCELLED) }))
        )
        val c = BoardList.entries(canceled, setOf(TaskStatus.CANCELLED), nowMs = now)
        assertEquals(15, c.filterIsInstance<BoardListEntry.Card>().size)
    }
}
