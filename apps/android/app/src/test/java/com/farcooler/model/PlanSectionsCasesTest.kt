package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * The Plan's glance band and sidebar lists (ov-503) against the second half of
 * `test/fixtures/plan-sections-cases.json`, which AgentKit's `PlanSectionsCasesTests` reads too (its first half, the sections,
 * is the Rust view model's: `crates/client/src/plan_view/tests.rs`). Needs the host JNI core: `cargo build -p farcooler-jni`
 * and `-Pfarcooler.nativeLibDir=<repo>/target/debug`. The inputs are
 * `plan --json` shapes as the runner sends them, decoded by the parser the app reads them with; the expected outlines
 * are lowercased, since Android says titles in sentence case and iOS in title case.
 */
class PlanSectionsCasesTest {
    private val fixture = Json.parseToJsonElement(repositoryFile("test/fixtures/plan-sections-cases.json")).jsonObject

    private class Case(c: JsonObject) {
        val name = c["name"]!!.jsonPrimitive.content
        val plan = Plan.decode(c["plan"]!!.jsonObject)
        val needsYou = c["needs_you"]?.jsonPrimitive?.intOrNull ?: 0
        val statuses: Map<String, TaskStatus> = c["statuses"]!!.jsonObject.mapNotNull { (k, v) ->
            TaskStatus.parse(v.jsonPrimitive.content)?.let { k to it }
        }.toMap()
        val expect = c["expect"]?.jsonArray?.map { it.jsonPrimitive.content }?.dropWhile { !it.startsWith("glance now") } ?: emptyList()
    }

    /**
     * One line a row, as the fixture writes them, from "glance now" on: the glance band, the themes sidebar and the tickets
     * sidebar, which are still Kotlin (ov-1016 phase 1) but read the shared core's view, asked for here over the host
     * build of the JNI core. Every line before it is the core's own, pinned by its Rust test over the same fixture.
     */
    private fun outline(c: Case): List<String> {
        val view = PlanViews.of(c.plan, c.needsYou, c.statuses) ?: throw AssertionError("${c.name}: the core gave no view")
        val out = mutableListOf<String>()
        val glance = PlanGlanceSentence.of(view)
        out += "glance now: ${glance.nowLine}"
        out += "glance next: ${glance.nextLine}"
        out += "glance text: ${glance.text}"

        for ((word, filter) in listOf("open" to OneTree.Filter.OPEN, "inreview" to OneTree.Filter.IN_REVIEW, "all" to OneTree.Filter.ALL)) {
            out += "themetree $word"
            for (theme in ThemeTree.of(c.plan, filter, c.statuses).themes) {
                out += "  theme ${theme.name} | ${theme.progress}"
                for (card in theme.cards) {
                    out += "    card ${card.glyph.name.lowercase().replace("_", "")} ${card.key}" + (card.ticket?.let { " | $it" } ?: "")
                }
            }
        }
        out += "tickettree"
        for (g in TicketTree.of(c.plan, view).groups) {
            out += "  group ${g.title} | ${g.detail ?: "-"} | ${if (g.folded) "folded" else "open"}"
            for (l in g.landings) {
                out += "    landing ${l.label} | ${l.subtitle ?: "-"} | ${l.phase}"
                for (t in l.tickets) {
                    out += "      ticket ${t.key} | ${t.state.ifEmpty { "-" }}"
                    for (a in t.agents) out += "        agent ${a.who} | ${if (a.working) "working" else "done"} | ${a.tokens ?: "-"}"
                }
            }
            for (t in g.tickets) {
                out += "    ticket ${t.key} | ${t.state}"
                for (a in t.agents) out += "      agent ${a.who} | ${if (a.working) "working" else "done"} | ${a.tokens ?: "-"}"
            }
        }
        // Lowercased, and without trailing spaces, which a row with an empty last column leaves.
        return out.map { it.lowercase().trimEnd(' ') }
    }

    @Test
    fun `the glance, themes sidebar and tickets sidebar are the fixture's, case by case`() {
        val cases = fixture["cases"]!!.jsonArray
        assertTrue(cases.size >= 10)
        // Setting FARCOOLER_REGENERATE_CASES to a path writes the outlines this build makes there, for a person to read and
        // copy into the fixture's `expect`; the test then says it did, and never passes on the strength of its own output.
        System.getenv("FARCOOLER_REGENERATE_CASES")?.let { path ->
            val made = cases.joinToString(",\n") { element ->
                val c = Case(element.jsonObject)
                "\"${c.name}\": ${Json.encodeToString(kotlinx.serialization.builtins.ListSerializer(kotlinx.serialization.serializer<String>()), outline(c))}"
            }
            File(path).writeText("{\n$made\n}\n")
            throw AssertionError("Wrote the outlines to $path; copy them into the fixture and run again without FARCOOLER_REGENERATE_CASES.")
        }
        for (element in cases) {
            val c = Case(element.jsonObject)
            assertTrue("${c.name} has an expected outline", c.expect.isNotEmpty())
            assertEquals(c.name, c.expect, outline(c))
        }
    }

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
