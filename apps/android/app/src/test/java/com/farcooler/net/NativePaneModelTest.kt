package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.core.DisconnectedException
import com.farcooler.model.AgentConversation
import com.farcooler.model.AgentRow
import com.farcooler.model.RunnerRefusal
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The pane's conversation side: which view shows, the draft, the follow, the send (the iPhone's `NativePaneModel`). */
class NativePaneModelTest {
    private val running = TestScope()
    private val source = FakeRowSource()
    private val memory = InMemoryPaneViews()
    private val composed = mutableListOf<String>()
    private val sends = AtomicInteger()
    private var answer: suspend () -> Boolean = { false }

    @After
    fun tearDown() = running.close()

    private fun model(views: PaneViewMemory = memory) = NativePaneModel(
        terminal = "t1",
        store = AgentRowStore(running.scope, retryDelayMs = 1, followWaitMs = 1),
        source = source,
        sink = ConversationSink { _, text ->
            sends.incrementAndGet()
            composed.add(text)
            answer()
        },
        memory = views,
        scope = running.scope,
    )

    @Test
    fun `the conversation is the default, and each pane remembers its view (R-27)`() {
        val first = model()
        assertTrue(first.showing)
        first.switchTo(false)
        assertFalse(first.showing)
        // Another model for the same pane, as after the process restarts: the terminal.
        assertFalse(model().showing)
        // Another pane is untouched.
        assertTrue(memory.wantsConversation("t2"))
    }

    @Test
    fun `the switch keeps the draft`() {
        val model = model()
        model.onDraft("half a sentence")
        model.switchTo(false)
        model.switchTo(true)
        assertEquals("half a sentence", model.draft)
    }

    @Test
    fun `the draft is one line, and a trailing Return sends it`() {
        val model = model()
        model.onDraft("one\ntwo")
        assertEquals("one two", model.draft)
        model.setOnScreen(true)
        source.answerPage()
        eventually("rows") { model.store.shown.value.rows.isNotEmpty() }
        model.onDraft("one two\n")
        eventually("the send") { composed == listOf("one two") }
        eventually("draft cleared") { model.draft.isEmpty() && !model.sending }
    }

    @Test
    fun `the follow runs only while the pane is on screen, in front, and showing the conversation`() {
        val model = model()
        source.answerPage()
        // Not on screen: nothing is read.
        assertFalse(model.store.isFollowing)
        assertEquals(0, source.pageCalls.get())

        model.setOnScreen(true)
        eventually("following") { source.followCalls.get() >= 1 }

        // Backgrounded, or the tab left: stops.
        model.setOnScreen(false)
        assertFalse(model.store.isFollowing)
        assertEquals(1, model.stops)

        // On screen but behind the terminal: nothing is read.
        model.switchTo(false)
        model.setOnScreen(true)
        assertFalse(model.store.isFollowing)
        assertEquals(1, model.stops)

        // Back to the conversation: resumes.
        model.switchTo(true)
        assertTrue(model.store.isFollowing)
    }

    @Test
    fun `a pane removed stops its follow`() {
        val model = model()
        source.answerPage()
        model.setOnScreen(true)
        eventually("following") { source.followCalls.get() >= 1 }
        // What the pane's composable does when it leaves composition.
        model.setOnScreen(false)
        assertFalse(model.store.isFollowing)
    }

    /** A runner that served rows and then says it doesn't, as a projector turned off does. */
    private fun followUntilUnavailable(model: NativePaneModel) {
        source.answerPage()
        source.failFollow(AgentRowsUnavailable())
        model.setOnScreen(true)
        eventually("unavailable") { model.store.shown.value.phase == AgentRowStore.Phase.Unavailable }
        eventually("the loop to end") { !model.store.isFollowing }
        model.phaseChanged()
    }

    @Test
    fun `off then on follows again after the conversation was released`() {
        val model = model()
        followUntilUnavailable(model)
        // Rows were held, so the pane keeps its view with a stale banner over them.
        assertTrue(model.showing)
        assertTrue(model.store.shown.value.isStale)
        assertFalse(model.canSend.also { model.onDraft("hello") })

        // The setting went off and the link reconnected: the view stops being offered...
        model.release()
        // ...and on again: offered, on screen.
        source.answerNothing()
        model.setOnScreen(true)
        eventually("following again") { model.store.shown.value.phase == AgentRowStore.Phase.Live }
        assertFalse(model.store.shown.value.isStale)
    }

    @Test
    fun `a follow that ended unavailable restarts the next time the pane is due, even with nothing released`() {
        val model = model()
        followUntilUnavailable(model)
        source.answerNothing()
        // `following` is still set from the loop that ended; being due again must not trust it.
        model.setOnScreen(true)
        eventually("following again") { model.store.shown.value.phase == AgentRowStore.Phase.Live }
    }

