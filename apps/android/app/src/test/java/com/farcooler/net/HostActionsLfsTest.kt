package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.core.TerminalTransport
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Test

/** Try again for a worktree's large files asks the runner, and a refusal says so (ov-199). */
@OptIn(ExperimentalCoroutinesApi::class)
class HostActionsLfsTest {
    private class Core(var fails: Exception? = null) : TerminalTransport {
        val calls = mutableListOf<Pair<String, String?>>()
        override suspend fun call(method: String, args: JsonObject): JsonObject {
            calls += method to args["worktree"]?.jsonPrimitive?.content
            fails?.let { throw it }
            return JsonObject(emptyMap())
        }
        override suspend fun startStream(terminal: String, onChunk: (ByteArray) -> Unit, onEnd: (String?) -> Unit) = false
        override suspend fun stopStream(terminal: String) {}
    }

    private suspend fun TestScope.said(core: Core, block: suspend (HostActions) -> Unit): List<String> {
        val said = mutableListOf<String>()
        val actions = HostActions(core, ActionNotices())
        val job = launch(UnconfinedTestDispatcher(testScheduler)) { actions.notices.sentences.toList(said) }
        block(actions)
        job.cancel()
        return said
    }

    @Test
    fun `try again asks the runner for that worktree and says nothing when it answers`() = runTest {
        val core = Core()
        val said = said(core) { it.hydrateLfs("w-1") }
        assertEquals(listOf("worktree.hydrate_lfs" to "w-1"), core.calls)
        assertEquals(emptyList<String>(), said)
    }

    @Test
    fun `a runner that refuses is said to have`() = runTest {
        val core = Core(CoreException("x"))
        val said = said(core) { it.hydrateLfs("w-1") }
        assertEquals(listOf("Couldn’t ask the runner to try again."), said)
    }
}
