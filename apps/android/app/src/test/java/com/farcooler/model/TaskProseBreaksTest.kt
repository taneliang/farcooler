package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * A task's text breaks where it was broken (ov-198): a single newline is a line
 * break inside its paragraph, a blank line starts a new one, and lists, code and
 * links come out as they always did.
 *
 * The cases are `test/fixtures/task-prose-breaks.json`, which AgentKit's
 * `TaskProseBreaksTests` reads too, so the Mac, iOS and Android split a task's
 * text the same way.
 */
class TaskProseBreaksTest {
    @Test
    fun `the shared fixture's breaks`() {
        val root = Json.parseToJsonElement(repositoryFile("test/fixtures/task-prose-breaks.json")).jsonObject
        val cases = root["cases"]!!.jsonArray.map { it.jsonObject }
        assertTrue("the fixture has cases", cases.size >= 7)
        for (case in cases) {
            val name = case.string("case")
            val expected = case["blocks"]!!.jsonArray.map { block(it.jsonObject) }
            val blocks = Markdown.blocks(case.string("text"))
            assertEquals(name, expected, blocks)

            // Each paragraph's words as drawn: the newline survives the inline
            // pass too, so the Text breaks where the line did.
            val plain = case["plain"]!!.jsonArray.map { it.jsonPrimitive.content }
            val drawn = blocks.filterIsInstance<Markdown.Block.Paragraph>().map { Markdown.plain(it.text) }
            assertEquals(name, plain, drawn)
        }
    }

    private fun block(json: JsonObject): Markdown.Block {
        val text = json.string("text")
        return when (val kind = json.string("kind")) {
            "paragraph" -> Markdown.Block.Paragraph(text)
            "heading" -> Markdown.Block.Heading(json["level"]!!.jsonPrimitive.int, text)
            "bullet" -> Markdown.Block.Bullet(text, json["depth"]!!.jsonPrimitive.int)
            "numbered" ->
                Markdown.Block.Numbered(json.string("number"), text, json["depth"]!!.jsonPrimitive.int)
            "code" -> Markdown.Block.Code(text, json.string("language"))
            else -> throw AssertionError("unknown block kind $kind")
        }
    }

    private fun JsonObject.string(key: String) = this[key]!!.jsonPrimitive.content

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
