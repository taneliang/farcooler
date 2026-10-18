package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.core.DisconnectedException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.currentTime
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Coming back to the app (ov-613), on the test dispatcher's virtual clock:
 * how soon a dead link is dialed again, and what a healthy one costs.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class LinkRevivalTest {

    @Test
    fun a_probe_timing_out_reconnects_at_the_probe_deadline() = runTest {
        val dials = mutableListOf<Pair<LinkRevival.Outcome, Long>>()
        val back = async {
            LinkRevival.onReturn(
                LinkRevival.Standing.Connected, backgroundScope,
                probe = { awaitCancellation() },
                dialNow = { dials += it to currentTime },
                refresh = { awaitCancellation() },
            )
        }
        advanceTimeBy(2_499)
        runCurrent()
        assertTrue("redialed before the deadline: $dials", dials.isEmpty())
        assertEquals(LinkRevival.Outcome.Redialed(LinkRevival.Probe.SILENT), back.await())
        assertEquals(listOf(LinkRevival.Outcome.Redialed(LinkRevival.Probe.SILENT) to 2_500L), dials)
    }

    @Test
    fun a_found_disconnect_dials_at_once() = runTest {
        val dials = mutableListOf<Long>()
        val outcome = LinkRevival.onReturn(
            LinkRevival.Standing.Connected, backgroundScope,
            probe = { LinkRevival.verdict { throw DisconnectedException("gone") } },
            dialNow = { dials += currentTime },
            refresh = { awaitCancellation() },
        )
        assertEquals(LinkRevival.Outcome.Redialed(LinkRevival.Probe.DROPPED), outcome)
        assertEquals(listOf(0L), dials)
    }

    @Test
    fun a_healthy_link_costs_one_probe() = runTest {
        val asked = mutableListOf<String>()
        val probeAnswer = CompletableDeferred<Unit>()
        var dials = 0
        val back = async {
            LinkRevival.onReturn(
                LinkRevival.Standing.Connected, backgroundScope,
                probe = { LinkRevival.verdict { asked += "probe"; probeAnswer.await() } },
                dialNow = { dials++ },
                refresh = { asked += "fleet"; awaitCancellation() },
            )
        }
        runCurrent()
        // Both out before either is answered: the fleet doesn't wait on the probe.
        assertEquals(setOf("probe", "fleet"), asked.toSet())
        probeAnswer.complete(Unit)
        assertEquals(LinkRevival.Outcome.Alive, back.await())
        assertEquals(listOf("probe", "fleet").sorted(), asked.sorted())
        assertEquals(0, dials)
        // A runner refusing the probe still answered it.
        assertEquals(LinkRevival.Probe.ANSWERED, LinkRevival.verdict { throw CoreException("no") })
    }

    @Test
    fun a_stuck_dial_is_restarted_and_a_fresh_one_left() = runTest {
        val cases = listOf(
            LinkRevival.Standing.Dialing(40_000, beganBeforeLeaving = true) to LinkRevival.Outcome.RestartedDial,
            // A slow first connect begun since the app came back is never churned.
            LinkRevival.Standing.Dialing(6_000, beganBeforeLeaving = false) to LinkRevival.Outcome.Left,
            LinkRevival.Standing.Dialing(300, beganBeforeLeaving = true) to LinkRevival.Outcome.Left,
            LinkRevival.Standing.Retrying to LinkRevival.Outcome.Dialed,
            LinkRevival.Standing.WaitingOnPerson to LinkRevival.Outcome.Left,
        )
        for ((standing, expected) in cases) {
            val dials = mutableListOf<LinkRevival.Outcome>()
            val outcome = LinkRevival.onReturn(
                standing, backgroundScope, probe = { error("no probe for $standing") },
                dialNow = { dials += it }, refresh = { error("no read for $standing") },
            )
            assertEquals("$standing", expected, outcome)
            assertEquals("$standing", if (expected == LinkRevival.Outcome.Left) emptyList() else listOf(expected), dials)
        }
    }

    @Test
    fun the_reads_start_concurrently() = runTest {
        val started = mutableListOf<String>()
        val release = CompletableDeferred<Unit>()
        val reads = listOf("fleet", "repositories", "themes")
        val bringUp = async {
            LinkRevival.together(*reads.map { name -> suspend { started += name; release.await() } }.toTypedArray())
        }
        runCurrent()
        assertEquals(reads.toSet(), started.toSet())
        release.complete(Unit)
        bringUp.await()
    }

    /**
     * A drop the fleet read finds while coming back dials at once; any other
     * waits out the backoff's first rung. This is the decision [Connection]'s
     * `linkDropped` makes, where the 1.6 to 2.4 s wait used to be.
     */
    @Test
    fun a_drop_found_on_return_skips_the_backoff() {
        assertEquals(LinkRevival.AfterDrop.DialNow, LinkRevival.afterDrop(returning = true))
        assertEquals(LinkRevival.AfterDrop.Backoff(1), LinkRevival.afterDrop(returning = false))
    }

    /** Which dials a return may restart: only one begun before the app went away. */
    @Test
    fun a_dial_is_dated_against_the_last_time_the_app_went_away() {
        var clock = 1_000L
        val lines = mutableListOf<String>()
        val steps = LinkSteps(now = { clock }, sink = { lines += it })
        val phase = Connection.Phase.Reconnecting(1)
        // A backoff waiting: no dial out, so the return dials.
        assertEquals(LinkRevival.Standing.Retrying, steps.standing(phase))
        val began = steps.dialBegan()
        clock += 500
        steps.left()
        clock += 9_500
        assertEquals(LinkRevival.Standing.Dialing(10_000, beganBeforeLeaving = true), steps.standing(phase))
        steps.dialEnded(began, connected = false)
        val again = steps.dialBegan()
        clock += 6_000
        assertEquals(LinkRevival.Standing.Dialing(6_000, beganBeforeLeaving = false), steps.standing(phase))
        steps.dialEnded(again, connected = true)
        assertTrue("$lines", lines.all { it.startsWith("[") && !it.contains("@") })
    }
}
