package com.farcooler.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** A watched pane keeps the font's own size and scrolls, instead of shrinking to the phone's width (ov-976). */
class AgentWatchLayoutTest {
    @Test fun `a wide pane is drawn wider than a phone`() {
        val (width, height) = AgentWatchLayout.contentSize(columns = 120, rows = 40, cellWidth = 24f, cellHeight = 48f, padding = 18f)
        assertEquals(120 * 24f + 36f, width, 0f)
        assertEquals(40 * 48f + 36f, height, 0f)
        // 960 px is a 320 dp phone at xxhdpi (3x): the pane is three times that, so it scrolls.
        assertTrue(width > 960f)
    }
}
