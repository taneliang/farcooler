package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Unread (ov-104, ov-113) on Android: AgentKit's `BoardUnreadTests` and
 * `BoardReadsKeeperTests` cases for the summary, copied so the Mac, the iPhone
 * and this phone say the same thing about the same board.
 */
class BoardSummaryTest {
    private val now = 1_800_000_000_000L
    private val hour = 3_600_000L
    private val day = 24 * hour

    private fun unreadSince(start: Long) = BoardReads(start - 1)

    private fun row(key: String, status: TaskStatus, since: Long, created: Long? = null) = TaskRow(
        id = "id-$key", key = key, title = "Title $key", status = status, statusSince = now - since,
        createdAt = created?.let { now - it }, updatedAt = now - since,
    )

    private fun note(id: String, kind: TaskNoteKind, ago: Long, body: String = "body") = TaskNoteRow(id, kind, now - ago, body)

    @Test
    fun openingATicketClearsItsUnreadItemsAndOnlyItsOwn() {
        var reads = BoardReads(now - day)
        val fin = row("fin", TaskStatus.DONE, hour)
        val new = row("new", TaskStatus.TODO, 2 * hour, created = 2 * hour)
        val notes = mapOf("id-new" to listOf(note("n1", TaskNoteKind.COMMENT, 30 * 60_000)))
        val before = BoardSummary.make(listOf(fin, new), notes, reads)
        assertEquals(listOf("fin"), before.finished.map { it.key })
        assertEquals(listOf("new"), before.created.map { it.key })
        assertEquals(listOf("new"), before.activity.map { it.key })

        reads = reads.open(new, now)
        val after = BoardSummary.make(listOf(fin, new), notes, reads)
        assertTrue(after.created.isEmpty())
        assertTrue(after.activity.isEmpty())
        assertEquals("another ticket's item stays", listOf("fin"), after.finished.map { it.key })

        // A note written after it was opened is unread again.
        val later = mapOf("id-new" to listOf(note("n2", TaskNoteKind.FINDING, -60_000)))
        assertEquals(listOf("n2"), BoardSummary.make(listOf(new), later, reads).activity.map { it.noteId })

        // A runner clock ahead of this one: what it stamped is read on opening.
        val future = row("f", TaskStatus.DONE, -10 * 60_000)
        assertFalse(BoardReads(now - day).open(future, now).finishedUnread(future))

        // Mark All as Read.
        val all = reads.markAllReadOnDeviceClock(listOf(fin, new), now)
        assertTrue(BoardSummary.make(listOf(fin, new), notes, all).isEmpty)
        assertTrue(all.opened.isEmpty())
    }

    @Test
    fun activityIsOneEntryPerTicketItsNewestNoteWholeAndPlusNForTheOlder() {
        val rows = listOf(row("a", TaskStatus.IN_PROGRESS, hour), row("b", TaskStatus.IN_PROGRESS, 2 * hour), row("x", TaskStatus.CANCELLED, hour))
        val notes = mapOf(
            "id-a" to listOf(
                note("a1", TaskNoteKind.DECISION, 50 * 60_000, "Use SQLite"),
                note("a2", TaskNoteKind.COMMENT, 10 * 60_000, "Looks\n\ngood   to me"),
                note("a3", TaskNoteKind.PROGRESS, 30 * 60_000),
                note("a4", TaskNoteKind.STATUS_CHANGE, 5 * 60_000),
                note("old", TaskNoteKind.FINDING, 3 * day),
            ),
            "id-b" to listOf(note("b1", TaskNoteKind.QUESTION, 20 * 60_000, "Which?")),
            "id-x" to listOf(note("x1", TaskNoteKind.FINDING, 60_000)),
        )
        val activity = BoardSummary.make(rows, notes, unreadSince(now - day)).activity
        assertEquals(listOf("a", "b"), activity.map { it.key })
        assertEquals("a2", activity[0].noteId)
        assertEquals(TaskNoteKind.COMMENT, activity[0].kind)
        assertEquals("Looks good to me", activity[0].text)
        assertEquals(2, activity[0].more)
        assertEquals("+2 more", activity[0].moreLine)
        assertEquals(0, activity[1].more)
        assertNull(activity[1].moreLine)
    }

