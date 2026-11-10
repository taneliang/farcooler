package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.util.Locale

/**
 * What a task's Usage section says (ov-195), from `test/fixtures/task-usage.json`,
 * which AgentKit's `TaskUsageTests` and the CLI's `usage_words` read too, so the
 * Mac, iOS, Android and `farcooler report` word spend the same way.
 */
class TaskUsageTest {
    private val fixture = Json.parseToJsonElement(repositoryFile("test/fixtures/task-usage.json")).jsonObject
    private val locale = Locale.forLanguageTag(fixture.string("locale").replace('_', '-'))

    @Test
    fun `token counts, dollars and agent time read as the shared fixture says`() {
        for (c in fixture["tokens"]!!.jsonArray.map { it.jsonObject }) {
            assertEquals("${c["n"]}", c.string("text"), TaskUsageFormat.tokens(c["n"]!!.jsonPrimitive.long, locale))
        }
        for (c in fixture["dollars"]!!.jsonArray.map { it.jsonObject }) {
            assertEquals("${c["micros"]}", c.string("text"), TaskUsageFormat.dollars(c["micros"]!!.jsonPrimitive.long, locale))
        }
        for (c in fixture["durations"]!!.jsonArray.map { it.jsonObject }) {
            assertEquals("${c["ms"]}", c.string("text"), TaskUsageFormat.duration(c["ms"]!!.jsonPrimitive.long))
        }
    }

    /** `test/fixtures/task-usage-shown.json`: the client core's own answer for each case. */
    private val shown = Json.parseToJsonElement(repositoryFile("test/fixtures/task-usage-shown.json")).jsonArray
        .associate { it.jsonObject.string("case") to TaskUsage.decode(it.jsonObject["shown"]!!.jsonObject) }

    @Test
    fun `the client core's answer for each case decodes to the lines the shared fixture says`() {
        val cases = fixture["cases"]!!.jsonArray.map { it.jsonObject }
        assertTrue("the fixture has its cases", cases.size >= 9)
        for (case in cases) {
            val name = case.string("case")
            val said = shown[name]!!.said
            assertEquals("$name: empty", case["empty"]!!.jsonPrimitive.boolean, said.nothing != null)
            if (said.nothing != null) continue
            assertEquals("$name: tokens", case.maybe("tokens"), said.tokens)
            assertEquals("$name: detail", case.maybe("token_detail"), said.tokenDetail)
            assertEquals("$name: cost", case.maybe("cost"), said.cost)
            assertEquals("$name: time", case.maybe("time"), said.time)
            val expected = case["rows"]!!.jsonArray.map { it.jsonObject.let { r -> r.string("title") to r.string("detail") } }
                .map { (title, detail) -> title.split(" · ", limit = 2).let { if (it.size == 2) "${it[0]} · ${PlanWords.model(it[1])}" else title } to detail }
            assertEquals("$name: rows", expected, said.rows.map { it.title to it.detail })
        }
    }

    @Test
    fun `the section's sentences are the shared fixture's`() {
        val words = fixture["words"]!!.jsonObject
        assertEquals(words.string("nothing_yet"), shown["nothing recorded"]!!.said.nothing)
        assertEquals(words.string("needs_update"), TaskUsageFormat.NEEDS_UPDATE)
        assertEquals(words.string("couldnt_read"), TaskUsageFormat.COULDNT_READ)
        assertEquals(words.string("api_equivalent"), TaskUsageFormat.API_EQUIVALENT)
        // The button is Android's own, in sentence case (ov-204).
        assertEquals("Try again", TaskUsageFormat.TRY_AGAIN)
    }

    @Test
    fun `an older runner needs an update, and a read that didn't come back failed`() {
        val usage = shown["nothing recorded"]!!
        assertEquals(TaskUsageState.NeedsUpdate, TaskUsageState.after(null, runnerCan = false))
        assertEquals(TaskUsageState.NeedsUpdate, TaskUsageState.after(usage, runnerCan = false))
        assertEquals(TaskUsageState.Failed, TaskUsageState.after(null, runnerCan = true))
        assertEquals(TaskUsageState.Failed, TaskUsageState.after(null, runnerCan = null))
        assertEquals(TaskUsageState.Loaded(usage), TaskUsageState.after(usage, runnerCan = null))
    }

    @Test
    fun `another locale's digits, not en_US's`() {
        assertEquals("1,2M", TaskUsageFormat.tokens(1_155_400, Locale.GERMANY))
        assertTrue(TaskUsageFormat.dollars(3_200_000, Locale.GERMANY).contains("3,20"))
    }

    private fun JsonObject.string(key: String) = this[key]!!.jsonPrimitive.content
    private fun JsonObject.maybe(key: String) = this[key].takeUnless { it is JsonNull }?.jsonPrimitive?.contentOrNull

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
