package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Every phone empty state that explains something says it as a short lede and
 * rows, never a paragraph (ov-245). The Mac's `EmptyStateCopyTests` shape: a
 * lede of one short sentence, two or three rows of an icon and at most eight
 * words each, and no layout talk. A board on an old runner is the one exception,
 * a single line short enough to need no rows.
 */
class PhoneEmptyStatesTest {
    private fun words(text: String) = text.split(Regex("\\s+")).filter { it.isNotEmpty() }.size

    private fun expectScannable(copy: PhoneEmptyCopy) {
        assertTrue("a lede of ${words(copy.lede)} words: ${copy.lede}", words(copy.lede) <= 12)
        assertFalse("more than one sentence: ${copy.lede}", copy.lede.dropLast(1).contains("."))
        if (copy.rows.isEmpty()) return
        assertTrue("rows, not a paragraph", copy.rows.size in 2..3)
        for (row in copy.rows) {
            assertTrue("a row of ${words(row.text)} words: ${row.text}", words(row.text) <= 8)
            assertFalse("a row is a list item: ${row.text}", row.text.endsWith("."))
            assertTrue("sentence case: ${row.text}", row.text.first().isUpperCase())
            // Sentence case on Android: no word after the first is capitalized
            // except one that is always, like the product's.
            assertFalse("title case: ${row.text}", Regex("\\s[A-Z][a-z]+\\s[A-Z][a-z]+").containsMatchIn(row.text))
        }
        val all = (listOf(copy.lede) + copy.rows.map { it.text }).joinToString(" ").lowercase()
        for (layout in listOf("left", "right", "title bar", "sidebar", "below", "above")) {
            assertFalse("describes the layout: $layout", all.contains(layout))
        }
    }

    @Test
    fun everyPhoneEmptyStateIsScannable() {
        PhoneEmptyStates.all.forEach(::expectScannable)
        assertEquals(7, PhoneEmptyStates.all.size)
    }

    /** The Mac's own words where the Mac has the same state, so a runner reads the same on every device. */
    @Test
    fun theStatesTheMacAlsoHasSayWhatTheMacSays() {
        assertEquals("An orchestrator runs this workspace’s board.", PhoneEmptyStates.NO_ORCHESTRATOR.lede)
        assertEquals(
            listOf(
                "Tell it what you want done",
                "It plans tasks and puts agents on them",
                "It asks you when it needs a decision",
            ),
            PhoneEmptyStates.NO_ORCHESTRATOR.rows.map { it.text },
        )
        assertEquals("A worktree is where an agent works.", PhoneEmptyStates.NO_WORKTREES.lede)
        assertTrue(PhoneEmptyStates.NO_REPOSITORIES.lede.startsWith("Add the repository"))
    }

    /** AgentKit's rows, word for word. */
    @Test
    fun theRowsReadTheSameAsTheIPhones() {
        val iphone = listOf(
            "Tell it what you want done", "It plans tasks and puts agents on them",
            "It asks you when it needs a decision", "Each workspace is one line of work",
            "Start a workspace’s orchestrator to begin", "Each agent gets its own folder and branch",
            "Your checkout changes only when you merge", "It has its own folder and branch",
            "Each piece of work becomes a task", "Start the orchestrator first",
        )
        val ours = PhoneEmptyStates.all.flatMap { it.rows.map { row -> row.text } }.toSet()
        assertEquals(iphone.toSet(), ours)
    }
}
