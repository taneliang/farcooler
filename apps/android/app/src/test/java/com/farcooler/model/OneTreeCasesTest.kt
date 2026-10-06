package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * The cases both phones' trees and strips must agree on (ov-300 review 7):
 * `test/fixtures/one-tree-cases.json`, which AgentKit's `PhoneTreeCasesTests`
 * reads too. Inputs are in the wire's own shapes, decoded by the parsers the
 * app reads them with; the expected outline and strip are lowercased, since
 * Android says titles in sentence case and iOS in title case.
 */
class OneTreeCasesTest {
    private val json = Json { ignoreUnknownKeys = true }
    private val fixture = Json.parseToJsonElement(repositoryFile("test/fixtures/one-tree-cases.json")).jsonObject

    private fun outline(nodes: List<OneTree.Node>, depth: Int = 0): List<String> = nodes.flatMap { node ->
        val line = buildString {
            append("  ".repeat(depth))
            append(node.kind.name.lowercase().replace("_", "")).append(' ')
            if (node.key.isNotEmpty()) append(node.key).append(' ')
            append(node.title)
            listOf(node.detail, node.caption, node.also).filter { it.isNotEmpty() }.forEach { append(" · ").append(it) }
            if (node.showsDot) append(" •")
        }
        listOf(line.lowercase()) + outline(node.children, depth + 1)
    }

    private fun filter(word: String?) = when (word) {
        "in_review" -> OneTree.Filter.IN_REVIEW
        "all" -> OneTree.Filter.ALL
        else -> OneTree.Filter.OPEN
    }

    @Test
    fun `the tree's outline is the fixture's, case by case`() {
        val cases = fixture["cases"]!!.jsonArray
        assertTrue(cases.isNotEmpty())
        for (element in cases) {
            val c = element.jsonObject
            val ws = c["workspace"]!!.jsonObject
            val workspace = WorkspaceSummary(id = ws.str("id"), name = ws.str("name"), repository = ws["repository"]?.jsonPrimitive?.contentOrNull)
            val tree = OneTree.build(
                workspace,
                TaskBoard.decode(c["board"]!!.jsonObject),
                Plan.decode(c["plan"]!!.jsonObject),
                json.decodeFromJsonElement(kotlinx.serialization.builtins.ListSerializer(Worktree.serializer()), c["worktrees"]!!),
                json.decodeFromJsonElement(kotlinx.serialization.builtins.ListSerializer(NeedsYouItem.serializer()), c["items"]!!),
                filter(c["filter"]?.jsonPrimitive?.contentOrNull),
                BoardPage.list(c["pages"]!!.jsonObject),
            )
            val expected = c["outline"]!!.jsonArray.map { it.jsonPrimitive.content }
            val actual = outline(tree.work + tree.below)
            assertEquals("${c.str("name")}:\n${actual.joinToString("\n")}", expected, actual)
        }
    }

    @Test
    fun `the strip's parts, state, line and spoken label are the fixture's`() {
        for (element in fixture["strips"]!!.jsonArray) {
            val s = element.jsonObject
            val orchestrator = (s["orchestrator"] as? JsonObject)?.let { json.decodeFromJsonElement(Terminal.serializer(), it) }
            val strip = PlanStrip.of(Plan.decode(s["plan"]!!.jsonObject), s["needs_you"]!!.jsonPrimitive.int, orchestrator)
            val name = s.str("name")
            assertEquals(name, s["parts"]!!.jsonArray.map { it.jsonPrimitive.content }, strip.parts.map { it.lowercase() })
            assertEquals(name, s.str("state"), strip.orchestrator.word.lowercase())
            assertEquals(name, (s["line"] as? JsonNull)?.let { null } ?: s["line"]?.jsonPrimitive?.contentOrNull, strip.line)
            assertEquals(name, s.str("label"), strip.accessibilityLabel.lowercase())
        }
    }

    private fun JsonObject.str(name: String) = this[name]?.jsonPrimitive?.contentOrNull ?: ""

    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}.")
    }
}
