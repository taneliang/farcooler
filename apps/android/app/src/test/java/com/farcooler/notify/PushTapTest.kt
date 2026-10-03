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

/** A task notice's tap (ov-94): its task, like a decision's, never a terminal. */
class TaskPushTapTest {
    private fun tap(vararg extras: Pair<String, String>) = PushTap.from(mapOf(*extras)::get)

    @Test
    fun aTaskNoticeOpensItsTaskOnItsRunner() {
        assertEquals(
            PushTap.Task("ov-90", "r-1"),
            tap("kind" to "task", "task" to "ov-90", "runner" to "r-1", "event" to "review", "noticeId" to "t:r-1:ov-90"),
        )
    }

    @Test
    fun aTaskNoticeWithNoTaskIsNothing() {
        assertNull(tap("kind" to "task", "noticeId" to "t:r-1:ov-90"))
    }
}
