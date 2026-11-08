package com.farcooler.net

import com.farcooler.core.AgentWatchTransport
import com.farcooler.core.TerminalGrid
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Watching an engine agent: the picture arrives, the end is told apart, and nothing goes back (ov-976). */
@OptIn(ExperimentalCoroutinesApi::class)
class AgentWatchSessionTest {
    /** Records every call the session makes. The interface has two verbs, so this list is everything that can leave. */
    private class Recorder : AgentWatchTransport {
        val calls = mutableListOf<String>()
        var chunk: ((ByteArray) -> Unit)? = null
        var end: ((String?) -> Unit)? = null
        var opens = true
        override suspend fun startWatch(workspace: String, item: String, onChunk: (ByteArray) -> Unit, onEnd: (String?) -> Unit): Boolean {
            calls += "start $workspace $item"; chunk = onChunk; end = onEnd; return opens
        }
        override suspend fun stopWatch(workspace: String, item: String) { calls += "stop $workspace $item" }
    }

    private class FakeScreen(val columns: Int, val rows: Int) : WatchScreen {
        val fed = StringBuilder()
        override fun feed(bytes: ByteArray) { fed.append(String(bytes)) }
        override fun snapshot(): TerminalGrid? = null
        override fun free() {}
    }

    private fun marker(c: Int, r: Int) = "\u001bP>farcooler-size;$c;$r\u001b\\"

    @Test fun `the marker sizes the screen and the picture follows`() = runTest {
        val rec = Recorder(); var screen: FakeScreen? = null
        val d = StandardTestDispatcher(testScheduler)
        val s = AgentWatchSession("w", "ov-9", rec, d) { c, r -> FakeScreen(c, r).also { screen = it } }
        s.start(); advanceUntilIdle()
        // The marker split across two chunks, as ssh may deliver it.
        val bytes = (marker(120, 40) + "hello").toByteArray()
        rec.chunk!!(bytes.copyOfRange(0, 10)); advanceUntilIdle()
        assertEquals(AgentWatchSession.Phase.Connecting, s.phase.value)
        rec.chunk!!(bytes.copyOfRange(10, bytes.size)); advanceUntilIdle()
        assertEquals(120 to 40, screen!!.columns to screen!!.rows)
        assertEquals("hello", screen!!.fed.toString())
        assertEquals(AgentWatchSession.Phase.Live, s.phase.value)
    }

    @Test fun `finished, lost and not running are told apart`() = runTest {
        suspend fun run(end: String?, picture: Boolean): AgentWatchSession.Phase {
            val rec = Recorder()
            val s = AgentWatchSession("w", "i", rec, StandardTestDispatcher(testScheduler)) { c, r -> FakeScreen(c, r) }
            s.start(); advanceUntilIdle()
            if (picture) { rec.chunk!!(marker(80, 24).toByteArray()); advanceUntilIdle() }
            rec.end!!(end); advanceUntilIdle()
            return s.phase.value
        }
        assertEquals(AgentWatchSession.Phase.Finished, run(null, true))
        assertEquals(AgentWatchSession.Phase.Lost, run(AgentWatchTransport.LOST, true))
        assertEquals(AgentWatchSession.Phase.Unavailable(false), run(null, false))
        assertEquals("This agent isn’t running right now.", AgentWatchSession.status(AgentWatchSession.Phase.Unavailable(false)))
        assertEquals("Finished", AgentWatchSession.status(AgentWatchSession.Phase.Finished))
        assertNull(AgentWatchSession.status(AgentWatchSession.Phase.Live))
    }

    @Test fun `a refused member and a drop before any picture are not 'not running'`() = runTest {
        suspend fun run(end: String?): AgentWatchSession.Phase {
            val rec = Recorder()
            val s = AgentWatchSession("w", "i", rec, StandardTestDispatcher(testScheduler)) { c, r -> FakeScreen(c, r) }
            s.start(); advanceUntilIdle()
            rec.end!!(end); advanceUntilIdle()
            return s.phase.value
        }
        assertEquals(AgentWatchSession.Phase.Denied, run(AgentWatchTransport.DENIED))
        assertEquals(AgentWatchSession.Phase.Unreachable, run(AgentWatchTransport.LOST))
        assertEquals("Only the owner can watch an agent.", AgentWatchSession.status(AgentWatchSession.Phase.Denied))
        assertEquals("Couldn’t reach this agent.", AgentWatchSession.status(AgentWatchSession.Phase.Unreachable))
    }

    @Test fun `no input reaches the agent - the only calls are start and stop`() = runTest {
        val rec = Recorder()
        val s = AgentWatchSession("w", "ov-9", rec, StandardTestDispatcher(testScheduler)) { c, r -> FakeScreen(c, r) }
        s.start(); advanceUntilIdle()
        rec.chunk!!((marker(80, 24) + "\u001b[6n").toByteArray()); advanceUntilIdle() // a program asking for the cursor position
        s.stop(); advanceUntilIdle()
        assertEquals(listOf("start w ov-9", "stop w ov-9"), rec.calls)
        // The type that watching holds has exactly those two verbs: a third would have to be added here.
        val verbs = AgentWatchTransport::class.java.methods.map { it.name }.filter { it.endsWith("Watch") }.sorted()
        assertEquals(listOf("startWatch", "stopWatch"), verbs)
        assertTrue(AgentWatchTransport::class.java.methods.none { it.name.contains("write", true) || it.name.contains("send", true) || it.name.contains("resize", true) })
    }

    @Test fun `a runner that cannot open it says so`() = runTest {
        val rec = Recorder().apply { opens = false }
        val s = AgentWatchSession("w", "i", rec, StandardTestDispatcher(testScheduler))
        s.start(); advanceUntilIdle()
        assertEquals(AgentWatchSession.Phase.Unavailable(false), s.phase.value)
    }
}
