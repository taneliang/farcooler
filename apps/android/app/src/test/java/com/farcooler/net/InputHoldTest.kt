package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.core.DisconnectedException
import com.farcooler.model.RunnerRefusal
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** A runner's `terminal.write`, scripted: each call takes the next outcome and records its bytes. */
private class FakeRunner(vararg outcomes: WriteOutcome) {
    val outcomes = outcomes.toMutableList()
    val sent = mutableListOf<List<Byte>>()
    var onSend: (suspend () -> Unit)? = null

    suspend fun write(bytes: ByteArray): WriteOutcome {
        sent += bytes.toList()
        val hook = onSend
        onSend = null
        hook?.invoke()
        return if (outcomes.isEmpty()) WriteOutcome.Written else outcomes.removeAt(0)
    }
}

private fun b(s: String) = s.toByteArray()
private fun l(s: String) = s.toByteArray().toList()
private val never = WriteOutcome.NeverSent(InputHold.Reason.DISCONNECTED)

class InputHoldTest {
    @Test
    fun aTimedOutWriteIsNeverSentAgain() = runBlocking {
        val hold = InputHold()
        val runner = FakeRunner(WriteOutcome.MaybeSent)
        hold.type(b("rm x\r"), runner::write)
        assertEquals("a timed-out key may have arrived", 0, hold.heldBytes.size)
        assertEquals("Some typing may not have reached the runner.", hold.line.value?.sentence)
        assertFalse(hold.line.value!!.holding)
        hold.retry(runner::write)
        hold.type(b("ls"), runner::write)
        assertEquals(listOf(l("rm x\r"), l("ls")), runner.sent)
    }

    @Test
    fun aWriteThatNeverLeftIsHeldWithItsReason() = runBlocking {
        val hold = InputHold()
        val runner = FakeRunner(never)
        hold.type(b("ls\r"), runner::write)
        assertArrayEquals(b("ls\r"), hold.heldBytes)
        assertTrue(hold.line.value!!.holding)
        assertTrue(hold.line.value!!.sentence.startsWith("Far Cooler lost the connection"))
    }

    @Test
    fun aNewKeyJoinsTheHoldAndDoesNotFlushIt() = runBlocking {
        val hold = InputHold()
        val runner = FakeRunner(never)
        hold.type(b("ls"), runner::write)
        hold.type(b("\r"), runner::write)
        assertEquals("the new key must not reach the runner", listOf(l("ls")), runner.sent)
        assertArrayEquals(b("ls\r"), hold.heldBytes)
    }

    @Test
    fun tryAgainSendsHeldThenNewInOrderOnce() = runBlocking {
        val hold = InputHold()
        val runner = FakeRunner(never, WriteOutcome.Written)
        hold.type(b("ls"), runner::write)
        hold.type(b("\r"), runner::write)
        hold.retry(runner::write)
        assertEquals(listOf(l("ls"), l("ls\r")), runner.sent)
        assertNull(hold.line.value)
        hold.retry(runner::write)
        assertEquals("a second Try again has nothing to send", 2, runner.sent.size)
    }

    @Test
    fun aFailedTryAgainKeepsTheHoldUntilAnAnswerConfirms() = runBlocking {
        val hold = InputHold()
        val runner = FakeRunner(never, WriteOutcome.NeverSent(InputHold.Reason.REFUSED), WriteOutcome.Written)
        hold.type(b("ab"), runner::write)
        hold.retry(runner::write)
        assertArrayEquals("cleared before the answer", b("ab"), hold.heldBytes)
        hold.retry(runner::write)
        assertEquals(0, hold.heldBytes.size)
    }

    @Test
    fun aKeyTypedWhileAnotherIsInFlightCannotOvertakeIt() = runBlocking {
        val hold = InputHold()
        val runner = FakeRunner(never)
        runner.onSend = { hold.type(b("b"), runner::write) }
        hold.type(b("a"), runner::write)
        assertEquals("b was sent past a failed a", listOf(l("a")), runner.sent)
        assertArrayEquals(b("ab"), hold.heldBytes)
    }

    @Test
    fun keysTypedBehindAWriteGoOutTogetherAfterItSucceeds() = runBlocking {
        val hold = InputHold()
        val runner = FakeRunner(WriteOutcome.Written, WriteOutcome.Written)
        runner.onSend = {
            hold.type(b("b"), runner::write)
            hold.type(b("c"), runner::write)
        }
        hold.type(b("a"), runner::write)
        assertEquals(listOf(l("a"), l("bc")), runner.sent)
    }

