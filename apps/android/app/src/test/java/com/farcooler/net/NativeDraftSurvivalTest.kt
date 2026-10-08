package com.farcooler.net

import com.farcooler.core.CoreException
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test

/** The conversation composer's draft survives the pane's model going away (ov-369 F4, R-38), and a send clears it only once confirmed. */
class NativeDraftSurvivalTest {
    private val running = TestScope()
    private val memory = InMemoryPaneViews()
    private var answer: suspend () -> Boolean = { false }

    @After
    fun tearDown() = running.close()

    private fun model() = NativePaneModel(
        terminal = "t1",
        store = AgentRowStore(running.scope, retryDelayMs = 1, followWaitMs = 1),
        source = FakeRowSource(),
        sink = ConversationSink { _, _, _ -> answer() },
        memory = memory,
        scope = running.scope,
        draftDelayMs = 1,
    )

    @Test
    fun `a draft survives the model being made again`() = runBlocking {
        val first = model()
        first.onDraft("fix the login, then the tests")
        first.draftSaved()
        assertEquals("fix the login, then the tests", model().draft)
    }

    @Test
    fun `a send the runner confirmed clears the saved draft`() = runBlocking {
        val first = model()
        first.onDraft("ship it")
        first.draftSaved()
        first.send()
        eventually("sent") { first.sent == 1 }
        assertEquals("", memory.draft("t1"))
        assertEquals("", model().draft)
    }

    @Test
    fun `a send the runner didn't confirm keeps the saved draft`() = runBlocking {
        answer = { throw CoreException("no", word = "resource-conflict", what = "draft") }
        val first = model()
        first.onDraft("ship it")
        first.draftSaved()
        first.send()
        eventually("refused") { first.issue != null && !first.sending }
        assertNotNull(first.issue)
        assertEquals("ship it", model().draft)
    }

    @Test
    fun `a long draft is cut to 64 KB on a character boundary`() {
        val text = "é".repeat(40_000)
        val saved = NativeDraft.capped(text)
        assertEquals(NativeDraft.CAP_BYTES, saved.toByteArray().size)
        assertEquals(true, text.startsWith(saved))
    }
}
