package com.farcooler.net

import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The retry schedule, which is shared by agreement rather than by code.
 *
 * The Mac's `DaemonClient.backoffSeconds` and the iPhone's
 * `Connection.backoff` compute the same thing in two other languages, and none
 * of the three can import the others. "How long until it comes back" still has
 * to have one answer, so this pins the numbers on the one platform with a unit
 * test to pin them on.
 */
class BackoffTest {

    @Test
    fun it_doubles_from_two_seconds_to_a_thirty_second_ceiling() {
        // Jitter is ±20%, so each rung is asserted as a band rather than a
        // number. The bands do not overlap, which is the property that makes
        // this a test of the schedule and not of the random number generator.
        for (attempt in 1..4) {
            val base = Math.pow(2.0, attempt.toDouble()) * 1000
            val wait = Connection.backoffMs(attempt)
            assertTrue(
                "attempt $attempt waited ${wait}ms, outside 80–120% of ${base}ms",
                wait >= base * 0.8 && wait <= base * 1.2,
            )
        }
    }

    @Test
    fun it_stops_growing_at_thirty_seconds() {
        // Without a ceiling this reaches hours, which on a phone means a
        // runner that came back an hour ago is still showing as down.
        for (attempt in 5..20) {
            val wait = Connection.backoffMs(attempt)
            assertTrue("attempt $attempt waited ${wait}ms", wait <= 30_000 * 1.2)
        }
    }

    @Test
    fun two_runners_recovering_together_do_not_retry_in_lockstep() {
        // The whole reason for the jitter: this app connects to every runner
        // at once, so one network event recovering them all is the ordinary
        // case here rather than the unlucky one, and an unjittered schedule
        // would aim the whole fleet's handshakes at the same instant.
        val waits = (1..50).map { Connection.backoffMs(6) }.toSet()
        assertTrue("every wait was identical: $waits", waits.size > 1)
    }
}
