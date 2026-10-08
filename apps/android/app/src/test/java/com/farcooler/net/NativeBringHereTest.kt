package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.model.AgentConversation
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Bring here on the pane's model (ov-369, R-28): the iPhone's and the Mac's `bringHere()`. */
class NativeBringHereTest {
    private val running = TestScope()
    private val calls = CopyOnWriteArrayList<String>()
    private var box = "fix the login\nthen the tests"
    private var clearFails: Exception? = null

    @After
    fun tearDown() = running.close()

    private fun model(bring: Boolean = true, preset: String = "claude"): NativePaneModel {
        val model = NativePaneModel(
            terminal = "t1",
            store = AgentRowStore(running.scope, retryDelayMs = 1, followWaitMs = 1),
            source = FakeRowSource(),
            sink = ConversationSink { _, _, _ -> false },
            memory = InMemoryPaneViews(),
            scope = running.scope,
            draftSink = DraftSink { terminal, expected ->
                if (expected == null) {
                    calls += "read $terminal"
                    box to false
                } else {
                    calls += "clear $expected"
                    clearFails?.let { throw it }
                    box = ""
                    expected to true
                }
            },
        )
        model.preset = preset
        model.offer(rich = true, interrupts = true, bring = bring)
        return model
    }

    @Test
    fun `offered for claude where the runner serves it`() {
        assertTrue(model().offersBringHere)
        assertFalse(model(bring = false).offersBringHere)
        assertFalse(model(preset = "codex").offersBringHere)
    }

    @Test
    fun `the box's text goes ahead of the composer's, and the box is cleared of exactly that`() {
        val model = model()
        model.onDraft("and the docs")
        model.bringHere()
        eventually("brought") { !model.bringing && calls.size == 2 }
        assertEquals("fix the login\nthen the tests\nand the docs", model.draft)
        assertEquals(listOf("read t1", "clear fix the login\nthen the tests"), calls.toList())
        assertEquals(null, model.issue)
    }

    @Test
    fun `a clear that fails keeps the text here and says so`() {
        clearFails = CoreException("changed", word = "resource-conflict", what = "changed")
        val model = model()
        model.bringHere()
        eventually("tried") { !model.bringing && calls.size == 2 }
        assertEquals("fix the login\nthen the tests", model.draft)
        assertTrue(model.issue is AgentConversation.SendIssue.DraftLeftInTerminal)
    }

    @Test
    fun `without the runner's bring_draft, nothing is asked`() {
        val model = model(bring = false)
        model.bringHere()
        Thread.sleep(100)
        assertEquals(emptyList<String>(), calls.toList())
    }
}
