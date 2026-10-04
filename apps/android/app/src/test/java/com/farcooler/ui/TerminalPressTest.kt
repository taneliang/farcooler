package com.farcooler.ui

import com.farcooler.core.CellLinks
import com.farcooler.core.TerminalTransport
import com.farcooler.model.TaskKeyIndex
import com.farcooler.model.TaskKeyLinks
import com.farcooler.model.TaskKeyTarget
import com.farcooler.net.TerminalSession
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.serialization.json.JsonObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A long press on terminal output, through the calls the pane and its dialog
 * make (ov-215, review 1004j X3): [TerminalPress.linkAt] on a real
 * [TerminalSession], whose `linkAt` asks the cell's URL and word, and
 * [TerminalPress.open], which the dialog's Open runs.
 *
 * The JVM has no native core, so the session's link answers come from a
 * stand-in screen that answers as the core does: a URL on the second row, and
 * the whitespace-delimited word under a cell. `TerminalTaskKeyDeviceTest`
 * asks the real core the same questions on a device.
 */
class TerminalPressTest {
    /** Two rows: a sentence naming a key, and a URL ending in one. */
    private class Screen : CellLinks {
        val rows = listOf("done in ov-190.", "https://x.dev/ov-190")

        override fun urlAt(row: Int, column: Int): String? =
            if (row == 1 && column in rows[1].indices) rows[1] else null

        override fun wordAt(row: Int, column: Int): Pair<String, Int>? {
            val text = rows.getOrNull(row) ?: return null
            if (column !in text.indices || text[column] == ' ') return null
            var lo = column
            while (lo > 0 && text[lo - 1] != ' ') lo -= 1
            var hi = column
            while (hi + 1 < text.length && text[hi + 1] != ' ') hi += 1
            return text.substring(lo, hi + 1) to column - lo
        }
    }

    private object NoRunner : TerminalTransport {
        override suspend fun call(method: String, args: JsonObject): JsonObject = JsonObject(emptyMap())
        override suspend fun startStream(terminal: String, onChunk: (ByteArray) -> Unit, onEnd: (String?) -> Unit) = false
        override suspend fun stopStream(terminal: String) {}
    }

    private val scope = TestScope()
    private val session = TerminalSession("t", NoRunner, StandardTestDispatcher(scope.testScheduler), links = Screen())

    private val navigated = mutableListOf<Route>()
    private val target = TaskKeyTarget("r1", "w", "t-190", "ov-190")
    /** As `taskKeyLinker` makes it: each link handing its task's route on. */
    private val linker = TaskKeyLinker(TaskKeyIndex("r1", setOf("ov"), mapOf("ov-190" to target))) { navigated += it.route() }

    @After
    fun tearDown() {
        session.dispose()
        scope.runCurrent()
    }

    @Test
    fun `a long press on a key opens its task in the app and never through the system`() {
        val link = TerminalPress.linkAt(session, linker, column = 9, row = 0)
        assertEquals(TaskKeyLinks.url("r1", "ov-190"), link)

        val opened = mutableListOf<String>()
        assertNull(TerminalPress.open(link!!, linker) { opened += it })
        assertEquals(listOf<Route>(Route.BoardTask("r1", "w", "t-190")), navigated)
        assertTrue("the system opener was handed $opened", opened.isEmpty())
    }

    @Test
    fun `a press beside a key pastes, and a url wins over the key inside it`() {
        assertNull("the word \"in\"", TerminalPress.linkAt(session, linker, column = 6, row = 0))
        assertNull("a blank", TerminalPress.linkAt(session, linker, column = 4, row = 0))
        assertNull("no board read", TerminalPress.linkAt(session, TaskKeyLinker.NONE, column = 9, row = 0))

        val url = TerminalPress.linkAt(session, linker, column = 16, row = 1)
        assertEquals("https://x.dev/ov-190", url)
        val opened = mutableListOf<String>()
        assertNull(TerminalPress.open(url!!, linker) { opened += it })
        assertEquals(listOf("https://x.dev/ov-190"), opened)
        assertTrue(navigated.isEmpty())
    }
}
