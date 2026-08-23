package com.farcooler.net

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * When a claim of attention goes out, and when it is held back.
 *
 * The two grounds are different questions and both matter. A CHANGE must leave
 * immediately, because backing out of a pane releases it in the same round trip
 * rather than leaving it silent until the runner's TTL runs out. A repeat must
 * be renewed on the floor, because the runner forgets a claim after ten seconds
 * and a pane somebody has been reading for a minute must not start buzzing them
 * again halfway through.
 */
class WatchingClaimTest {
    private val claim = WatchingClaim()

    /**
     * A link that has been told nothing is not a link claiming nothing. The
     * first call sends whatever it has, so a runner that just came up hears from
     * this client at all.
     */
    @Test
    fun `the first claim on a link always goes`() {
        assertTrue(claim.due(emptyList(), 0))
    }

    @Test
    fun `an unchanged claim waits for the floor`() {
        assertTrue(claim.due(listOf("a"), 0))
        assertFalse(claim.due(listOf("a"), 1_999))
        assertTrue(claim.due(listOf("a"), 2_000))
    }

    /** The floor is a floor on ASKING, so the clock restarts on a skipped send too. */
    @Test
    fun `the floor is measured from the last attempt`() {
        assertTrue(claim.due(listOf("a"), 0))
        assertTrue(claim.due(listOf("a"), 2_500))
        assertFalse(claim.due(listOf("a"), 4_000))
        assertTrue(claim.due(listOf("a"), 4_500))
    }

    /**
     * Moving between tabs releases the pane you left in the same round trip that
     * claims the one you arrived at, which is the whole reason the runner keys
     * this by client rather than by terminal.
     */
    @Test
    fun `a change goes out under the floor`() {
        assertTrue(claim.due(listOf("a"), 0))
        assertTrue(claim.due(listOf("b"), 1))
        assertTrue(claim.due(emptyList(), 2))
    }

    /**
     * "I am looking at nothing" is a real claim and the reason this is not
     * skipped when there is nothing to name — going to the Changes tab, or into
     * a pocket, has to be said out loud.
     */
    @Test
    fun `releasing everything is a claim, not a skip`() {
        assertTrue(claim.due(listOf("a"), 0))
        assertTrue(claim.due(emptyList(), 100))
        assertFalse(claim.due(emptyList(), 200))
    }

    /**
     * A reconnect is a runner that has been told nothing. Without the reset a
     * phone that dropped and came back on one pane would match its own last
     * claim, skip the send, and go on being notified about the pane in front of
     * it.
     */
    @Test
    fun `a reconnect re-announces the same pane`() {
        assertTrue(claim.due(listOf("a"), 0))
        assertFalse(claim.due(listOf("a"), 500))
        claim.reset()
        assertTrue(claim.due(listOf("a"), 501))
    }

    /**
     * Well under the ten seconds the runner believes a claim for
     * (`WATCHED_TTL_MS`), which leaves room for two lost round trips before a
     * pane somebody is plainly looking at starts buzzing them again.
     */
    @Test
    fun `the floor leaves room for lost round trips`() {
        assertTrue(WatchingClaim.FLOOR_MS * 3 < 10_000)
    }
}
