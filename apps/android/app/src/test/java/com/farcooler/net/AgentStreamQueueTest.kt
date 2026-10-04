package com.farcooler.net

import com.farcooler.core.ClientCore
import com.farcooler.core.CoreException
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

        suspend fun call(method: String, args: JsonObject): JsonObject {
            calls += method
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

    /** Mutation: `retryQueue` leaving the failure up. Red: the banner outlives the success. */
    @Test
    fun tryAgainRunsTheSameCallAndClearsOnSuccess() = runTest {
        val runner = Runner().apply { refuse = CoreException("raw", word = "agent-stopped") }
        val stream = stream(runner)
        stream.cancelQueued("q1")
        advanceUntilIdle()
        runner.refuse = null
        stream.retryQueue()
        advanceUntilIdle()
        assertNull(stream.queueFailure.value)
        assertEquals(listOf("terminal.agent_cancel_queued", "terminal.agent_cancel_queued"), runner.calls)
    }

    @Test
    fun aTakenCallSaysNothing() = runTest {
        val stream = stream(Runner())
        stream.editQueued("q1", "x")
        advanceUntilIdle()
        assertNull(stream.queueFailure.value)
    }
}