    @Test
    fun `a pane whose rows are unavailable and none held falls back to its terminal until it is offered again`() {
        val model = model()
        source.failPage(AgentRowsUnavailable())
        model.setOnScreen(true)
        eventually("unavailable") { model.store.shown.value.phase == AgentRowStore.Phase.Unavailable }
        model.phaseChanged()
        assertTrue(model.unavailable)
        assertFalse("never a blank pane: the terminal shows", model.showing)
        model.release()
        assertFalse(model.unavailable)
        assertTrue(model.showing)
    }

    @Test
    fun `a send clears the draft, and a message claude queued shows as Queued until the transcript has it`() {
        val model = model()
        source.answerPage()
        model.setOnScreen(true)
        eventually("rows") { model.store.shown.value.rows.isNotEmpty() }
        answer = { true }
        model.onDraft("and then the docs")
        model.send()
        eventually("queued") { model.queued == listOf("and then the docs") }
        assertEquals("", model.draft)
        assertEquals(1, model.sent)
        // The fixture's page already holds a Queued row with this text: the echo settles.
        model.settleQueued()
        assertEquals(emptyList<String>(), model.queued)
    }

    @Test
    fun `a timed-out send keeps the draft and says it may have been sent`() {
        val model = model()
        source.answerPage()
        model.setOnScreen(true)
        eventually("rows") { model.store.shown.value.rows.isNotEmpty() }
        answer = { throw CoreException("late", RunnerRefusal.TIMED_OUT_WORD) }
        model.onDraft("run the tests")
        model.send()
        eventually("the issue") { model.issue != null }
        assertEquals(AgentConversation.SendIssue.Said(AgentConversation.MAY_HAVE_BEEN_SENT), model.issue)
        assertEquals("run the tests", model.draft)
        assertEquals(0, model.sent)
    }

    @Test
    fun `each failure is worded for what it says, and a dialog is a handoff`() {
        val model = model()
        source.answerPage()
        model.setOnScreen(true)
        eventually("rows") { model.store.shown.value.rows.isNotEmpty() }
        model.onDraft("hello")
        answer = { throw CoreException("no", "invalid-argument", "dialog") }
        model.send()
        eventually("handoff") { model.issue == AgentConversation.SendIssue.Handoff }
        answer = { throw DisconnectedException("gone", notSent = true) }
        model.send()
        eventually("not connected") {
            model.issue == AgentConversation.SendIssue.Said("The runner isn’t connected, so the message wasn’t sent.")
        }
        // The core closing under the call is not "wasn't sent": the runner may have it.
        answer = { throw CoreException("The connection was closed.") }
        model.send()
        eventually("may have been sent") {
            model.issue == AgentConversation.SendIssue.Said(AgentConversation.MAY_HAVE_BEEN_SENT)
        }
    }

    @Test
    fun `a command is refused before it reaches the runner, and so is a message that is too long`() {
        val model = model()
        source.answerPage()
        model.setOnScreen(true)
        eventually("rows") { model.store.shown.value.rows.isNotEmpty() }
        model.onDraft("/clear")
        model.send()
        assertEquals(AgentConversation.SendIssue.Said(AgentConversation.COMMAND), model.issue)
        model.onDraft("x".repeat(AgentConversation.LONGEST + 1))
        assertFalse(model.canSend)
        model.send()
        assertEquals(AgentConversation.SendIssue.Said(AgentConversation.TOO_LONG), model.issue)
        assertEquals(0, sends.get())
    }

    @Test
    fun `the box waits while the rows are stale`() {
        val model = model()
        source.answerPage()
        model.setOnScreen(true)
        eventually("live") { model.store.shown.value.phase == AgentRowStore.Phase.Live }
        model.onDraft("hello")
        assertTrue(model.canSend)
        source.failFollow(CoreException("The runner took too long to answer."))
        eventually("stale") { model.store.shown.value.isStale }
        assertFalse(model.canSend)
        assertNull(model.issue)
    }

    @Test
    fun `a send's outcome is kept by the model even when no view is there to see it`() {
        val model = model()
        source.answerPage()
        model.setOnScreen(true)
        eventually("rows") { model.store.shown.value.rows.isNotEmpty() }
        val gate = CompletableDeferred<Unit>()
        answer = { gate.await(); throw CoreException("late", RunnerRefusal.TIMED_OUT_WORD) }
        model.onDraft("hello")
        model.send()
        // The pane is removed while the send is in flight.
        model.setOnScreen(false)
        gate.complete(Unit)
        eventually("the outcome") { model.issue != null && !model.sending }
        assertEquals(AgentConversation.SendIssue.Said(AgentConversation.MAY_HAVE_BEEN_SENT), model.issue)
        assertTrue(model.draft == "hello")
        assertTrue(AgentRow::class.java.simpleName.isNotEmpty())
    }
}
