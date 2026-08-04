package com.farcooler.net

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Cancellation must never be reported as a failure.
 *
 * This pins a defect that reached a real phone: every keystroke cancels the
 * terminal's in-flight poll on purpose — `write` calls `wake`, which restarts
 * the loop at the fast interval — and `catch (e: Exception)` caught the
 * cancellation and painted "Could not load: StandaloneCoroutine was cancelled"
 * over the screen.
 *
 * The trap is that [CancellationException] extends `IllegalStateException`, so
 * every ordinary defensive idiom swallows it and looks correct doing so.
 */
class CancellationTest {

    @Test
    fun cancellationIsNotAFailure() {
        val result = runCatching { attempt<Unit> { throw CancellationException("cancelled") } }
        assertTrue(
            "attempt must let cancellation through",
            result.exceptionOrNull() is CancellationException,
        )
    }

    @Test
    fun everyOtherFailureIsStillCaught() {
        val result = attempt<Unit> { throw IllegalStateException("the host said no") }
        assertTrue(result.isFailure)
        assertEquals("the host said no", result.exceptionOrNull()?.message)
    }

    @Test
    fun aSuccessComesBackUnchanged() {
        assertEquals(7, attempt { 7 }.getOrNull())
    }

    @Test
    fun rethrowIgnoresOrdinaryFailures() {
        // It must be safe to call unconditionally at the top of a catch block,
        // which is the only way it gets used consistently.
        IllegalStateException("nope").rethrowIfCancellation()
    }

    /**
     * The shape the bug actually had: a loop that catches broadly, cancelled
     * from outside while suspended.
     */
    @Test
    fun aCancelledPollDoesNotReportAnError() = runTest {
        var reported: String? = null
        var iterations = 0

        val job = launch {
            while (true) {
                try {
                    iterations += 1
                    delay(1_000)
                } catch (e: Exception) {
                    e.rethrowIfCancellation()
                    reported = e.message
                }
            }
        }

        // Let the loop reach its suspension point, then stop it the way `wake`
        // stops the poller.
        delay(10)
        job.cancel()
        job.join()

        assertTrue("the loop should have started", iterations > 0)
        assertEquals("cancellation must not be reported", null, reported)
        assertTrue("and the job must actually stop", job.isCancelled)
    }

    @Test
    fun withoutTheRethrowTheLoopWouldReportAndKeepRunning() = runTest {
        // The same loop with the bug in it, so the test says what the fix is
        // for rather than only that it works. Without the rethrow the
        // cancellation is caught, a message is produced for the screen, and the
        // loop carries on past the point it was told to stop.
        var reported: String? = null
        var iterationsAfterCancel = 0

        val job = launch {
            var cancelled = false
            while (iterationsAfterCancel < 3) {
                try {
                    if (cancelled) iterationsAfterCancel += 1
                    delay(1_000)
                } catch (e: Exception) {
                    // Deliberately no rethrow.
                    reported = e.message
                    cancelled = true
                }
            }
        }

        delay(10)
        job.cancel()
        job.join()

        assertFalse("the buggy shape produces a message for the screen", reported == null)
        assertTrue(
            "and keeps running after being told to stop",
            iterationsAfterCancel > 0,
        )
    }
}
