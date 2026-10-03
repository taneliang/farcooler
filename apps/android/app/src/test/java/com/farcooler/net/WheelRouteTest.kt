package com.farcooler.net

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** A swipe on the primary screen scrolls the pane, even with mouse reporting on. See [WheelRoute]. */
class WheelRouteTest {
    /** The bug: a program with the mouse on, on the primary screen, swallowed every swipe. */
    @Test
    fun `the primary screen scrolls locally even when the program wants the mouse`() {
        assertFalse(WheelRoute.toProgram(alternateScreen = false, programTakesWheel = true))
    }

    @Test
    fun `the alternate screen gives the wheel to a program that takes it`() {
        assertTrue(WheelRoute.toProgram(alternateScreen = true, programTakesWheel = true))
    }

    @Test
    fun `a program that declines the wheel leaves it local`() {
        assertFalse(WheelRoute.toProgram(alternateScreen = true, programTakesWheel = false))
        assertFalse(WheelRoute.toProgram(alternateScreen = false, programTakesWheel = false))
    }
}
