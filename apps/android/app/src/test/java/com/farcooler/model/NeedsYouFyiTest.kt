package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * An FYI (a member kept a ruling, ov-756) is a row in the list and nothing else: it adds to no count, and a
 * list of nothing but FYIs still says nothing needs you.
 *
 * Mutation: drop the `isCounted` test from `NeedsYouItems.count` or from `NeedsYou.nothingNeedsYou`. Red.
 */
class NeedsYouFyiTest {
    private val fyi = NeedsYouItem(id = "fyi:kept:w:bob:1", kind = "fyi", rank = 600, workspaceId = "w1")
    private val ask = NeedsYouItem(id = "ask:1", kind = "ask", rank = 5, workspaceId = "w1")

    private fun runner(vararg items: NeedsYouItem) =
        NeedsYouRunner("studio", "studio", RunnerNeedsYou(items.toList()), Fleet(workspaces = emptyList()), emptyList())

    @Test
    fun `an FYI is placed as its own kind and adds to no workspace count`() {
        assertEquals(NeedsYouKind.FYI, fyi.kindValue)
        assertFalse(fyi.isCounted)
        val merged = NeedsYouItems.merge(mapOf("studio" to listOf(ask, fyi)))
        assertEquals(2, merged.size)
        assertEquals(1, NeedsYouItems.count(merged, "studio", "w1"))
    }

    @Test
    fun `a list of only FYIs still says nothing needs you, and an ask does not`() {
        val only = runner(fyi)
        assertTrue(NeedsYou.nothingNeedsYou(listOf(only), NeedsYou.rows(listOf(only))))
        val both = runner(fyi, ask)
        assertFalse(NeedsYou.nothingNeedsYou(listOf(both), NeedsYou.rows(listOf(both))))
    }

    @Test
    fun `an FYI has no button`() {
        assertTrue(NeedsYouAnswer.buttons(fyi, mayAnswer = true).isEmpty())
    }
}
