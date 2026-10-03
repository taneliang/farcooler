package com.farcooler.net

import com.farcooler.core.ClientCore
import com.farcooler.core.DisconnectedException
import com.farcooler.model.AgentEvent
import com.farcooler.model.PermissionOption
import com.farcooler.model.Sequenced
import com.farcooler.model.TranscriptRow
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * [AgentStream.answer] and [AgentStream.send] as the agent screen calls them,
 * with only the runner faked. [AgentSendingTest] proves the rules; this proves
 * the pane uses them.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class AgentStreamSendTest {

    private class Runner {
        val calls = mutableListOf<Pair<String, JsonObject>>()
        var fail = false
        var gate: CompletableDeferred<Unit>? = null

        suspend fun call(method: String, args: JsonObject): JsonObject {
            calls += method to args
            gate?.await()
            if (fail) throw DisconnectedException("link gone")
            return JsonObject(emptyMap())
        }

        fun sent(method: String) = calls.filter { it.first == method }
    }

    private fun TestScope.stream(runner: Runner) =
        AgentStream("t1", ClientCore(), this, call = runner::call)

    private fun AgentStream.ask(id: String = "r1") {
        transcript.apply(
            listOf(
                Sequenced(
                    0,
                    AgentEvent.Permission(id, "tool-1", listOf(PermissionOption("allow", "Allow", "allow_once"))),
                )
            )
        )
    }

    private fun AgentStream.userRows() = transcript.rows.count {
        (it.kind as? TranscriptRow.Kind.Message)?.role == com.farcooler.model.Role.USER
    }

    /**
     * Mutation: `answer` back to clearing on the tap and `fireAndForget`.
     * Red: the card is gone and nothing is said.
     */
    @Test
    fun aRefusedAnswerLeavesTheCardUpAndSaysSo() = runTest {
        val runner = Runner().apply { fail = true }
        val stream = stream(runner)
        stream.ask()
        stream.answer("r1", "allow")
        advanceUntilIdle()
        assertEquals("r1", stream.transcript.pendingPermission?.id)
        assertEquals(
            "Your answer may not have reached the runner. Try again.",
            stream.answering.value.sentence("r1"),
        )
        assertEquals(1, runner.sent("terminal.agent_answer").size)
    }

    /** Taken by the runner: the card comes down, and the pane redraws. */
    @Test
    fun aTakenAnswerTakesTheCardDown() = runTest {
        val runner = Runner()
        val stream = stream(runner)
        stream.ask()
        val before = stream.revision.value
        stream.answer("r1", "allow")
        advanceUntilIdle()
        assertNull(stream.transcript.pendingPermission)
        assertEquals(stream.transcript.revision, stream.revision.value)
        assert(stream.revision.value != before)
        val args = runner.sent("terminal.agent_answer").single().second
        assertEquals("allow", args["optionId"]?.jsonPrimitive?.content)
    }

    /**
     * Mutation: `send` back to `attempt`. Red: no failure to show.
     */
    @Test
    fun aFailedPromptIsSaidAndTryAgainSendsItOnce() = runTest {
        val runner = Runner().apply { fail = true }
        val stream = stream(runner)
        stream.send("hello")
        advanceUntilIdle()
        assertNotNull(stream.sendFailure.value)
        assertEquals(1, stream.userRows())

        runner.fail = false
        stream.retrySend()
        advanceUntilIdle()
        assertNull(stream.sendFailure.value)
        assertEquals(2, runner.sent("terminal.agent_prompt").size)
        // The words were drawn once, on the first try.
        assertEquals(1, stream.userRows())
    }

    /**
     * Two taps on Try Again while the first is out send once.
     *
     * Mutation: `retry` leaving the failure up while it sends. Red: three
     * prompts.
     */
    @Test
    fun aDoubleTapOnTryAgainSendsOnce() = runTest {
        val runner = Runner().apply { fail = true }
        val stream = stream(runner)
        stream.send("hello")
        advanceUntilIdle()
        runner.fail = false
        runner.gate = CompletableDeferred()
        stream.retrySend()
        stream.retrySend()
        advanceUntilIdle()
        runner.gate?.complete(Unit)
        advanceUntilIdle()
        assertEquals(2, runner.sent("terminal.agent_prompt").size)
        assertNull(stream.sendFailure.value)
    }

    /** A picture travels as standard Base64, as `android.util.Base64.NO_WRAP` sent it. */
    @Test
    fun anImageTravelsAsStandardBase64() = runTest {
        val runner = Runner()
        val stream = stream(runner)
        stream.send("look", listOf(AgentStream.Attachment("image/png", byteArrayOf(-5, -1, 0, 1, 2, 3))))
        advanceUntilIdle()
        val image = runner.sent("terminal.agent_prompt").single().second["images"]!!.jsonArray[0].jsonObject
        assertEquals("+/8AAQID", image["base64"]?.jsonPrimitive?.content)
        assertEquals("image/png", image["mime"]?.jsonPrimitive?.content)
    }
}
