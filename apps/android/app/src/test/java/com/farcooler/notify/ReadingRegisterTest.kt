package com.farcooler.notify

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Which terminal is being read, and who may give it back (ov-69). Two screens
 * decide it, the worktree's and the workspace's orchestrator tab, and each used
 * to write nothing-being-read on its way out, which wiped a claim the other had
 * made since. The iPhone's `Notifier.claim` and `release`, and the same rules.
 */
class ReadingRegisterTest {

    /** A claim is what is read, and claiming nothing is the Changes tab or the grid. */
    @Test
    fun aClaimIsWhatIsRead() {
        val reading = ReadingRegister()
        assertNull(reading.current)
        reading.claim("a")
        assertEquals("a", reading.current)
        reading.claim(null)
        assertNull(reading.current)
    }

    /** A release gives back what it names. */
    @Test
    fun aReleaseGivesBackWhatItNames() {
        val reading = ReadingRegister()
        reading.claim("a")
        assertTrue(reading.release("a"))
        assertNull(reading.current)
    }

    /**
     * A screen leaving after another has claimed gives back nothing: the
     * orchestrator's tab claims while the worktree under it is still leaving.
     */
    @Test
    fun aReleaseAfterAnotherClaimIsANoOp() {
        val reading = ReadingRegister()
        reading.claim("worktree-pane")
        reading.claim("orchestrator")
        assertFalse(reading.release("worktree-pane"))
        assertEquals("orchestrator", reading.current)
    }
}
