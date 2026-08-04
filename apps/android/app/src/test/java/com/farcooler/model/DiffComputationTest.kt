package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The third copy of this algorithm — Swift has two, this is Kotlin's — so the
 * only thing holding them to the same answers is a suite that asserts the
 * answers rather than the shape of the code.
 */
class DiffComputationTest {
    private fun render(lines: List<DiffComputation.Line>) = lines.joinToString("\n") {
        val marker = when (it.kind) {
            DiffComputation.Kind.CONTEXT -> " "
            DiffComputation.Kind.ADDED -> "+"
            DiffComputation.Kind.REMOVED -> "-"
        }
        "$marker${it.text}"
    }

    @Test
    fun oneChangedLineInsideAnUnchangedFileIsOneAddAndOneRemove() {
        val lines = DiffComputation.compute(
            old = "a\nb\nc",
            new = "a\nB\nc",
        )
        assertEquals(" a\n-b\n+B\n c", render(lines))
    }

    @Test
    fun lineNumbersCountEachSideSeparately() {
        // The gutter is what makes a diff readable; a removed line has no new
        // number and an added one has no old number.
        val lines = DiffComputation.compute(old = "a\nb", new = "a\nb\nc")
        val added = lines.last()
        assertEquals(DiffComputation.Kind.ADDED, added.kind)
        assertEquals(null, added.oldNumber)
        assertEquals(3, added.newNumber)
    }

    @Test
    fun creatingAFileIsAllAdditions() {
        val lines = DiffComputation.compute(old = "", new = "one\ntwo")
        assertEquals("+one\n+two", render(lines))
    }

    @Test
    fun emptyingAFileIsAllRemovals() {
        val lines = DiffComputation.compute(old = "one\ntwo", new = "")
        assertEquals("-one\n-two", render(lines))
    }

    @Test
    fun anUnchangedFileIsAllContext() {
        val lines = DiffComputation.compute(old = "a\nb", new = "a\nb")
        assertTrue(lines.all { it.kind == DiffComputation.Kind.CONTEXT })
    }

    @Test
    fun aVeryLargeRewriteFallsBackToAFlatReplaceRatherThanHanging() {
        // Past 160,000 cells the LCS table is not worth its cost, and the
        // fallback is still a correct diff — just not the minimal one. What it
        // must never be is slow enough to freeze the screen it is drawn on.
        val old = (0 until 500).joinToString("\n") { "old $it" }
        val new = (0 until 500).joinToString("\n") { "new $it" }
        val lines = DiffComputation.compute(old, new)
        assertEquals(500, lines.count { it.kind == DiffComputation.Kind.REMOVED })
        assertEquals(500, lines.count { it.kind == DiffComputation.Kind.ADDED })
    }

    @Test
    fun idsAreUniqueSoAKeyedListDoesNotCollapseRows() {
        // Two identical lines are two lines. A list keyed on content would draw
        // one of them.
        val lines = DiffComputation.compute(old = "x\nx\nx", new = "x\nx\nx\nx")
        assertEquals(lines.size, lines.map { it.id }.toSet().size)
    }
}
