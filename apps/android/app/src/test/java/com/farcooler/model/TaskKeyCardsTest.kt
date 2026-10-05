package com.farcooler.model

import com.farcooler.core.CellLinks
import com.farcooler.core.TerminalTransport
import com.farcooler.net.TerminalSession
import com.farcooler.ui.TaskKeyLinker
import com.farcooler.ui.TerminalPress
import com.farcooler.ui.taskKeyLinker
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.io.File

/**
 * A task key's card (ov-299): the CLI's own board and plan through this app's
 * parsers, the rules AgentKit's `TaskKeyCardsTests` holds, and a long press
 * on a terminal key through the pane's own hit-testing finding the card.
 */
class TaskKeyCardsTest {
    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}")
    }

    private val board = TaskBoard.decode(repositoryFile("test/fixtures/task-key-cards/tasks.json"))
    private val plan = Plan.decode(repositoryFile("test/fixtures/task-key-cards/plan.json"))
    private val workspace = WorkspaceSummary("w", taskPrefix = "bil")

    @Test
    fun `the CLI's board and plan give each key its card`() {
        val cards = TaskKeyCards.of("r1", mapOf("w" to board), mapOf("w" to plan))
        assertEquals("Total each invoice", cards.card("bil-1")?.title)
        assertEquals("In progress · Invoices · invoice-totals", cards.card("bil-1")?.details)
        assertEquals("Invoices show the sum of their lines.", cards.card("bil-1")?.excerpt)
        assertEquals("Invoices · rounding", cards.card("bil-2")?.details?.substringAfter(" · "))
        assertEquals("bil-1, Total each invoice", cards.card("bil-1")?.accessibilityLabel)
        assertNull(cards.card("bil-4"))
    }

    @Test
    fun `unknown keys and other runners' links show nothing`() {
        val linker = taskKeyLinker("r1", listOf(workspace), mapOf("w" to board), mapOf("w" to plan)) {}
        assertEquals("Round half-cents", linker.cardFor(TaskKeyLinks.url("r1", "bil-2"))?.title)
        assertNull(linker.cardFor(TaskKeyLinks.url("r2", "bil-2")))
        assertNull(linker.cardFor(TaskKeyLinks.url("r1", "bil-9")))
        assertNull(linker.cardFor("https://example.com/bil-2"))
        assertNull(TaskKeyLinker(linker.index, TaskKeyCards.of("r2", mapOf("w" to board))) {}.card("bil-1"))
    }

    @Test
    fun `a long-press excerpt is the first line, cut at a word`() {
        assertEquals("Why it matters", TaskKeyCard.excerpt("\n## Why it matters\nmore"))
        val cut = TaskKeyCard.excerpt("word ".repeat(60))
        assertEquals(true, cut.length <= TaskKeyCard.EXCERPT_LIMIT && cut.endsWith("word…"))
    }

    /** One row naming a key, answered as the core answers (see `TerminalPressTest`). */
    private class Screen : CellLinks {
        val text = "Working on bil-1 now"
        override fun urlAt(row: Int, column: Int): String? = null
        override fun wordAt(row: Int, column: Int): Pair<String, Int>? {
            if (row != 0 || column !in text.indices || text[column] == ' ') return null
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

    @Test
    fun `a long press on a key in a terminal finds its card through the pane's hit-testing`() {
        val scope = TestScope()
        val session = TerminalSession("t", NoRunner, StandardTestDispatcher(scope.testScheduler), links = Screen())
        try {
            val linker = taskKeyLinker("r1", listOf(workspace), mapOf("w" to board), mapOf("w" to plan)) {}
            val held = TerminalPress.linkAt(session, linker, column = 12, row = 0)
            assertEquals("Total each invoice", held?.let(linker::cardFor)?.title)
            assertNull(TerminalPress.linkAt(session, linker, column = 3, row = 0)?.let(linker::cardFor))
        } finally {
            session.dispose()
            scope.runCurrent()
        }
    }
}