    @Test
    fun identityIsTheTicketsAndTheNotes() {
        val a = row("a", TaskStatus.DONE, hour)
        val notes = mapOf("id-a" to listOf(note("n1", TaskNoteKind.COMMENT, 60_000)))
        val one = BoardSummary.make(listOf(a), notes, unreadSince(now - day))
        assertEquals(listOf("id-a/done"), one.finished.map { it.id })
        assertEquals(listOf("id-a/activity"), one.activity.map { it.id })
        val two = BoardSummary.make(listOf(a), mapOf("id-a" to notes.getValue("id-a") + note("n2", TaskNoteKind.DECISION, 30)), unreadSince(now - day))
        assertEquals(one.activity.map { it.id }, two.activity.map { it.id })
        assertEquals("n2", two.activity.first().noteId)
    }

    @Test
    fun aMovedTicketAndANewOneAreListedUnderTheirOwnGroups() {
        val rows = listOf(
            row("dec", TaskStatus.NEEDS_DECISION, hour),
            row("rev", TaskStatus.IN_REVIEW, 2 * hour),
            row("new", TaskStatus.TODO, 3 * hour, created = 3 * hour),
            row("gone", TaskStatus.CANCELLED, hour, created = hour),
        )
        val summary = BoardSummary.make(rows, reads = unreadSince(now - day))
        assertEquals(listOf("dec", "rev"), summary.moved.map { it.key })
        assertEquals(listOf("Needs Decision", "In Review"), summary.moved.map { it.detail })
        assertEquals(listOf("new"), summary.created.map { it.key })
        assertTrue(summary.finished.isEmpty())
        assertEquals(3, summary.taskCount)
    }

    @Test
    fun theTicketsWorthReadingForNotesAreTheUnreadOnesNewestFirstAtMostTen() {
        val rows = (1..12).map { row("t$it", TaskStatus.IN_PROGRESS, it * 60_000L) } + row("c", TaskStatus.CANCELLED, 1000)
        val picked = BoardSummary.noteCandidates(rows, unreadSince(now - day))
        assertEquals(10, picked.size)
        assertEquals("t1", picked.first().key)
        assertFalse(picked.any { it.status == TaskStatus.CANCELLED })
        assertTrue(BoardSummary.noteCandidates(rows, BoardReads(now)).isEmpty())
    }

    @Test
    fun theLinesSayWhatHappenedAndWhen() {
        val done = BoardSummary.Item("t/done", "t", "k", "T", null, now - 2 * hour - 100_000)
        assertEquals("Done 2h ago", done.whenSaid(now))
        assertEquals("Added 3m ago", BoardSummary.Item("t/created", "t", "k", "T", null, now - 200_000).whenSaid(now))
        assertEquals("Needs Decision just now", BoardSummary.Item("t/needs_decision", "t", "k", "T", "Needs Decision", now - 10_000).whenSaid(now))
        val activity = BoardSummary.Activity("t", "k", "T", "n", TaskNoteKind.FINDING, "x", now - 12 * 60_000, 2)
        assertEquals("12m ago · +2 more", activity.foot(now))
    }

    @Test
    fun groupsAreCutAtFiveAndMarkAllReadSaysWhetherItReachesEveryDevice() {
        assertEquals(5 to 2, BoardSummary.capped((1..7).toList()).let { it.first.size to it.second })
        assertEquals("1 task will be marked as read on this device only.", BoardSummary.markAllReadMessage(1, false))
        assertEquals("68 tasks will be marked as read on all your devices.", BoardSummary.markAllReadMessage(68, true))
        assertEquals("You’re all caught up.", BoardSummary.NOTHING)
    }

    @Test
    fun notesDecodeAndAKindThisBuildCantReadIsSkipped() {
        val notes = TaskNotes.decode(
            """{"notes":[{"id":"a","kind":"comment","at":5,"body":"hi"},{"id":"b","kind":"from_the_future","at":6,"body":"?"},{"id":"c","kind":"status_change","at":7,"body":"moved"}]}""",
        )
        assertEquals(listOf("a", "c"), notes.map { it.id })
        assertTrue(notes[1].kind.isMachineWritten)
        assertEquals(emptyList<TaskNoteRow>(), TaskNotes.decode("""{"blocks":[]}"""))
    }
}
