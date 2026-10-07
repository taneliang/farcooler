package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.core.DisconnectedException
import com.farcooler.model.AgentConversation
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
        sink = ConversationSink { _, text, _ ->
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
        model.onDraft("hello")
        assertFalse(model.canSend)

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
    }

    @Test
    fun `sync follows live while offered, and lets go when not offered`() {
        val model = model()
        source.answerPage()
        model.sync(offered = true, live = true)
        eventually("following") { source.followCalls.get() >= 1 }
        // In front but not in the foreground.
        model.sync(offered = true, live = false)
        assertFalse(model.store.isFollowing)
        assertEquals(1, model.stops)
        model.sync(offered = true, live = true)
        assertTrue(model.store.isFollowing)
        // The setting went off: nothing is read for a pane that isn't offered.
        model.sync(offered = false, live = true)
        assertFalse(model.store.isFollowing)
        assertEquals(2, model.stops)
    }

    @Test
    fun `sync on a pane whose follow ended unavailable lets go and follows again once offered`() {
        val model = model()
        followUntilUnavailable(model)
        model.sync(offered = false, live = true)
        source.answerNothing()
        model.sync(offered = true, live = true)
        eventually("following again") { model.store.shown.value.phase == AgentRowStore.Phase.Live }
    }

    @Test
    fun `a pane removed means off screen, and that is a stop`() {
        val model = model()
        source.answerPage()
        model.sync(offered = true, live = true)
        eventually("following") { source.followCalls.get() >= 1 }
        model.removed()
        assertFalse(model.store.isFollowing)
        assertEquals(1, model.stops)
        // Removing a pane that wasn't following stops nothing twice.
        model.removed()
        assertEquals(1, model.stops)
    }

    @Test
    fun `an unavailable phase left behind doesn't send a pane that isn't followed to its terminal`() {
        val model = model()
        source.failPage(AgentRowsUnavailable())
        model.sync(offered = true, live = true)
        eventually("unavailable") { model.store.shown.value.phase == AgentRowStore.Phase.Unavailable }
        model.release()
        // A fresh composition while not live reads the phase the last follow left.
        model.phaseChanged()
        assertFalse(model.unavailable)
        assertTrue(model.showing)
    }

    @Test
    fun `an older page that keeps failing is asked for after a growing wait, never in a loop`() {
        assertEquals(0L, AgentRowStore.olderBackoffMs(0))
        assertEquals(500L, AgentRowStore.olderBackoffMs(1))
        assertEquals(1_000L, AgentRowStore.olderBackoffMs(2))
        assertEquals(10_000L, AgentRowStore.olderBackoffMs(9))
        assertEquals(10_000L, AgentRowStore.olderBackoffMs(500))
    }

    /** A held ask's answers, as the runner was asked them (ov-370). */
    private val answered = mutableListOf<String>()
    private var refuse: Throwable? = null

    private fun answering() = NativePaneModel(
        terminal = "t1",
        store = AgentRowStore(running.scope, retryDelayMs = 1, followWaitMs = 1),
        source = source,
        sink = ConversationSink { _, _, _ -> false },
        memory = memory,
        scope = running.scope,
        answers = AnswerSink { terminal, ask, option, given ->
            synchronized(answered) { answered.add("$terminal $ask $option $given") }
            refuse?.let { throw it }
        },
    )

    private val held = com.farcooler.model.AgentRow.Ask("Permission", "Bash touch x", "Bash", 1, false, held = "hook-ask-1")

    @Test
    fun anAnswerGoesToTheRunnerForThisPaneAndTheHeldId() {
        val model = answering()
        model.answer(held, AgentConversation.ALLOW)
        eventually("the answer landed") { model.answering == null && synchronized(answered) { answered.size == 1 } }
        assertEquals(listOf("t1 hook-ask-1 allow {}"), answered)
        val question = held.copy(kind = "Question")
        model.answer(question, AgentConversation.ANSWER, mapOf("Which color?" to "Blue"))
        eventually("the question's answer landed") { model.answering == null && synchronized(answered) { answered.size == 2 } }
        assertEquals("t1 hook-ask-1 answer {Which color?=Blue}", answered[1])
        assertTrue(model.answerIssues.isEmpty())
    }

    @Test
    fun anAskNothingHoldsIsNotAnswered() {
        val model = answering()
        model.answer(held.copy(held = null), AgentConversation.ALLOW)
        model.answer(held.copy(answered = true), AgentConversation.ALLOW)
        Thread.sleep(50)
        assertTrue(answered.isEmpty())
    }

    @Test
    fun aRefusedAnswerIsSaidByTheAsk() {
        refuse = CoreException("Someone already answered this.", word = "resource-conflict", what = "not_held")
        val model = answering()
        model.answer(held, AgentConversation.DENY)
        eventually("the refusal is said") { model.answerIssues["hook-ask-1"] != null }
        assertEquals(AgentConversation.answerIssue("not_held"), model.answerIssues["hook-ask-1"])
    }
}
