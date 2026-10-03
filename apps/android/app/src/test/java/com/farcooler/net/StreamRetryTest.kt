package com.farcooler.net

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

/**
 * A late re-attach after a stop opens no SSH channel. Every one it did open was
 * one of the ten a default sshd gives the whole phone, held for a pane nobody
 * was looking at. See [StreamRetry].
 */
@OptIn(ExperimentalCoroutinesApi::class)
class StreamRetryTest {
    private val scope = TestScope()
    private var started = true
    private var opened = 0
    private val retry = StreamRetry(scope, wanted = { started })

    @Test
    fun `a retry runs once its wait is over`() {
        retry.schedule(500) { opened += 1 }
        scope.advanceTimeBy(501)
        assertEquals(1, opened)
        assertFalse(retry.pending)
    }

    /** What `stop` does: tear down, which cancels the wait. */
    @Test
    fun `a stop during the wait cancels the retry`() {
        retry.schedule(500) { opened += 1 }
        scope.advanceTimeBy(100)
        started = false
        retry.cancel()
        scope.advanceTimeBy(1_000)
        assertEquals(0, opened)
    }

    /** The cancel and the wake-up crossing: the wait is over, but nobody wants the pane. */
    @Test
    fun `a retry that wakes after a stop opens nothing`() {
        retry.schedule(500) { opened += 1 }
        started = false
        scope.advanceTimeBy(1_000)
        assertEquals(0, opened)
    }

    @Test
    fun `scheduling again replaces the retry waiting`() {
        retry.schedule(500) { opened += 1 }
        retry.schedule(500) { opened += 10 }
        scope.advanceTimeBy(1_000)
        scope.runCurrent()
        assertEquals(10, opened)
    }

    /**
     * The cliff: a pane used to stop asking after three dead attaches and poll
     * for good, stranded on the slower path until someone switched tabs. The
     * wait widens instead, and never runs out.
     */
    @Test
    fun `retries widen to the ceiling and never stop`() {
        val waits = List(20) { retry.nextDelayMs() }
        assertEquals(listOf(500L, 800L, 1_280L, 2_048L), waits.take(4))
        assertEquals(StreamRetry.CEILING_MS, waits.last())
        assertEquals(waits, waits.sorted())
    }

    /** A stream that delivered is evidence about this link that the failures before it are not. */
    @Test
    fun `a first byte starts the wait over`() {
        repeat(5) { retry.nextDelayMs() }
        retry.resetBackoff()
        assertEquals(StreamRetry.FLOOR_MS, retry.nextDelayMs())
    }

    /** The channel goes back before the wait, so retries cannot pile up channels on the runner. */
    @Test
    fun `the channel is released before the wait`() {
        val log = mutableListOf<String>()
        retry.schedule(500, before = { log += "stop" }) { log += "open" }
        scope.runCurrent()
        assertEquals(listOf("stop"), log)
        scope.advanceTimeBy(501)
        assertEquals(listOf("stop", "open"), log)
    }
}
