package com.farcooler.net

import com.farcooler.core.TerminalTransport
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * When a terminal pane opens and closes stream channels, driven through the
 * real [TerminalSession] against a fake runner, on virtual time.
 *
 * The emulator is the real [com.farcooler.core.VtCore] with no native library
 * behind it, so it draws nothing; what is checked is the channels: how many
 * opened, when, and whether polling is painting. A channel is one of the ten a
 * default sshd gives the whole phone.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class TerminalStreamRecoveryTest {
    private class FakeRunner : TerminalTransport {
        var starts = 0
        var stops = 0
        var streamOpens = true
        /** Holds the next call until completed, to land a stop mid-`prime`. */
        var holdNext: CompletableDeferred<Unit>? = null
        val asked = mutableListOf<JsonObject>()
        private var onEnd: ((String?) -> Unit)? = null
        private var onChunk: ((ByteArray) -> Unit)? = null

        /** Poll-loop screen asks: no history, no known revision. */
        val polls get() = asked.count { "historyLines" !in it && "knownRevision" !in it && "terminal" in it && it.size == 1 }

        override suspend fun call(method: String, args: JsonObject): JsonObject {
            if (method == "terminal.screen") asked += args
            holdNext?.let { holdNext = null; it.await() }
            return buildJsonObject { put("contents", ""); put("columns", 80); put("rows", 24) }
        }

        override suspend fun startStream(terminal: String, onChunk: (ByteArray) -> Unit, onEnd: (String?) -> Unit): Boolean {
            starts += 1
            this.onChunk = onChunk
            this.onEnd = onEnd
            return streamOpens
        }

        override suspend fun stopStream(terminal: String) {
            stops += 1
        }

        fun drop() = onEnd!!("channel closed")
        fun speak() = onChunk!!(byteArrayOf(0x41))
    }

    private val scope = TestScope()
    private val runner = FakeRunner()
    private val session = TerminalSession("t", runner, StandardTestDispatcher(scope.testScheduler))

    @After
    fun tearDown() {
        session.dispose()
        scope.runCurrent()
    }

    private fun open() {
        session.configure(80, 24)
        scope.runCurrent()
        assertEquals(1, runner.starts)
    }

    /** The orphan channel: a stop during the retry wait, and the wait ending anyway. */
    @Test
    fun `a stop during the retry wait opens no channel`() {
        open()
        runner.drop()
        scope.advanceTimeBy(100)
        session.stop()
        scope.advanceTimeBy(60_000)
        assertEquals(1, runner.starts)
    }

    /** The same, landing while `open` is still waiting on the screen read. */
    @Test
    fun `a stop while the screen is being read opens no channel`() {
        runner.holdNext = CompletableDeferred()
        val held = runner.holdNext!!
        session.configure(80, 24)
        scope.runCurrent()
        session.stop()
        scope.runCurrent()
        held.complete(Unit)
        scope.advanceTimeBy(60_000)
        assertEquals(0, runner.starts)
    }

    /** The cliff: three drops used to send a pane to polling for good. */
    @Test
    fun `a pane keeps asking for the stream however often it drops`() {
        open()
        // Five drops in a row with no byte between them, each retried on the
        // widening wait (0.5 s to 3.3 s), all well inside the 12 s deadline.
        repeat(5) {
            runner.drop()
            scope.advanceTimeBy(4_000)
        }
        assertEquals(6, runner.starts)
    }

    /** A stream that opens and never speaks: painted by polling at once, retried at the deadline. */
    @Test
    fun `a silent stream is painted by polling, then retried`() {
        open()
        scope.advanceTimeBy(500)
        assertEquals(0, runner.polls)
        scope.advanceTimeBy(300)
        assertTrue("polling paints after the grace", runner.polls > 0)
        val stopsBefore = runner.stops
        scope.advanceTimeBy(12_000)
        assertTrue("the wedged channel is handed back", runner.stops > stopsBefore)
        scope.advanceTimeBy(1_000)
        assertEquals(2, runner.starts)
    }

    /** The handover: the first byte takes the screen back, and polling stops. */
    @Test
    fun `the first byte stops polling`() {
        open()
        scope.advanceTimeBy(1_000)
        assertTrue(runner.polls > 0)
        runner.speak()
        scope.runCurrent()
        val pollsAtHandover = runner.polls
        scope.advanceTimeBy(30_000)
        assertEquals(pollsAtHandover, runner.polls)
        assertEquals(TerminalSession.Phase.Live, session.phase.value)
        assertEquals(1, runner.starts)
    }
}
