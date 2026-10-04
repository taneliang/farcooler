package com.farcooler.model

import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Task keys in terminal output (ov-215): AgentKit's `TaskKeyTerminalTests`, against the same fixture. */
class TerminalLinksTest {
    private fun fixtureIndex(): Pair<TaskKeyIndex, List<JsonObjectCase>> {
        val root = Json.parseToJsonElement(repositoryFile("test/fixtures/task-key-links.json")).jsonObject
        val prefixes = root.getValue("prefixes").jsonArray.map { it.jsonPrimitive.content }.toSet()
        val known = root.getValue("known").jsonArray.map { it.jsonPrimitive.content }
        val targets = known.associateWith { TaskKeyTarget("r1", "w", "t-$it", it) }
        val cases = root.getValue("cases").jsonArray.map { case ->
            JsonObjectCase(
                case.jsonObject.getValue("text").jsonPrimitive.content,
                case.jsonObject.getValue("links").jsonArray.map {
                    it.jsonObject.getValue("start").jsonPrimitive.int to it.jsonObject.getValue("key").jsonPrimitive.content
                },
            )
        }
        return TaskKeyIndex("r1", prefixes, targets) to cases
    }

    private data class JsonObjectCase(val text: String, val links: List<Pair<Int, String>>)

    /** What the core answers for the cell [cell] of a one-row screen showing [text]. */
    private fun wordAt(text: String, cell: Int): Pair<String, Int>? {
        fun space(c: Char) = c in '\u0009'..'\u000D' || c == ' '
        if (cell >= text.length || space(text[cell])) return null
        var lo = cell
        while (lo > 0 && !space(text[lo - 1])) lo -= 1
        var hi = cell
        while (hi + 1 < text.length && !space(text[hi + 1])) hi += 1
        return text.substring(lo, hi + 1) to cell - lo
    }

    @Test
    fun `every fixture key hits from each of its cells and no cell beside it does`() {
        val (index, cases) = fixtureIndex()
        assertTrue(cases.size >= 20)
        for (case in cases) {
            for (cell in 0..case.text.length) {
                val hit = wordAt(case.text, cell)?.let { (word, offset) -> TerminalLinks.taskLink(word, offset, index) }
                val link = case.links.firstOrNull { (start, key) -> cell >= start && cell < start + key.length }
                assertEquals("${case.text} cell $cell", link?.let { TaskKeyLinks.url("r1", it.second) }, hit)
            }
        }
    }

    @Test
    fun `a url wins over a key and a key wins over a paste`() {
        val (index, _) = fixtureIndex()
        val url = "https://x.dev/ov-190"
        assertEquals(url, TerminalLinks.resolve({ url }, { "ov-190" to 1 }, index))
        assertEquals(TaskKeyLinks.url("r1", "ov-190"), TerminalLinks.resolve({ null }, { "(ov-190)." to 4 }, index))
        assertNull("blank cell", TerminalLinks.resolve({ null }, { null }, index))
        assertNull("unknown key", TerminalLinks.resolve({ null }, { "ov-191" to 1 }, index))
        assertNull("version", TerminalLinks.resolve({ null }, { "ov-1.2" to 1 }, index))
        assertNull("no board read", TerminalLinks.resolve({ null }, { "ov-190" to 1 }, TaskKeyIndex.EMPTY))
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
