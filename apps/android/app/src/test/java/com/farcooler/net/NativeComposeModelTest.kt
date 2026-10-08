package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.model.AgentConversation
import com.farcooler.model.OutgoingImage
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The conversation composer at the Mac's parity (ov-404): lines, images, a slash
 * command, Stop and Send now, as the pane's model decides them (the iPhone's
 * `NativePaneModel` and its UI tests). The views are held by the capture-package
 * tests.
 */
class NativeComposeModelTest {
    private val running = TestScope()
    private val source = FakeRowSource()
    private val sent = CopyOnWriteArrayList<Triple<String, String, List<OutgoingImage>>>()
    private val pressed = CopyOnWriteArrayList<String>()
    private var answer: suspend () -> Boolean = { false }
    private var press: suspend (String) -> Unit = {}

    @After
    fun tearDown() = running.close()

    private fun model(rich: Boolean = true, interrupts: Boolean = true): NativePaneModel {
        val model = NativePaneModel(
            terminal = "t1",
            store = AgentRowStore(running.scope, retryDelayMs = 1, followWaitMs = 1),
            source = source,
            sink = ConversationSink { terminal, text, images ->
                sent.add(Triple(terminal, text, images))
                answer()
            },
            memory = InMemoryPaneViews(),
            scope = running.scope,
            interruptSink = object : InterruptSink {
                override suspend fun interrupt(terminal: String) {
                    press("stop")
                    pressed.add("stop")
                }

                override suspend fun sendNow(terminal: String) {
                    press("sendnow")
                    pressed.add("sendnow")
                }
            },
        )
        model.offer(rich, interrupts)
        return model
    }

    private fun live(model: NativePaneModel, page: kotlinx.serialization.json.JsonObject = RowJson.page(1, 2, RowJson.turn(0, "Hi"))) {
        source.answerPage(page)
        model.setOnScreen(true)
        eventually("rows") { model.store.shown.value.rows.isNotEmpty() }
    }

    private fun image(id: Int, mime: String = "image/png", bytes: Int = 8) = OutgoingImage(id, mime, ByteArray(bytes))

    // Lines.

    @Test
    fun `with compose the draft is as typed, and Return is a new line, not a send`() {
        val model = model()
        live(model)
        model.onDraft("one\ntwo")
        assertEquals("one\ntwo", model.draft)
        model.onDraft("one\ntwo\n")
        assertEquals("a trailing Return is a new line too", "one\ntwo\n", model.draft)
        assertEquals(0, sent.size)
        model.send()
        eventually("the send") { sent.size == 1 }
        // As typed: the runner trims the ends, so the lines are the draft's.
        assertEquals("one\ntwo\n", sent.single().second)
    }

    @Test
    fun `without compose it is one line, a Return sends, and a command is refused here`() {
        val model = model(rich = false)
        live(model)
        model.onDraft("one\ntwo")
        assertEquals("one two", model.draft)
        model.onDraft("one two\n")
        eventually("a Return sent it") { sent.size == 1 && model.draft.isEmpty() }
        model.onDraft("/clear")
        model.send()
        assertEquals(AgentConversation.SendIssue.Said(AgentConversation.COMMAND), model.issue)
        assertEquals(1, sent.size)
    }

    @Test
    fun `losing compose flattens what was typed and drops the images`() {
        val model = model()
        model.onDraft("a\nb")
        model.attach(listOf(image(1)))
        model.offer(rich = false, interrupts = true)
        assertEquals("a b", model.draft)
        assertTrue(model.images.isEmpty())
    }

    @Test
    fun `with compose a slash command goes to the runner, which drives claude's picker`() {
        val model = model()
        live(model)
        model.onDraft("/compact")
        model.send()
        eventually("the command") { sent.size == 1 }
        assertEquals("/compact", sent.single().second)
        assertNull(model.issue)
    }

    @Test
    fun `the longest message follows compose`() {
        val rich = model()
        rich.onDraft("x".repeat(AgentConversation.LONGEST + 1))
        assertTrue("500 is the one-line box's, not compose's", rich.canSend || rich.store.shown.value.isStale)
        rich.onDraft("x".repeat(AgentConversation.LONGEST_COMPOSED + 1))
        assertFalse(rich.canSend)
        rich.send()
        assertEquals(AgentConversation.SendIssue.Said(AgentConversation.TOO_LONG_COMPOSED), rich.issue)
        val plain = model(rich = false)
        plain.onDraft("x".repeat(AgentConversation.LONGEST + 1))
        plain.send()
        assertEquals(AgentConversation.SendIssue.Said(AgentConversation.TOO_LONG), plain.issue)
    }

    // Images.

    @Test
    fun `an image goes with the message, and a photo alone is a message`() {
        val model = model()
        live(model)
        assertFalse(model.canSend)
        model.attach(listOf(image(1), image(2)))
        assertTrue("a picture on its own is a message", model.canSend)
        model.onDraft("what is this")
        model.send()
        eventually("the send") { sent.size == 1 }
        assertEquals(listOf(1, 2), sent.single().third.map { it.id })
        eventually("the images gone with the send") { model.images.isEmpty() && !model.sending }
        assertEquals("", model.draft)
    }

