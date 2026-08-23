package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * How one file's patch is cut up, and the three off-by-ones in it.
 *
 * The whole reason this arithmetic lives in `model/` and not inside the
 * composable: hunk boundaries are recovered from a jump in the line NUMBERS
 * rather than from a `@@` header, and every one of the boundary rules — where a
 * gap is, what a fold keeps, when a fold is worth drawing — is a comparison that
 * is one wrong in either direction and looks plausible on screen either way.
 */
class DiffLayoutTest {
    private var id = 0

    private fun line(
        kind: DiffComputation.Kind,
        old: Int?,
        new: Int?,
        text: String = "x",
    ) = DiffComputation.Line(id++, kind, old, new, text)

    private fun context(old: Int, new: Int) = line(DiffComputation.Kind.CONTEXT, old, new)
    private fun added(new: Int) = line(DiffComputation.Kind.ADDED, null, new)
    private fun removed(old: Int) = line(DiffComputation.Kind.REMOVED, old, null)

    // ---- where the hunks are ----

    @Test
    fun `a jump in the new numbering is where one hunk ended and the next began`() {
        val lines = listOf(
            context(10, 10), added(11), context(11, 12),
            // 12 → 40 is the gap the `@@` header used to say.
            context(39, 40), removed(40), context(40, 41),
        )
        val hunks = DiffLayout.hunks(lines)
        assertEquals(2, hunks.size)
        assertEquals(listOf(0, 1), hunks.map { it.id })
        assertEquals(3, hunks[0].lines.size)
        assertEquals(3, hunks[1].lines.size)
        assertEquals(10, hunks[0].firstLine)
        assertEquals(12, hunks[0].lastLine)
        assertEquals("Lines 10-12", hunks[0].rangeLabel)
        assertEquals("Lines 40-41", hunks[1].rangeLabel)
    }

    @Test
    fun `consecutive numbering is one hunk`() {
        val lines = listOf(context(1, 1), added(2), context(2, 3))
        assertEquals(1, DiffLayout.hunks(lines).size)
    }

    /**
     * A removed line has no new number and must not be read as a gap — otherwise
     * every deletion would split the hunk it is in.
     */
    @Test
    fun `a line with no new number is not a boundary`() {
        val lines = listOf(context(1, 1), removed(2), removed(3), context(4, 2))
        assertEquals(1, DiffLayout.hunks(lines).size)
    }

    @Test
    fun `a deleted file has no new-side numbering and therefore no range label`() {
        val hunk = DiffLayout.hunks(listOf(removed(1), removed(2), removed(3))).single()
        assertNull(hunk.firstLine)
        assertNull(hunk.lastLine)
        assertNull(hunk.rangeLabel)
    }

    @Test
    fun `a one-line hunk says line rather than lines`() {
        val hunk = DiffLayout.hunks(listOf(added(7))).single()
        assertEquals("Line 7", hunk.rangeLabel)
    }

    @Test
    fun `nothing at all is no hunks`() {
        assertEquals(emptyList<DiffLayout.Hunk>(), DiffLayout.hunks(emptyList()))
    }

    // ---- what a fold keeps ----

    /**
     * Five is the threshold, and it is chosen so this fires on hunks git MERGED:
     * with `-U3` a run of six unchanged lines between two changes is what two
     * nearby edits look like joined together, which is the shape an LLM refactor
     * produces twenty times down one file.
     */
    @Test
    fun `a long run between two changes folds to head, fold, tail`() {
        val lines = mutableListOf(added(1))
        for (n in 2..11) lines.add(context(n - 1, n))
        lines.add(added(12))
        val segments = DiffLayout.segments(DiffLayout.hunks(lines).single())
        // The change, two kept above the gap, the fold, two kept below, the
        // change on the other side.
        assertEquals(5, segments.size)
        assertEquals(listOf(1), segments[0].lines.map { it.newNumber })
        assertNull(segments[0].folded)
        assertEquals(listOf(2, 3), segments[1].lines.map { it.newNumber })
        // Six hidden: ten unchanged, two kept at each end.
        assertEquals(6, segments[2].folded)
        assertEquals(listOf(4, 5, 6, 7, 8, 9), segments[2].lines.map { it.newNumber })
        assertEquals(listOf(10, 11), segments[3].lines.map { it.newNumber })
        assertNull(segments[3].folded)
        assertEquals(listOf(12), segments[4].lines.map { it.newNumber })
        assertEquals(listOf(0, 1, 2, 3, 4), segments.map { it.id })
    }

    /**
     * At the very start and end of a hunk there is nothing above or below to give
     * a change its place, so nothing is kept there.
     */
    @Test
    fun `a run at the start of a hunk keeps no head`() {
        val lines = mutableListOf<DiffComputation.Line>()
        for (n in 1..8) lines.add(context(n, n))
        lines.add(added(9))
        val segments = DiffLayout.segments(DiffLayout.hunks(lines).single())
        assertEquals(3, segments.size)
        assertEquals(6, segments[0].folded)
        assertEquals(listOf(1, 2, 3, 4, 5, 6), segments[0].lines.map { it.newNumber })
        assertEquals(listOf(7, 8), segments[1].lines.map { it.newNumber })
        assertNull(segments[2].folded)
    }

    @Test
    fun `a run shorter than the threshold is drawn whole`() {
        val lines = listOf(added(1), context(1, 2), context(2, 3), context(3, 4), added(5))
        val segments = DiffLayout.segments(DiffLayout.hunks(lines).single())
        assertEquals(3, segments.size)
        assertEquals(listOf(null, null, null), segments.map { it.folded })
    }

    /**
     * Long enough to reach the threshold and still not worth a fold: five
     * unchanged lines with two kept at each end hides one, and a control that
     * says "1 more line" is bigger than the line it replaces.
     */
    @Test
    fun `a run that would hide fewer than two lines is drawn whole`() {
        val lines = mutableListOf(added(1))
        for (n in 2..6) lines.add(context(n - 1, n))
        lines.add(added(7))
        val segments = DiffLayout.segments(DiffLayout.hunks(lines).single())
        assertEquals(3, segments.size)
        assertNull(segments[1].folded)
        assertEquals(5, segments[1].lines.size)
    }

    /** A new file has no unchanged lines at all, so nothing ever folds in one. */
    @Test
    fun `a file of nothing but additions is one drawn segment`() {
        val lines = (1..40).map { added(it) }
        val segments = DiffLayout.segments(DiffLayout.hunks(lines).single())
        assertEquals(1, segments.size)
        assertEquals(40, segments[0].lines.size)
        assertNull(segments[0].folded)
    }

    /** Every line of the hunk survives into the segments, folded or not. */
    @Test
    fun `folding hides lines from the screen and never from the model`() {
        val lines = mutableListOf(added(1))
        for (n in 2..15) lines.add(context(n - 1, n))
        lines.add(added(16))
        val hunk = DiffLayout.hunks(lines).single()
        val segments = DiffLayout.segments(hunk)
        assertEquals(hunk.lines.size, segments.sumOf { it.lines.size })
        assertEquals(hunk.lines.map { it.id }, segments.flatMap { it.lines }.map { it.id })
    }
}
