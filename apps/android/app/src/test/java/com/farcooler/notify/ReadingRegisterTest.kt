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

    /** A stand-in for a connection: what the runner would be told. */
    private class Sink : VisibleTerminalSink {
        override var visibleTerminal: String? = null
    }

    /** A claim reaches both the banner rule and the runner. */
    @Test
    fun aClaimReachesBothSides() {
        val reading = ReadingRegister()
        val runner = Sink()
        reading.claim(runner, "a")
        assertEquals("a", reading.current)
        assertEquals("a", runner.visibleTerminal)
        reading.claim(runner, null)
        assertNull(reading.current)
        assertNull(runner.visibleTerminal)
    }

    /**
     * The orchestrator's tab claims while the worktree under it is still
     * leaving: the leaving screen's release wipes neither side.
     */
    @Test
    fun aLeavingScreenWipesNeitherSide() {
        val reading = ReadingRegister()
        val runner = Sink()
        reading.claim(runner, "worktree-pane")
        reading.claim(runner, "orchestrator")
        reading.release(runner, "worktree-pane")
        assertEquals("orchestrator", reading.current)
        assertEquals("orchestrator", runner.visibleTerminal)
        reading.release(runner, "orchestrator")
        assertNull(reading.current)
        assertNull(runner.visibleTerminal)
    }

    /**
     * Each connection gives back only its own claim: runner A keeps nothing
     * stale when a pane on runner B claimed the register, and B's is untouched
     * by A's release.
     */
    @Test
    fun aConnectionGivesBackOnlyItsOwnClaim() {
        val reading = ReadingRegister()
        val a = Sink()
        val b = Sink()
        reading.claim(a, "on-a")
        reading.claim(b, "on-b")
        reading.release(a, "on-a")
        assertNull(a.visibleTerminal)
        assertEquals("on-b", b.visibleTerminal)
        assertEquals("on-b", reading.current)
        // And a sink that has since been given another pane keeps it.
        a.visibleTerminal = "other"
        a.releaseVisible("on-a")
        assertEquals("other", a.visibleTerminal)
    }
}