    @Test
    fun `a failed send keeps the draft and the images`() {
        val model = model()
        live(model)
        model.attach(listOf(image(1)))
        model.onDraft("try this")
        answer = { throw CoreException("no", "invalid-argument", "image_too_large") }
        model.send()
        eventually("the issue") { model.issue != null }
        assertEquals(AgentConversation.SendIssue.Said(AgentConversation.IMAGE_TOO_LARGE), model.issue)
        assertEquals("try this", model.draft)
        assertEquals(1, model.images.size)
    }

    @Test
    fun `at most ten images, and a runner without compose takes none`() {
        val model = model()
        model.attach((0 until 12).map { image(it) })
        assertEquals(AgentConversation.MOST_IMAGES, model.images.size)
        assertEquals(AgentConversation.SendIssue.Said(AgentConversation.TOO_MANY_IMAGES), model.issue)
        assertEquals(0, model.imageRoom)
        model.detach(3)
        assertEquals(1, model.imageRoom)
        val plain = model(rich = false)
        plain.attach(listOf(image(1)))
        assertTrue(plain.images.isEmpty())
    }

    @Test
    fun `a queued message with images shows as Queued with its images, and settles against claude's row`() {
        val model = model()
        live(model)
        answer = { true }
        model.attach(listOf(image(1), image(2)))
        model.onDraft("look")
        model.send()
        eventually("queued") { model.queued == listOf("[Image] [Image] look") }
    }

