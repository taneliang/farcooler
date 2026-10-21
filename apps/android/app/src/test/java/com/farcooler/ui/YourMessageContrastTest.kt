package com.farcooler.ui

import androidx.compose.ui.graphics.Color
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** The person's bubble under the system's increased contrast (ov-624). */
class YourMessageContrastTest {
    private fun ratio(a: Color, b: Color): Double {
        fun lum(c: Color): Double {
            fun ch(v: Float) = if (v <= 0.03928f) v / 12.92 else Math.pow((v + 0.055) / 1.055, 2.4)
            return 0.2126 * ch(c.red) + 0.7152 * ch(c.green) + 0.0722 * ch(c.blue)
        }
        val l1 = lum(a)
        val l2 = lum(b)
        return (maxOf(l1, l2) + 0.05) / (minOf(l1, l2) + 0.05)
    }

    @Test
    fun highContrastFaceBeatsBothOthersAndReaches8Point4() {
        val high = ratio(YourMessage.highContrast, YourMessage.text)
        assertTrue("was $high", high >= 8.4)
        assertTrue(high > ratio(YourMessage.light, YourMessage.text))
        assertTrue(high > ratio(YourMessage.dark, YourMessage.text))
        // The other two stay above WCAG AA, as ov-610 recorded.
        assertTrue(ratio(YourMessage.light, YourMessage.text) >= 4.5)
        assertTrue(ratio(YourMessage.dark, YourMessage.text) >= 4.5)
    }

    @Test
    fun highContrastFaceIsChosenWhenContrastIsAboveZero() {
        for (dark in listOf(false, true)) {
            assertEquals(YourMessage.highContrast, YourMessage.fill(dark, highContrast = true))
        }
        assertEquals(Color(0xFF0848A6), YourMessage.highContrast)
    }

    @Test
    fun standardContrastKeepsTheThemeFaces() {
        assertEquals(YourMessage.light, YourMessage.fill(dark = false, highContrast = false))
        assertEquals(YourMessage.dark, YourMessage.fill(dark = true, highContrast = false))
    }
}
