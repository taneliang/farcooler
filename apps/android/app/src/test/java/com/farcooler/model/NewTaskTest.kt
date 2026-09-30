package com.farcooler.model

import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * New Task… on the board (ov-62): who's offered it, which titles it takes,
 * what it sends the client core's `task.create`, and what it says when the
 * runner refuses. The Mac's `NewTaskTests` hold the same rules.
 */
class NewTaskTest {
    private fun build(scope: String) =
        DaemonBuild(version = "1", matches = true, platform = "", capabilities = setOf("tasks"), grantedScope = scope)

    /** A Control-scope write, so a read-scoped connection sees no button. */
    @Test
    fun `a read-only connection is offered no New Task`() {
        assertFalse(NewTask.offered(build("read")))
        assertTrue(NewTask.offered(build("control")))
        assertTrue(NewTask.offered(build("host_admin")))
        // No answer, or one this build has no word for, is not a refusal.
        assertTrue(NewTask.offered(build("unspecified")))
        assertTrue(NewTask.offered(null))
    }

    /**
     * Held to the runner's `checked_title`: trimmed, not empty, and at most
     * 200 Unicode scalars. Code points, not chars: an emoji is two chars and
     * one scalar, and a flag is two scalars.
     */
    @Test
    fun `a title is measured as the runner measures it`() {
        assertTrue(NewTask.titleFits("a".repeat(200)))
        assertFalse(NewTask.titleFits("a".repeat(201)))
        assertTrue(NewTask.titleFits("  " + "a".repeat(200) + "  "))
        assertFalse(NewTask.titleFits("   "))
        assertFalse(NewTask.titleFits(""))
        // 200 emoji: 400 chars, 200 scalars.
        assertTrue(NewTask.titleFits("😀".repeat(200)))
        // 101 flags: 202 scalars.
        assertFalse(NewTask.titleFits("🇸🇬".repeat(101)))
        assertTrue(NewTask.titleFits("🇸🇬".repeat(100)))
    }

    /** The board it was chosen from, the title trimmed, and Details as the intent. */
    @Test
    fun `a task is filed on the board it was chosen from`() {
        val billing = WorkspaceSummary(id = "w-billing", name = "Billing", repository = "r-1")
        val sent = NewTask.request(billing, "  Fix the flaky test \n", "  It fails one run in ten.  ")
        assertEquals(JsonPrimitive("r-1"), sent["repository"])
        assertEquals(JsonPrimitive("w-billing"), sent["workspace"])
        assertEquals(JsonPrimitive("Fix the flaky test"), sent["title"])
        assertEquals(JsonPrimitive("It fails one run in ten."), sent["intent"])

        // No Details, no intent; a runner without workspaces names no workspace.
        val implicit = NewTask.request(WorkspaceSummary.implicit("r-2"), "Ship it", "   ")
        assertEquals(JsonPrimitive("r-2"), implicit["repository"])
        assertNull(implicit["workspace"])
        assertNull(implicit["intent"])
    }

    /** What a refused create says: this app's sentence, never the runner's words. */
    @Test
    fun `a refused create says why in the app's own words`() {
        assertEquals("A title can be at most 200 characters.", NewTask.refusal("invalid-argument", "title"))
        assertEquals(
            "This device can only look at this runner, so it can’t add tasks.",
            NewTask.refusal("scope-denied", null),
        )
        assertEquals(
            "This runner’s Far Cooler is too old to add tasks from a phone. Update it there, then try again.",
            NewTask.refusal("capability-unsupported", null),
        )
        assertEquals("This board isn’t on the runner anymore.", NewTask.refusal("not-found", null))
        assertEquals(
            "Couldn’t add that task. Check that the runner is reachable, then try again.",
            NewTask.refusal(null, null),
        )
        // Any other word, a newer runner's included, is still a sentence.
        val other = NewTask.refusal("something-new", null)
        assertEquals(
            "The runner couldn’t add that task. That’s a problem in the app, not in anything you typed.",
            other,
        )
        for (word in listOf("invalid-argument", "scope-denied", null, "something-new")) {
            assertNotEquals("", NewTask.refusal(word, null))
        }
    }
}
