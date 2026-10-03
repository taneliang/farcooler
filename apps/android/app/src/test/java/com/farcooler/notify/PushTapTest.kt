package com.farcooler.notify

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** How a tapped notification routes, from the extras Firebase puts in the launch intent. */
class PushTapTest {
    private fun tap(vararg extras: Pair<String, String>) = PushTap.from(mapOf(*extras)::get)

    @Test
    fun backgroundedDecisionPushWithEmptyTerminalOpensTheTaskOnItsRunner() {
        // The relay sends terminal "" for a task-only push.
        val got = tap("terminal" to "", "kind" to "decision", "task" to "ov-1", "runner" to "r-7")
        assertEquals(PushTap.Task("ov-1", "r-7"), got)
    }

    @Test
    fun decisionPushWithNoTerminalKeyOpensTheTask() {
        assertEquals(PushTap.Task("ov-1", null), tap("kind" to "decision", "task" to "ov-1"))
    }

    @Test
    fun aNamedTerminalStillWinsOverTheTask() {
        assertEquals(PushTap.Terminal("t1"), tap("terminal" to "t1", "kind" to "decision", "task" to "ov-1"))
        assertEquals(PushTap.Terminal("t2"), tap("com.farcooler.terminal" to "t2"))
    }

    @Test
    fun nothingToOpenIsNull() {
        assertNull(tap("terminal" to ""))
        assertNull(tap("kind" to "decision", "task" to ""))
        assertNull(tap("kind" to "done", "task" to "ov-1"))
    }
}
