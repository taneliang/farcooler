package com.farcooler.model

import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The page a terminal with no running pane opens to (ov-191), held to
 * `test/fixtures/lost-pane.json`, which AgentKit's `LostPaneTests` replays
 * too: every sentence whole, so the phone and the Mac can't drift apart.
 */
class LostPaneTest {
    private val root = Json.parseToJsonElement(repositoryFile("test/fixtures/lost-pane.json")).jsonObject

    private fun list(key: String) = root[key]!!.jsonArray.map { it.jsonObject }
    private fun JsonObject.text(key: String) = this[key]!!.jsonPrimitive.content

    @Test
    fun everyKindSaysWhatTheFixtureSays() {
        val kinds = list("kinds")
        assertEquals(3, kinds.size)
        for (each in kinds) {
            val state = each.text("state")
            val kind = LostPane.kind(StateKind.parse(state))!!
            assertEquals(state, each.text("title"), LostPane.title(kind))
            assertEquals(state, each.text("explanation"), LostPane.explanation(kind))
            assertEquals(state, each["actions"]!!.jsonArray.map { it.jsonPrimitive.content }, LostPane.actions(kind).map { it.title })
            assertEquals(state, each.text("shell_message"), LostPane.message(kind, "shell"))
        }
    }

    @Test
    fun aLiveOrUnreadPaneIsNotThisPage() {
        val states = root["not_this_page"]!!.jsonArray.map { it.jsonPrimitive.content }
        assertTrue(states.isNotEmpty())
        for (state in states) assertNull(state, LostPane.kind(StateKind.parse(state)))
    }

    /** Restart with and without a recorded command. */
    @Test
    fun restartNotesAreTheFixtures() {
        val notes = list("restart_notes")
        assertTrue(notes.size >= 6)
        for (each in notes) assertEquals(each.text("preset"), each.text("note"), LostPane.restartNote(each.text("preset")))
    }

    @Test
    fun dismissAndFailuresAreTheFixtures() {
        assertEquals(root["dismiss_note"]!!.jsonPrimitive.content, LostPane.DISMISS_NOTE)
        val failures = root["failures"]!!.jsonObject
        for (action in LostPane.Action.entries) {
            assertEquals(action.title, failures[action.title]!!.jsonPrimitive.content, action.failure)
        }
    }

    /**
     * When a not-live pane re-attaches: after a Restart, among others. Without
     * this a restarted lost pane stayed on "Not live" for good.
     */
    @Test
    fun revivalIsTheFixtures() {
        val cases = list("revives")
        assertTrue(cases.size >= 10)
        for (each in cases) {
            val was = StateKind.parse(each.text("from"))
            val now = StateKind.parse(each.text("to"))
            assertEquals(each.toString(), each["revives"]!!.jsonPrimitive.boolean, NotLivePane.revives(was, now))
        }
    }

    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}")
    }
}
