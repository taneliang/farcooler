package com.farcooler.notify

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Notifications about tasks on Android (ov-94): which channel each class goes
 * on, the five switches and their defaults, a decision's buttons, and an answer
 * from one of them sent exactly once.
 */
class TaskNoticesTest {
    private val data = mapOf(
        "kind" to "task", "task" to "ov-90", "runner" to "r-1", "event" to "decision",
        "noticeId" to "t:r-1:ov-90", "title" to "ov-90 Wake the agent on an answer",
        "body" to "Needs your decision · Which?", "options" to "[\"pdfkit\",\"pdf.js\"]",
    )

    @Test
    fun aTaskPushIsReadWhole() {
        val notice = TaskNotice.of(data)!!
        assertEquals("ov-90", notice.key)
        assertEquals("r-1", notice.runner)
        assertEquals("decision", notice.event)
        assertEquals("t:r-1:ov-90", notice.noticeId)
        assertEquals(listOf("pdfkit", "pdf.js"), notice.options)
        // A legacy decision carrying the task notice's fields is one, so it
        // keeps its buttons; an old runner's bare decision isn't.
        assertEquals(listOf("pdfkit", "pdf.js"), TaskNotice.of(data + ("kind" to "decision"))!!.options)
        assertNull(TaskNotice.of(mapOf("kind" to "decision", "task" to "ov-1")))
        assertNull(TaskNotice.of(data + ("task" to "")))
        // Options that aren't a JSON list are none, not a crash.
        assertEquals(emptyList<String>(), TaskNotice.of(data + ("options" to "pdfkit"))!!.options)
    }

    @Test
    fun eachClassHasItsOwnChannel() {
        for (event in TaskNotices.EVENTS) {
            assertEquals("tasks.$event", TaskNotices.channelFor(event))
        }
        // The push path picks by class too, and leaves agent notices as they were.
        assertEquals("tasks.review", NotificationCopy.channelForPush(mapOf("kind" to "task", "event" to "review")))
        assertEquals(Notifier.CHANNEL_BLOCKED, NotificationCopy.channelForPush(mapOf("status" to "blocked")))
        assertEquals(Notifier.CHANNEL_BLOCKED, NotificationCopy.channelForPush(mapOf("kind" to "decision")))
    }

    @Test
    fun theDefaultsAreTheThreeThatWaitOnAPerson() {
        val on = mutableMapOf<String, Boolean>()
        val events = { master: Boolean -> TaskNotices.events(master) { on[it] ?: TaskNotices.onByDefault(it) } }
        assertEquals(listOf("decision", "review", "blocked"), events(true))
        on["review"] = false
        on["done"] = true
        assertEquals(listOf("decision", "blocked", "done"), events(true))
        // The master switch off is no classes at all, at the relay too.
        assertEquals(emptyList<String>(), events(false))
        assertEquals(listOf("Needs a decision", "Ready for review", "Blocked", "Done", "New task"), TaskNotices.EVENTS.map(TaskNotices::title))
    }

    @Test
    fun aButtonSendsItsOptionAndAnswerSendsWhatWasTyped() {
        val options = listOf("pdfkit", "pdf.js")
        assertEquals("pdf.js", TaskNotices.answerOf(TaskNotices.ACTION_OPTION, 1, options, null))
        assertEquals("qpdf", TaskNotices.answerOf(TaskNotices.ACTION_TEXT, -1, options, "  qpdf \n"))
        assertNull(TaskNotices.answerOf(TaskNotices.ACTION_TEXT, -1, options, "   "))
        assertNull(TaskNotices.answerOf(TaskNotices.ACTION_OPTION, 5, options, null))
        assertNull(TaskNotices.answerOf("something.else", 0, options, null))
    }

    @Test
    fun anAnswerIsSentExactlyOnceHoweverOftenItIsTapped() = runBlocking {
        val desk = TaskAnswers()
        val answer = TaskAnswer(key = "ov-90", runner = "r-1", noticeId = "t:r-1:ov-90", body = "pdfkit")
        assertTrue(desk.submit(answer))
        assertFalse(desk.submit(answer), "a second tap on the same notice")
        assertFalse(desk.submit(answer.copy(body = "pdf.js")), "a second answer to the same notice")
        val sent = mutableListOf<TaskAnswer>()
        desk.deliver { sent += it; null }
        desk.deliver { sent += it; null }
        assertEquals(listOf(answer), sent)
    }

    @Test
    fun anAnswerThatDidNotLandCanBeTriedAgain() = runBlocking {
        val desk = TaskAnswers()
        val answer = TaskAnswer(key = "ov-90", runner = null, noticeId = "t:r-1:ov-90", body = "pdfkit")
        desk.submit(answer)
        val refusals = mutableListOf<String>()
        desk.deliver({ "Your phone can’t reach that runner right now." }) { _, why -> refusals += why }
        assertEquals(listOf("Your phone can’t reach that runner right now."), refusals)
        assertTrue("released, so a later tap is sent", desk.submit(answer))
    }

    /**
     * An answer moves somebody's work, so a locked phone can't send one: every
     * answer button asks for the device to be unlocked first, as iOS's
     * `.authenticationRequired` does (ov-94 review).
     */
    @Test
    fun everyAnswerButtonNeedsTheDeviceUnlocked() {
        val option = TaskNotices.answerAction("pdfkit", null, null)
        val typed = TaskNotices.answerAction("Answer", null, null)
        assertTrue(option.isAuthenticationRequired)
        assertTrue(typed.isAuthenticationRequired)
    }

    private fun assertFalse(condition: Boolean, message: String) = org.junit.Assert.assertFalse(message, condition)
}