    @Test
    fun discardDropsTheHold() = runBlocking {
        val hold = InputHold()
        val runner = FakeRunner(never)
        hold.type(b("ls"), runner::write)
        hold.discard()
        assertEquals(0, hold.heldBytes.size)
        assertNull(hold.line.value)
        hold.retry(runner::write)
        assertEquals(listOf(l("ls")), runner.sent)
    }

    @Test
    fun heldInputIsCappedKeepingTheEarliestAndSaysSo() = runBlocking {
        val hold = InputHold()
        val runner = FakeRunner(never)
        hold.type(b("x"), runner::write)
        hold.type(ByteArray(10_000) { 'y'.code.toByte() }, runner::write)
        assertEquals(InputHold.CAP, hold.heldBytes.size)
        assertEquals("the earliest bytes are the ones kept", 'x'.code.toByte(), hold.heldBytes[0])
        assertTrue(hold.line.value!!.sentence.endsWith("Only the first 4 KB is kept."))
    }

    @Test
    fun keysTypedDuringATryAgainFlightShareTheCap() = runBlocking {
        val hold = InputHold()
        val runner = FakeRunner(never, WriteOutcome.Written, WriteOutcome.Written)
        hold.type(b("a"), runner::write)
        runner.onSend = { hold.type(ByteArray(10_000) { 'y'.code.toByte() }, runner::write) }
        hold.retry(runner::write)
        assertEquals("the flight's keys are capped", InputHold.CAP, runner.sent.last()!!.size)
        assertEquals("dropped keys are admitted, not silent", "Some typing may not have reached the runner.", hold.line.value?.sentence)
    }

    @Test
    fun aClosedPaneDropsTheHoldAndAnythingInFlight() = runBlocking {
        val hold = InputHold()
        val runner = FakeRunner(never, WriteOutcome.Written)
        hold.type(b("ls"), runner::write)
        hold.paneClosed()
        assertNull(hold.line.value)
        hold.retry(runner::write)
        assertEquals("a closed pane is never written to", listOf(l("ls")), runner.sent)

        // A write that fails after the pane closed doesn't resurrect a hold.
        val late = InputHold()
        val slow = FakeRunner(never)
        slow.onSend = { late.paneClosed() }
        late.type(b("x"), slow::write)
        assertEquals(0, late.heldBytes.size)
        assertNull(late.line.value)
    }

    @Test
    fun onlyAProvablyUnsentFailureIsResendable() {
        assertEquals(never, WriteOutcome.failed(null, disconnected = true, notSent = true))
        assertEquals(WriteOutcome.MaybeSent, WriteOutcome.failed(null, disconnected = true, notSent = false))
        assertEquals(
            WriteOutcome.MaybeSent,
            WriteOutcome.failed(RunnerRefusal.TIMED_OUT_WORD, disconnected = false, notSent = false),
        )
        assertEquals(
            WriteOutcome.NeverSent(InputHold.Reason.REFUSED),
            WriteOutcome.failed("not-found", disconnected = false, notSent = false),
        )
        assertEquals(WriteOutcome.MaybeSent, WriteOutcome.failed(null, disconnected = false, notSent = false))
        // And from what a call throws.
        assertEquals(never, WriteOutcome.of(DisconnectedException("not connected", notSent = true)))
        assertEquals(WriteOutcome.MaybeSent, WriteOutcome.of(DisconnectedException("gone")))
        assertEquals(WriteOutcome.MaybeSent, WriteOutcome.of(CoreException("slow", RunnerRefusal.TIMED_OUT_WORD)))
    }

    @Test
    fun aTimedOutLineIsReadAsATimeoutAndSaysSoPlainly() {
        val json = kotlinx.serialization.json.Json
        fun line(raw: String) = json.parseToJsonElement(raw) as kotlinx.serialization.json.JsonObject
        val late = line("""{"ticket":4,"ok":false,"disconnected":false,"timed_out":true,"error":"x"}""")
        val word = RunnerRefusal.wordInAnswerLine(late)
        assertEquals(RunnerRefusal.TIMED_OUT_WORD, word)
        val trouble = com.farcooler.model.troubleFor(word, "raw", "Generic.")
        assertEquals(RunnerRefusal.TIMED_OUT_SENTENCE, trouble.sentence)
        assertNull("a timeout has no raw words to show beneath it", trouble.transcript)
        assertEquals("not-found", RunnerRefusal.wordInAnswerLine(line("""{"code":"not-found","timed_out":true}""")))
        assertNull(RunnerRefusal.wordInAnswerLine(line("""{"ok":false,"timed_out":false}""")))
    }
}
