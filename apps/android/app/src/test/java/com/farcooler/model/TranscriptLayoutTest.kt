package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Where the bottom of the agent transcript is, and when it stops following it.
 *
 * Both of these were arithmetic inside a composable, which is the reason they
 * were wrong: nothing on this screen has drawn a frame in a test, so a number
 * computed in a `LazyColumn` builder is a number nobody can check. They are
 * pure functions now for the same reason `ChangesScreenTest` gives about the
 * sentences that screen says.
 *
 * What is deliberately NOT claimed here: that the transcript LOOKS right while
 * a reply streams. That needs a device, and this suite has never had one.
 */
class TranscriptLayoutTest {
    private fun rows(n: Int): List<TranscriptRow> =
        (0 until n).map { TranscriptRow(it, TranscriptRow.Kind.Gap(GapReason.Unparsed)) }

    // ---- where the bottom is ----

    /**
     * The off-by-one, in the four shapes the item list actually takes.
     *
     * The screen scrolled to `transcript.rows.size`. That is the last index
     * only when EXACTLY ONE of the two optional items — the failed-poll banner
     * above the rows, the working row below them — is present, which is two of
     * these four cases. Each assertion below is `rows.size` in exactly one
     * case and something else in the other three, and the old code had no way
     * to be told apart from the right answer.
     */
    @Test
    fun `the last index counts both of the optional items`() {
        val five = rows(5)
        // Neither: the old code aimed at 5, one PAST the end.
        assertEquals(5, transcriptItems(false, five, false).lastIndex)
        // The working row only: the old code aimed at 5, which was that row.
        assertEquals(6, transcriptItems(false, five, true).lastIndex)
        // The banner only: the old code aimed at 5, which was the last ROW —
        // and scrolling to a row puts its TOP at the top of the screen, so a
        // final message taller than the phone hid its own tail.
        assertEquals(6, transcriptItems(true, five, false).lastIndex)
        // Both: the old code aimed at 5, two short.
        assertEquals(7, transcriptItems(true, five, true).lastIndex)
    }

    /** The end item is always there, so `lastIndex` is never -1. */
    @Test
    fun `an empty transcript still has an end to scroll to`() {
        assertEquals(listOf(TranscriptItem.End), transcriptItems(false, emptyList(), false))
        assertEquals(0, transcriptItems(false, emptyList(), false).lastIndex)
    }

    @Test
    fun `the items come in the order the screen draws them`() {
        val items = transcriptItems(true, rows(2), true)
        assertEquals(
            listOf(
                TranscriptItem.Notice,
                TranscriptItem.Row(rows(2)[0]),
                TranscriptItem.Row(rows(2)[1]),
                TranscriptItem.Working,
                TranscriptItem.End,
            ),
            items,
        )
    }

    /**
     * A `LazyColumn` over duplicate keys renders blank bands and repeats rows,
     * which is the failure [TranscriptRow]'s own doc comment describes. Four
     * kinds of item in one list is four chances to collide.
     */
    @Test
    fun `every item has its own key`() {
        val keys = transcriptItems(true, rows(30), true).map { it.key }
        assertEquals(keys.size, keys.toSet().size)
    }

    // ---- when it stops following ----

    private fun at(index: Int, offset: Int = 0, atEnd: Boolean = false) =
        TailSample(firstIndex = index, firstOffset = offset, atEnd = atEnd)

    /**
     * The defect this replaces, stated as the case that reaches it.
     *
     * `AgentStream.pump` applies a whole batch and bumps the revision once, so
     * one 700 ms poll routinely lands several rows: a message, a tool call, its
     * result. Nothing about that moves the list — the reader's position is
     * exactly where it was — and the old predicate, `last >= total - 2`, went
     * false anyway as soon as two items landed below the fold. Following was
     * then off, so the list never returned to the tail and the predicate never
     * recovered.
     */
    @Test
    fun `rows arriving below the fold do not stop the transcript following`() {
        val parked = at(index = 40, offset = 120)
        assertTrue(TranscriptTail.following(was = true, previous = parked, now = parked))
    }

    /** Only a reader moves the list backwards; the screen's own scroll does not. */
    @Test
    fun `scrolling up detaches`() {
        assertFalse(TranscriptTail.following(true, at(40, 120), at(39, 900)))
        assertFalse(TranscriptTail.following(true, at(40, 120), at(40, 119)))
    }

    /** Forward is either the reader chasing the tail or the screen's own scroll. */
    @Test
    fun `scrolling down does not detach on its own`() {
        assertTrue(TranscriptTail.following(true, at(40, 120), at(41, 0)))
        assertFalse(TranscriptTail.following(false, at(40, 120), at(41, 0)))
    }

    /** Reaching the bottom is how a reader who scrolled away comes back. */
    @Test
    fun `the end coming on screen re-attaches`() {
        assertTrue(TranscriptTail.following(false, at(10, 0), at(41, 0, atEnd = true)))
    }

    /**
     * Re-attaching is tested FIRST, and this is the case that needs it: the
     * working row goes away at the end of a turn, or a row's height resolves
     * shorter than it was measured, and the list walks backwards while the end
     * stays on screen. That is not somebody scrolling up.
     */
    @Test
    fun `the end staying on screen outranks the list moving backwards`() {
        assertTrue(TranscriptTail.following(true, at(41, 300), at(40, 80, atEnd = true)))
    }
}
