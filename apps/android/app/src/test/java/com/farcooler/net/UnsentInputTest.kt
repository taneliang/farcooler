package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.core.DisconnectedException
import com.farcooler.model.RunnerRefusal
import com.farcooler.model.troubleFor
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class UnsentInputTest {
    private fun line(raw: String) = Json.parseToJsonElement(raw).jsonObject

    @Test
    fun aTimedOutLineIsReadAsATimeoutAndSaysSoPlainly() {
        val late = line(
            """{"ticket":4,"ok":false,"disconnected":false,"timed_out":true,""" +
                """"error":"The runner took too long to answer. Try again."}"""
        )
        val word = RunnerRefusal.wordInAnswerLine(late)
        assertEquals(RunnerRefusal.TIMED_OUT_WORD, word)
        val trouble = troubleFor(word, "raw", "Generic.")
        assertEquals(RunnerRefusal.TIMED_OUT_SENTENCE, trouble.sentence)
        assertNull("a timeout has no raw words to show beneath it", trouble.transcript)
        // A refusal's own code still wins, and an ordinary line has no word.
        assertEquals(
            "not-found",
            RunnerRefusal.wordInAnswerLine(line("""{"code":"not-found","timed_out":true}""")),
        )
        assertNull(RunnerRefusal.wordInAnswerLine(line("""{"ok":false,"timed_out":false}""")))
    }

    @Test
    fun aFailedWriteIsHeldAndSentAgainFirstAndInOrder() {
        val why = UnsentInput.whyOf(CoreException("raw", RunnerRefusal.TIMED_OUT_WORD))
        assertEquals(UnsentInput.Why.TIMED_OUT, why)
        val held = UnsentInput(byteArrayOf(0x6c, 0x73), why)
        assertArrayEquals(
            byteArrayOf(0x6c, 0x73, 0x0d), UnsentInput.bytesToSend(held, byteArrayOf(0x0d)))
        assertArrayEquals(byteArrayOf(0x0d), UnsentInput.bytesToSend(null, byteArrayOf(0x0d)))
        val more = held.holding(byteArrayOf(0x0d), UnsentInput.Why.DISCONNECTED)
        assertArrayEquals(byteArrayOf(0x6c, 0x73, 0x0d), more.bytes)
        assertEquals(UnsentInput.Why.DISCONNECTED, more.why)
    }

    @Test
    fun theLineNamesTheReasonAndPromisesNothingLost() {
        assertEquals(UnsentInput.Why.DISCONNECTED, UnsentInput.whyOf(DisconnectedException("x")))
        assertEquals(UnsentInput.Why.OTHER, UnsentInput.whyOf(CoreException("x", "not-found")))
        assertEquals(UnsentInput.Why.OTHER, UnsentInput.whyOf(IllegalStateException()))
        for (why in UnsentInput.Why.entries) {
            val sentence = UnsentInput(byteArrayOf(1), why).sentence
            assertTrue(sentence, sentence.endsWith("Your typing is waiting."))
            assertFalse(sentence, sentence.contains("rror"))
        }
        assertTrue(UnsentInput(byteArrayOf(1), UnsentInput.Why.TIMED_OUT).sentence.contains("took too long"))
        assertEquals("Try again", UnsentInput.RETRY)
    }
}