    @Test
    fun `a picked photo is read, converted where the runner would not take it, and added`() = runBlocking {
        val model = model()
        val png = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 1, 2, 3)
        val heic = byteArrayOf(0, 0, 0, 0x18, 0x66, 0x74, 0x79, 0x70, 0x68, 0x65, 0x69, 0x63)
        val converted = CopyOnWriteArrayList<Int>()
        model.attachPicked(listOf(png, heic, null, "not a picture".toByteArray())) { bytes ->
            converted.add(bytes.size)
            if (bytes.contentEquals(heic)) OutgoingImage.Converted("image/jpeg", byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte())) else null
        }
        // The PNG is kept as it is; the HEIC is converted; the unloaded one and the non-image are said.
        assertEquals(listOf("image/png", "image/jpeg"), model.images.map { it.mime })
        assertEquals("the converter saw only what the runner wouldn't take as it is", 2, converted.size)
        assertEquals(AgentConversation.SendIssue.Said(AgentConversation.UNREADABLE_IMAGE), model.issue)
    }

    // The rule for what is kept.

    @Test
    fun `a ten megabyte photo is sent as it is`() {
        val jpeg = ByteArray(10 * 1024 * 1024 + 7).also { it[0] = 0xFF.toByte(); it[1] = 0xD8.toByte(); it[2] = 0xFF.toByte() }
        var converterCalled = false
        val image = OutgoingImage.make(1, jpeg) { converterCalled = true; null }
        assertEquals("image/jpeg", image?.mime)
        assertTrue("the bytes weren't touched", image?.data === jpeg)
        assertFalse(converterCalled)
    }

    @Test
    fun `a kept format past the limit is converted, and what is not an image is null`() {
        val big = ByteArray(OutgoingImage.LARGEST_KEPT + 1).also { it[0] = 0x89.toByte(); it[1] = 0x50; it[2] = 0x4E; it[3] = 0x47 }
        val small = byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte(), 9)
        val image = OutgoingImage.make(1, big) { OutgoingImage.Converted("image/jpeg", small) }
        assertEquals("image/jpeg", image?.mime)
        assertTrue(image?.data === small)
        assertNull(OutgoingImage.make(2, "nope".toByteArray()) { null })
        assertEquals("image/webp", OutgoingImage.sniff("RIFF\u0000\u0000\u0000\u0000WEBPVP8 ".toByteArray(Charsets.ISO_8859_1)))
        assertEquals("image/gif", OutgoingImage.sniff("GIF89a".toByteArray()))
        assertNull(OutgoingImage.sniff(ByteArray(0)))
    }

    // Stop and Send now.

    private fun working(model: NativePaneModel, activity: String = "Busy") =
        live(model, RowJson.page(1, 2, RowJson.turn(0, "Go", outcome = null, activity = activity)))

    @Test
    fun `Stop is offered only while claude works, and the runner serves it`() {
        val busy = model()
        working(busy)
        assertTrue(busy.offersStop)
        assertTrue(busy.offersSendNow)
        val waiting = model()
        working(waiting, "Waiting")
        assertFalse("an Esc under a dialog would answer it No", waiting.offersStop)
        val idle = model()
        live(idle)
        assertFalse("the newest turn finished", idle.offersStop)
        val unserved = model(interrupts = false)
        working(unserved)
        assertFalse("a runner without terminal_interrupt", unserved.offersStop)
    }

    @Test
    fun `Stop and Send now press their keys once, and a refusal says which key to try again`() {
        val model = model()
        working(model)
        model.stop()
        eventually("stop") { pressed.toList() == listOf("stop") && model.pressing == null }
        model.sendNow()
        eventually("send now") { pressed.toList() == listOf("stop", "sendnow") && model.pressing == null }
        press = { throw CoreException("settling", "resource-conflict", "settling") }
        model.stop()
        eventually("the issue") { model.issue != null }
        assertEquals(AgentConversation.SendIssue.Said("Claude is starting a step. Try Stop again in a moment."), model.issue)
        model.sendNow()
        eventually("the other key's words") {
            model.issue == AgentConversation.SendIssue.Said("Claude is starting a step. Try Send now again in a moment.")
        }
    }

    @Test
    fun `a key that did nothing needed says nothing`() {
        val model = model()
        working(model)
        press = { throw CoreException("idle", "resource-conflict", "idle") }
        model.stop()
        eventually("the press ended") { model.pressing == null }
        assertNull(model.issue)
    }

    /** Neither key is offered over rows the runner stopped answering for, though claude was working when they were read. */
    @Test
    fun `neither key is offered over stale rows, and Send now not under a dialog`() {
        val stale = model()
        working(stale)
        assertTrue(stale.offersSendNow)
        source.failFollow(CoreException("The runner took too long to answer."))
        eventually("stale") { stale.store.shown.value.isStale }
        assertFalse(stale.offersStop)
        assertFalse("Send now on a stale pane", stale.offersSendNow)
        stale.sendNow()
        assertEquals("a press over stale rows is a no-op", emptyList<String>(), pressed.toList())
        val waiting = model()
        working(waiting, "Waiting")
        assertFalse("Send now under a dialog", waiting.offersSendNow)
    }

    @Test
    fun `the composed limit's words say the number the limit is`() {
        assertTrue(AgentConversation.TOO_LONG_COMPOSED.contains(String.format(java.util.Locale.US, "%,d", AgentConversation.LONGEST_COMPOSED)))
    }

    // claude's suggested prompt (ov-409).

    private fun resting(suggestion: String?, activity: String? = "Idle") =
        RowJson.page(1, 2, RowJson.turn(0, "Hi", activity = activity, suggestion = suggestion))

    @Test
    fun `claude's suggestion is the placeholder, and taking it makes a draft that nothing sends`() {
        val model = model()
        live(model, resting("run the tests again"))
        assertEquals("run the tests again", model.suggestion)
        assertTrue(model.takeSuggestion())
        assertEquals("run the tests again", model.draft)
        assertNull("a draft hides it", model.suggestion)
        assertFalse("and a second take has nothing to take", model.takeSuggestion())
        Thread.sleep(300)
        assertEquals("never sent on its own", 0, sent.size)
        // Edited, then sent by the person's own Send.
        model.onDraft(model.draft + " please")
        model.send()
        eventually("the send") { sent.size == 1 }
        assertEquals("run the tests again please", sent[0].second)
    }

    @Test
    fun `no suggestion while the agent works, holds a dialog, or the rows may be old`() {
        val busy = model()
        live(busy, resting("run the tests again", activity = "Busy"))
        assertNull(busy.suggestion)
        assertFalse(busy.takeSuggestion())
        assertEquals("", busy.draft)

        val waiting = model()
        live(waiting, resting("run the tests again", activity = "Waiting"))
        assertNull(waiting.suggestion)

        val none = model()
        live(none, resting(null))
        assertNull(none.suggestion)
    }

    @Test
    fun `the suggestion rules, on their own`() {
        val turn = com.farcooler.model.AgentRow.Turn("go", "Typed", activity = "Idle", suggestion = " do it ")
        assertEquals("do it", AgentConversation.suggestion(turn, "", stale = false))
        assertEquals("do it", AgentConversation.suggestion(turn, " \n", stale = false))
        assertNull(AgentConversation.suggestion(turn, "fix", stale = false))
        assertNull(AgentConversation.suggestion(turn, "", stale = true))
        assertNull(AgentConversation.suggestion(turn.copy(suggestion = "  "), "", stale = false))
        assertNull(AgentConversation.suggestion(null, "", stale = false))
    }

    @Test
    fun `claude's Try example is the placeholder, and neither a tap nor Tab takes it`() {
        val example = "Try \"how does <filepath> work?\""
        val hint = RowJson.hint(1, example)
        val model = model()
        live(model, RowJson.page(1, 2, RowJson.turn(0, "Hi", activity = "Idle"), hint))
        assertEquals(example, model.hint)
        assertNull("an example is not a prediction", model.suggestion)
        assertFalse(model.takeSuggestion())
        assertEquals("", model.draft)
        model.onDraft("x")
        assertNull("typing wins", model.hint)
        model.onDraft("")
        val rows = model.store.shown.value.rows
        assertEquals(null, AgentConversation.hint(rows, "", stale = true))
        assertEquals(example, AgentConversation.hint(rows, "", stale = false))
        // Emptied once the box shows something else.
        val emptied = model()
        live(emptied, RowJson.page(1, 2, RowJson.turn(0, "Hi"), RowJson.hint(1, "")))
        assertNull(emptied.hint)
    }
}
