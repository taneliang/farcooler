package com.farcooler.model

import com.farcooler.net.Connection
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A runner that refuses this phone's key after letting it in has taken its access away; a refusal or two may pass
 * (ov-818). AgentKit's `KeyRefusalsTests` holds the Apple apps to the same rule.
 */
class KeyRefusalsTest {
    /** The backoff's spacing: about two, four and eight seconds apart. */
    private val t0 = 1_800_000_000_000L
    private val dials = listOf(0L, 2_000, 6_000, 14_000, 44_000).map { t0 + it }

    @Test
    fun `a runner that never let this phone in is a stranger's refusal, never retried`() {
        val refusals = KeyRefusals(admitted = false)
        for (at in dials) {
            val kind = Connection.Failure.KEY_REJECTED.reading(refusals, at)
            assertEquals(Connection.Failure.KEY_REJECTED, kind)
            assertEquals(Connection.Retry.NEVER, kind.retry)
        }
    }

    @Test
    fun `refusals held over ten seconds from a runner that let this phone in are access removed, checked slowly`() {
        val refusals = KeyRefusals(admitted = true)
        for (at in dials.take(3)) {
            val kind = Connection.Failure.KEY_REJECTED.reading(refusals, at)
            assertEquals(Connection.Failure.REFUSED_THIS_TIME, kind)
            assertEquals(Connection.Retry.ON_THE_BACKOFF, kind.retry)
        }
        val kind = Connection.Failure.KEY_REJECTED.reading(refusals, dials[3])
        assertEquals(Connection.Failure.ACCESS_REMOVED, kind)
        assertEquals(Connection.Retry.AFTER_A_WHILE, kind.retry)
        assertFalse(kind.worthRetryingAsAlternative)
    }

    /** The transient case: refused, then let in, then refused again. Never access removed. */
    @Test
    fun `a refusal that passes is retried on the backoff and a dial that gets in starts again`() {
        val refusals = KeyRefusals(admitted = true)
        for (at in dials.take(3)) Connection.Failure.KEY_REJECTED.reading(refusals, at)
        refusals.letIn()
        for (at in dials.drop(3)) {
            assertEquals(Connection.Failure.REFUSED_THIS_TIME, Connection.Failure.KEY_REJECTED.reading(refusals, at))
        }
    }

    @Test
    fun `many refusals within a second are still passing`() {
        val refusals = KeyRefusals(admitted = true)
        repeat(8) { assertEquals(KeyRefusals.Verdict.PASSING, refusals.refused(t0 + it * 100L)) }
    }

    @Test
    fun `a failure that is not a refusal leaves the count alone`() {
        val refusals = KeyRefusals(admitted = true)
        Connection.Failure.KEY_REJECTED.reading(refusals, t0)
        assertEquals(Connection.Failure.UNREACHABLE, Connection.Failure.UNREACHABLE.reading(refusals, t0 + 1))
        assertEquals(Connection.Failure.TUNNEL_NO_ANSWER, Connection.Failure.TUNNEL_NO_ANSWER.reading(refusals, t0 + 2))
        assertEquals(1, refusals.refusals)
    }

    @Test
    fun `being let in is remembered per runner`() {
        val runners = InMemoryAdmittedRunners()
        assertFalse(runners.admitted("a"))
        runners.remember("a")
        assertTrue(runners.admitted("a"))
        assertFalse(runners.admitted("b"))
    }

    @Test
    fun `the words match AgentKit's in Material's sentence case`() {
        assertEquals("You no longer have access to this runner", RunnerTeam.NO_ACCESS_TITLE)
        assertTrue(RunnerTeam.NO_ACCESS_BODY.contains("ask the runner’s owner to add this device again"))
    }
}
