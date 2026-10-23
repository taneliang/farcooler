package com.farcooler.model

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A phone marks a lane's pull request ready and replies to a review thread (ov-317): AgentKit's `PhonePrActionsTests`.
 * The runner's answers are `test/fixtures/pr-actions/`, written by `the_answers_are_the_bytes_the_phones_read` in
 * `crates/client` from the wire messages, so the decoder is fed what the runner sends.
 */
class PrActionsTest {
    private fun fixture(name: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            File(directory, "test/fixtures/pr-actions/$name").takeIf { it.isFile }?.let { return it.readText() }
            directory = directory.parentFile
        }
        throw AssertionError("Could not find test/fixtures/pr-actions/$name above ${System.getProperty("user.dir")}.")
    }

    /** AgentKit's `onlyTheOwnerActsOnPullRequests` (ov-712): both post as the owner's `gh` login. */
    @Test
    fun `a member's phone is offered no Mark ready or Reply`() {
        fun build(capabilities: Set<String>, scope: String) =
            DaemonBuild(version = "t", matches = true, platform = "t", capabilities = capabilities, grantedScope = scope)
        val actions = build(setOf("pr_actions"), "control")
        assertTrue("the owner's phone", actions.mayActOnPullRequests(""))
        assertTrue("not said yet, as Keep", actions.mayActOnPullRequests(null))
        assertFalse("a member's phone", actions.mayActOnPullRequests("bob"))
        assertFalse("a released one", actions.mayActOnPullRequests("(released)"))
        assertFalse("a Read grant", build(setOf("pr_actions"), "read").mayActOnPullRequests(""))
        assertFalse("a runner without them", build(emptySet(), "control").mayActOnPullRequests(""))
    }

    @Test
    fun `Mark ready names the lane and the number the row showed, never a URL`() {
        val request = PrActions.readyRequest("lane-id", 41)
        assertEquals(listOf("lane" to "lane-id", "pr_number" to 41), request)
        assertEquals(listOf("task" to "task-id", "body" to "Done.", "send" to false), PrActions.replyRequest("task-id", "Done.", false))
        assertEquals("Mark PR 41 ready for review?", PrActions.readyTitle(PlanPrStage(PlanPrStageKind.AGENT_REVIEW, prNumber = 41)))
        assertEquals("Mark ready for review?", PrActions.readyTitle(PlanPrStage(PlanPrStageKind.AGENT_REVIEW)))
    }

    @Test
    fun `a reply is trimmed, never empty, and at most 2000 characters`() {
        assertEquals("Thanks, fixed.", PrActions.reply("  Thanks, fixed.\n"))
        assertNull(PrActions.reply(""))
        assertNull(PrActions.reply(" \n "))
        assertNotNull(PrActions.reply("x".repeat(2000)))
        assertNull(PrActions.reply("x".repeat(2001)))
        assertNotNull("characters, as the runner counts", PrActions.reply("é".repeat(2000)))
        assertNotNull("an emoji is one scalar, two UTF-16 units", PrActions.reply("👍".repeat(2000)))
        assertNull("a family is seven scalars: 286 are past the cap", PrActions.reply("👨‍👩‍👧‍👦".repeat(286)))
        assertNotNull(PrActions.reply("👨‍👩‍👧‍👦".repeat(285)))
    }

    @Test
    fun `the runner's preview names the pull request and the login the reply posts as`() {
        val preview = PrActions.ReplyPreview.decode(fixture("reply-preview.json"))!!
        assertEquals("acme/billing #213", preview.pullRequest)
        assertEquals("e-liang", preview.login)
        assertTrue(preview.inThread)
        assertEquals("01000000-0000-7000-8000-000000000015", preview.noteId)
        assertEquals("9f2c41d07b3a5e8812ac6d00", preview.digest)
        val send = PrActions.replyRequest("t", "Hi", true, preview).toMap()
        assertEquals("the send is bound to the preview", preview.noteId to preview.digest, send["note_id"] to send["digest"])
        assertEquals(listOf("task", "body", "send"), PrActions.replyRequest("t", "", false).map { it.first })
        assertEquals("Reply on acme/billing #213?", preview.title)
        assertEquals("Posts your reply in this thread as e-liang on GitHub.", preview.message)
        assertEquals("Posts your reply on the pull request as e-liang on GitHub.", preview.copy(inThread = false).message)
        for (bad in listOf("{}", """{"repository":"a/b","number":0,"login":"x","note_id":"n","digest":"d"}""", """{"repository":"a/b","number":3,"login":"","note_id":"n","digest":"d"}""", """{"repository":"a/b","number":3,"login":"x"}""", "nope", "[]")) {
            assertNull(bad, PrActions.ReplyPreview.decode(bad))
        }
    }

    @Test
    fun `Mark ready's answer decodes with the fixture the runner's test writes`() {
        val answer = kotlinx.serialization.json.Json.parseToJsonElement(fixture("ready.json")).let { it as kotlinx.serialization.json.JsonObject }
        assertEquals("31", answer["number"].toString())
        assertEquals("false", answer["already_ready"].toString())
        assertNotNull(GitHubLink.url(answer["url"]?.let { (it as kotlinx.serialization.json.JsonPrimitive).content }))
    }

    @Test
    fun `a refusal says what happened in words, never the runner's`() {
        assertTrue(PrActions.refusal("invalid-argument", "lane_pr", "mark it ready").contains("no draft pull request"))
        assertTrue(PrActions.refusal("invalid-argument", "thread_link", "reply").contains("Open it there"))
        assertTrue(PrActions.refusal("invalid-argument", "reply_body", "reply").contains("2000"))
        assertEquals("This thread was already answered.", PrActions.refusal("invalid-argument", "already_answered", "reply"))
        assertTrue(PrActions.refusal("pr-state-unavailable", null, "reply").contains("GitHub didn’t answer"))
        assertEquals("Couldn’t mark it ready. Try again.", PrActions.refusal(null, null, "mark it ready"))
        assertTrue(PrActions.refusal("scope-denied", null, "reply").contains("only look"))
        for (said in listOf(PrActions.refusal("pr-state-unavailable", null, "reply"), PrActions.refusal("weird-new-code", null, "reply"))) {
            assertFalse(said, said.contains("pr-state") || said.contains("_") || said.contains("weird-new-code"))
        }
    }

    @Test
    fun `an excerpt cuts a long reply for the confirmation and keeps a short one whole`() {
        assertEquals("Short.", PrActions.excerpt("Short."))
        val cut = PrActions.excerpt("x".repeat(500))
        assertEquals(280, cut.length)
        assertTrue(cut.endsWith("…"))
    }

    @Test
    fun `Mark ready and Reply are offered only where the runner has them and this device holds control`() {
        fun build(vararg capabilities: String, scope: String = "control") =
            DaemonBuild(version = "x", matches = true, platform = "x", capabilities = capabilities.toSet(), grantedScope = scope)
        assertTrue(build("pr_actions").mayActOnPullRequests(""))
        assertFalse("an older runner", build("tasks").mayActOnPullRequests(""))
        assertFalse("a Read grant", build("pr_actions", scope = "read").mayActOnPullRequests(""))
        assertTrue(build("pr_actions", scope = "host_admin").mayActOnPullRequests(""))
    }
}
