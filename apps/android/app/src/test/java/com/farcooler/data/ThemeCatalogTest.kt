package com.farcooler.data

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Which themes a phone with several runners offers, and which one is in force.
 *
 * The catalog rules are `test/fixtures/theme-catalog.json`, which AgentKit's
 * `ThemeCatalogTests` reads too, so the two phones cannot drift apart on them.
 */
class ThemeCatalogTest {
    private data class Named(val name: String, val background: Int)

    private fun named(element: kotlinx.serialization.json.JsonElement) = element.jsonObject.let {
        Named(it["name"]!!.jsonPrimitive.content, it["background"]!!.jsonPrimitive.int)
    }

    @Test
    fun `the shared fixture's catalogs`() {
        val root = Json.parseToJsonElement(repositoryFile("test/fixtures/theme-catalog.json")).jsonObject
        val builtIn = root["builtIn"]!!.jsonArray.map(::named)
        val cases = root["cases"]!!.jsonArray.map { it.jsonObject }
        assertTrue("the fixture has cases", cases.size >= 5)
        for (case in cases) {
            // Replayed in the order the runners answered, as a phone sees them.
            val byRunner = LinkedHashMap<String, List<Named>>()
            for (answer in case["answered"]!!.jsonArray.map { it.jsonObject }) {
                byRunner[answer.string("runner")] = answer["themes"]!!.jsonArray.map(::named)
            }
            val expected = case["catalog"]!!.jsonArray.map(::named)
            assertEquals(case.string("case"), expected, ThemeCatalog.merged(builtIn, byRunner) { it.name })
        }
    }

    private fun JsonObject.string(key: String) = this[key]!!.jsonPrimitive.content

    // ---- the singleton the app reads ----

    private fun theme(name: String, background: Int) =
        Theme(name, dark = true, background = background, foreground = 0, cursor = 0, ansi = List(16) { 0 })

    @After
    fun forgetTheRunners() {
        Themes.forget("a")
        Themes.forget("b")
        Themes.select("Nord")
    }

    /**
     * The user-visible bug: a theme from runner A, chosen, and then runner B
     * answers. The old merge rebuilt the catalog from B alone, the stored name
     * stopped resolving, and the app went Nord.
     */
    @Test
    fun `a theme from one runner stays in force after another answers`() {
        Themes.merge(listOf(theme("FromA", 10)), runner = "a")
        Themes.select("FromA")
        Themes.merge(listOf(theme("FromB", 11)), runner = "b")

        assertEquals("FromA", Themes.current.name)
        assertEquals(listOf("Nord", "FromA", "FromB"), Themes.available.value.map { it.name })
    }

    /** A removed runner takes its themes with it, and nothing else's. */
    @Test
    fun `forgetting a runner drops only its themes`() {
        Themes.merge(listOf(theme("FromA", 10)), runner = "a")
        Themes.merge(listOf(theme("FromB", 11)), runner = "b")
        Themes.forget("a")

        assertEquals(listOf("Nord", "FromB"), Themes.available.value.map { it.name })
    }

    /** A file in this checkout, found by walking up from wherever Gradle runs. */
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
