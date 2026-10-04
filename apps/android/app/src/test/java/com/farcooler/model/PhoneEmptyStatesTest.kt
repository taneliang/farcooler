package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Every phone empty state that explains something says it as a short lede and
 * rows, never a paragraph (ov-245). The Mac's `EmptyStateCopyTests` shape: a
 * lede of one short sentence, one to three rows of an icon and at most eight
 * words each, and no layout talk. A board on an old runner is the one exception,
 * a single line short enough to need no rows.
 */
class PhoneEmptyStatesTest {
    private fun words(text: String) = text.split(Regex("\\s+")).filter { it.isNotEmpty() }.size

    private fun expectScannable(copy: PhoneEmptyCopy) {
        assertTrue("a lede of ${words(copy.lede)} words: ${copy.lede}", words(copy.lede) <= 12)
        assertFalse("more than one sentence: ${copy.lede}", copy.lede.dropLast(1).contains("."))
        if (copy.rows.isEmpty()) return
        assertTrue("rows, not a paragraph", copy.rows.size in 1..3)
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
        assertEquals(
            "An orchestrator turns your requests into tasks and starts agents on them.",
            PhoneEmptyStates.NO_ORCHESTRATOR.lede,
        )
        assertEquals(
            listOf("Tell it what you want built", "Anything it can’t decide comes to you"),
            PhoneEmptyStates.NO_ORCHESTRATOR.rows.map { it.text },
        )
        assertEquals("A worktree gives an agent its own folder and branch.", PhoneEmptyStates.NO_WORKTREES.lede)
        assertTrue(PhoneEmptyStates.NO_REPOSITORIES.lede.startsWith("Add the repository"))
    }

    /** AgentKit's rows, word for word. */
    @Test
    fun theRowsReadTheSameAsTheIPhones() {
        val iphone = listOf(
            "Tell it what you want built", "Anything it can’t decide comes to you",
            "Start an orchestrator and give it work", "Each agent gets its own folder and branch",
            "Your files don’t change until you merge", "Finished work waits here for your review",
            "Start the orchestrator, then give it work",
        )
        val ours = PhoneEmptyStates.all.flatMap { it.rows.map { row -> row.text } }.toSet()
        assertEquals(iphone.toSet(), ours)
    }
}
