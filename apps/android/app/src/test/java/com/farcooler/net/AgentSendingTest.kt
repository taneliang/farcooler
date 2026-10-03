package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.core.DisconnectedException
import com.farcooler.model.PermissionAnswering
import com.farcooler.model.RunnerRefusal
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.yield
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A permission answer or a prompt that didn't go is said, and can be tried
 * again. Android dropped both through `attempt`, and took the permission card
 * down on the tap, so a refused answer left the agent blocked on an ask the
 * phone could no longer show. Mirrors AgentKit's `PermissionAnsweringTests`.
 */
class AgentSendingTest {

    private val answerArgs = buildJsonObject { put("requestId", JsonPrimitive("hook-ask-1")) }
    private val promptArgs = buildJsonObject { put("text", JsonPrimitive("hello")) }

    /** Taken: the card comes down. */
    @Test
    fun aSentAnswerTakesTheCardDown() = runTest {
        val sending = AgentSending { _, _ -> }
        assertTrue(sending.answer("hook-ask-1", answerArgs))
        assertNull(sending.answering.value.sending)
        assertNull(sending.answering.value.sentence("hook-ask-1"))
    }

    /**
     * The link dropped: the card stays up, because the ask may still be held
     * on the runner, and it says why.
     *
     * Mutation: the error dropped and the card taken down anyway (the old
     * `answer`). Red: `answer` returns true and there is no sentence.
     */
    @Test
    fun aFailedAnswerKeepsTheCardAndSaysSo() = runTest {
        val sending = AgentSending { _, _ -> throw DisconnectedException("link gone") }
        assertFalse(sending.answer("hook-ask-1", answerArgs))
        assertEquals(
            "Your answer may not have reached the runner. Try again.",
            sending.answering.value.sentence("hook-ask-1"),
        )
        assertNull(sending.answering.value.sentence("hook-ask-2"))
        // And the buttons are back on, to try again.
        assertFalse(sending.answering.value.isSending("hook-ask-1"))
    }

    /** Answered somewhere else, or ended: the card comes down without a word. */
    @Test
    fun aConflictTakesTheCardDownSilently() = runTest {
        val sending = AgentSending { _, _ ->
            throw CoreException("Someone already answered this.", word = "resource-conflict")
        }
        assertTrue(sending.answer("hook-ask-1", answerArgs))
        assertNull(sending.answering.value.sentence("hook-ask-1"))
    }

    /** A known refusal is said in Far Cooler's words, never the runner's. */
    @Test
    fun aKnownRefusalIsSaidInFarCoolersWords() = runTest {
        val sending = AgentSending { _, _ ->
            throw CoreException("runner prose", word = "invalid-argument")
        }
        assertFalse(sending.answer("hook-ask-1", answerArgs))
        val sentence = sending.answering.value.sentence("hook-ask-1").orEmpty()
        assertTrue(sentence, sentence.startsWith("The runner didn’t take your answer. "))
        assertTrue(sentence.contains(RunnerRefusal.INVALID_ARGUMENT.sentence))
        assertFalse(sentence.contains("runner prose"))
    }

    /**
     * One answer at a time: a second tap while the first is out sends
     * nothing, and the buttons are off while it is.
     */
    @Test
    fun aSecondTapWhileAnAnswerIsOutSendsNothing() = runTest {
        val gate = CompletableDeferred<Unit>()
        var calls = 0
        val sending = AgentSending { _, _ -> calls++; gate.await() }
        val first = async { sending.answer("hook-ask-1", answerArgs) }
        yield()
        assertTrue(sending.answering.value.isSending("hook-ask-1"))
        assertFalse(sending.answer("hook-ask-1", answerArgs))
        gate.complete(Unit)
        assertTrue(first.await())
        assertEquals(1, calls)
    }

    /** Trying again clears the old sentence, and a success takes the card down. */
    @Test
    fun tryingAgainAfterAFailureWorks() = runTest {
        var fail = true
        val sending = AgentSending { _, _ -> if (fail) throw DisconnectedException("gone") }
        assertFalse(sending.answer("hook-ask-1", answerArgs))
        fail = false
        assertTrue(sending.answer("hook-ask-1", answerArgs))
        assertNull(sending.answering.value.sentence("hook-ask-1"))
    }

    /**
     * A prompt that didn't go is said, with the words to send again.
     *
     * Mutation: the error dropped (the old `send`). Red: no failure.
     */
    @Test
    fun aFailedPromptIsSaidAndKept() = runTest {
        val sending = AgentSending { _, _ -> throw DisconnectedException("link gone") }
        assertFalse(sending.prompt(promptArgs))
        val failure = sending.sendFailure.value
        assertEquals("Couldn’t reach this runner. Your message wasn’t sent.", failure?.message)
        assertEquals(promptArgs, failure?.args)
    }

    /** Try Again sends the same prompt, and its success clears the failure. */
    @Test
    fun tryAgainSendsTheSamePrompt() = runTest {
        val sent = mutableListOf<Pair<String, JsonObject>>()
        var fail = true
        val sending = AgentSending { method, args ->
            if (fail) throw DisconnectedException("gone")
            sent += method to args
        }
        sending.prompt(promptArgs)
        fail = false
        assertTrue(sending.retry())
        assertEquals(listOf("terminal.agent_prompt" to promptArgs), sent)
        assertNull(sending.sendFailure.value)
    }

    @Test
    fun anOversizePromptSaysSo() {
        assertEquals(
            "That was too large to send. Try a smaller image.",
            AgentSending.messageFor(CoreException("payload too large: 1048577 bytes")),
        )
    }

    @Test
    fun aBlankWordIsAFailureNotAConflict() {
        assertTrue(PermissionAnswering.outcome("") is PermissionAnswering.Outcome.Failed)
        assertEquals(
            PermissionAnswering.Outcome.AnsweredElsewhere,
            PermissionAnswering.outcome("resource-conflict"),
        )
    }
}
