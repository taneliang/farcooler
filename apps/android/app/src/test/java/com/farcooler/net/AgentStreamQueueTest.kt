package com.farcooler.net

import com.farcooler.core.ClientCore
import com.farcooler.core.CoreException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** ov-171: a queue call the runner refuses is said, never swallowed. */
@OptIn(ExperimentalCoroutinesApi::class)
class AgentStreamQueueTest {
    private class Runner {
        val calls = mutableListOf<String>()
        var refuse: CoreException? = null
        var gate: CompletableDeferred<Unit>? = null

        suspend fun call(method: String, args: JsonObject): JsonObject {
            calls += method
            gate?.await()
            refuse?.let { throw it }
            return JsonObject(emptyMap())
        }
    }

    private fun TestScope.stream(runner: Runner) =
        AgentStream("t1", ClientCore(), this, call = runner::call)

    /** Mutation: `queueCall` back to `fireAndForget`. Red: no failure. */
    @Test
    fun aRefusedCancelIsSaidWithTheRunnersReason() = runTest {
        val runner = Runner().apply { refuse = CoreException("raw", word = "agent-stopped") }
        val stream = stream(runner)
        stream.cancelQueued("q1")
        advanceUntilIdle()
        val said = stream.queueFailure.value?.message
        assertNotNull(said)
        assertTrue(said!!.startsWith("Couldn’t take that message back."))
        assertTrue(said.contains("The agent stopped."))
    }

    @Test
    fun eachCallSaysItsOwnStep() = runTest {
        val runner = Runner().apply { refuse = CoreException("raw", word = "agent-stopped") }
        val stream = stream(runner)
        stream.editQueued("q1", "x")
        advanceUntilIdle()
        assertTrue(stream.queueFailure.value!!.message.startsWith("Couldn’t save that edit."))
        stream.steerQueued("q1")
        advanceUntilIdle()
        assertTrue(
            stream.queueFailure.value!!.message.startsWith("Couldn’t send that into the running turn.")
        )
    }

    /**
     * Only `retryQueue` takes the failure down while the retried call is out;
     * the call's own success path hasn't run yet. Mutation: `retryQueue`
     * leaving the failure up. Red: still set while the call is in flight.
     */
    @Test
    fun tryAgainTakesTheFailureDownAndRunsTheSameCall() = runTest {
        val runner = Runner().apply { refuse = CoreException("raw", word = "agent-stopped") }
        val stream = stream(runner)
        stream.cancelQueued("q1")
        advanceUntilIdle()
        runner.refuse = null
        runner.gate = CompletableDeferred()
        stream.retryQueue()
        advanceUntilIdle()
        assertNull(stream.queueFailure.value)
        runner.gate?.complete(Unit)
        advanceUntilIdle()
        assertEquals(listOf("terminal.agent_cancel_queued", "terminal.agent_cancel_queued"), runner.calls)
    }

    /**
     * Only the call's success path can clear it here: no retry is involved.
     * Mutation: `queueCall` never clearing on success. Red: the old failure stays.
     */
    @Test
    fun aLaterCallThatWorksClearsTheOldFailure() = runTest {
        val runner = Runner().apply { refuse = CoreException("raw", word = "agent-stopped") }
        val stream = stream(runner)
        stream.cancelQueued("q1")
        advanceUntilIdle()
        assertNotNull(stream.queueFailure.value)
        runner.refuse = null
        stream.editQueued("q2", "x")
        advanceUntilIdle()
        assertNull(stream.queueFailure.value)
    }

    @Test
    fun aTakenCallSaysNothing() = runTest {
        val stream = stream(Runner())
        stream.editQueued("q1", "x")
        advanceUntilIdle()
        assertNull(stream.queueFailure.value)
    }
}
