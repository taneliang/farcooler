package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.core.DisconnectedException
import com.farcooler.core.TerminalTransport
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A refused restart, hide, reorder or pane switch says so (ov-180). Before,
 * each ran in `attempt {}` and dropped the result.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class HostActionsTest {
    private class Core(var fails: Exception? = null) : TerminalTransport {
        val calls = mutableListOf<String>()
        override suspend fun call(method: String, args: JsonObject): JsonObject {
            calls += method
            fails?.let { throw it }
            return JsonObject(emptyMap())
        }
        override suspend fun startStream(terminal: String, onChunk: (ByteArray) -> Unit, onEnd: (String?) -> Unit) = false
        override suspend fun stopStream(terminal: String) {}
    }

    /** What [block] makes the actions say, in order. */
    private suspend fun TestScope.said(core: Core, block: suspend (HostActions) -> Unit): List<String> {
        val said = mutableListOf<String>()
        val actions = HostActions(core, ActionNotices())
        val job = launch(UnconfinedTestDispatcher(testScheduler)) {
            actions.notices.sentences.toList(said)
        }
        block(actions)
        job.cancel()
        return said
    }

    @Test
    fun `a refused restart stop or dismiss names the step`() = runTest {
        val core = Core(CoreException("x"))
        val said = said(core) { a ->
            a.act(Connection.Action.RESTART, "t")
            a.act(Connection.Action.STOP, "t")
            a.act(Connection.Action.DISMISS_LOST, "t")
        }
        assertEquals(
            listOf(
                "Couldn’t restart this terminal.",
                "Couldn’t stop this terminal.",
                "Couldn’t dismiss this terminal.",
            ),
            said,
        )
    }

    @Test
    fun `a refusal the table knows adds its sentence`() = runTest {
        val core = Core(CoreException("x", word = "scope-denied"))
        val said = said(core) { it.setHidden("w", true) }
        assertEquals(1, said.size)
        assertTrue(said[0], said[0].startsWith("Couldn’t hide this worktree. This device can only look"))
    }

    @Test
    fun `a dropped link says the connection dropped`() = runTest {
        val core = Core(DisconnectedException("EPIPE"))
        val said = said(core) { it.setHidden("w", false) }
        assertEquals(listOf("Couldn’t unhide this worktree. The connection to this runner dropped."), said)
    }

    @Test
    fun `a refused reorder and pane switch each say so`() = runTest {
        val core = Core(CoreException("x"))
        val said = said(core) { a ->
            a.reorder(listOf("a", "b"))
            a.setPaneMode("t", "terminal")
            a.setPaneMode("t", "agent")
        }
        assertEquals(
            listOf(
                "Couldn’t save the new order.",
                "Couldn’t switch this pane to its terminal.",
                "Couldn’t switch this pane to its chat.",
            ),
            said,
        )
    }

    @Test
    fun `a call that works says nothing and still goes out`() = runTest {
        val core = Core()
        val said = said(core) { it.act(Connection.Action.RESTART, "t") }
        assertEquals(emptyList<String>(), said)
        assertEquals(listOf("terminal.restart"), core.calls)
    }
}
