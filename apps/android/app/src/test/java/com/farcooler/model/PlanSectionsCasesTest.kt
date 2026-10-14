package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.util.Locale

/**
 * The Plan's sections, glance band, themes grid and sidebar lists (ov-503) against
 * `test/fixtures/plan-sections-cases.json`, which AgentKit's `PlanSectionsCasesTests` reads too. The inputs are
 * `plan --json` shapes as the runner sends them, decoded by the parser the app reads them with; the expected outlines
 * are lowercased, since Android says titles in sentence case and iOS in title case.
 */
class PlanSectionsCasesTest {
    private val en = Locale.US
    private val fixture = Json.parseToJsonElement(repositoryFile("test/fixtures/plan-sections-cases.json")).jsonObject

    private class Case(c: JsonObject) {
        val name = c["name"]!!.jsonPrimitive.content
        val plan = Plan.decode(c["plan"]!!.jsonObject)
        val needsYou = c["needs_you"]?.jsonPrimitive?.intOrNull ?: 0
        val agentsSupported = c["agents_supported"]?.jsonPrimitive?.booleanOrNull ?: true
        val statuses: Map<String, TaskStatus> = c["statuses"]!!.jsonObject.mapNotNull { (k, v) ->
            TaskStatus.parse(v.jsonPrimitive.content)?.let { k to it }
        }.toMap()
        val expect = c["expect"]?.jsonArray?.map { it.jsonPrimitive.content } ?: emptyList()
        val sections get() = PlanSections.of(plan, needsYou, statuses, agentsSupported = agentsSupported)
    }

    private fun kind(k: PlanSections.Kind) = k.name.lowercase().replace("_", "")

    /** One line a row, as the fixture writes them. */
    private fun outline(c: Case): List<String> {
        val s = c.sections
        val out = mutableListOf("shown: " + s.shown.joinToString(" ") { kind(it) })
        fun lane(label: String, row: PlanSections.LaneRow) {
            var line = "  $label ${row.lane.name} | ${PlanWords.status(row.lane)}"
            row.agent?.let { line += " | agent: ${it.who}" }
            if (row.waitsOnOwner) line += " | owner"
            row.landedTrain?.let { line += " | names landed $it" }
            out += line
        }
        if (s.needsYou > 0) out += "needs-you ${s.needsYou}"
        out += "landing"
        for (card in s.trains) {
            out += "  train ${card.train.label} | ${card.train.subtitle ?: "-"} | ${card.phase.sentence}"
            out += "    ladder: ${PlanLadder.compact(card.phase)} | ${card.phase.explanation}"
            for (l in card.lanes) {
                out += "    lane ${l.name} | ${l.note ?: "-"}" + (if (l.waitsOnOwner) " | owner" else "")
            }
        }
        s.emptyWords(PlanSections.Kind.LANDING)?.let { out += "  empty: $it" }
        if (s.inReview.isNotEmpty()) {
            out += "inreview"
            s.inReview.forEach { lane("lane", it) }
        }
        out += "building"
        s.building.forEach { lane("lane", it) }
        s.emptyWords(PlanSections.Kind.BUILDING)?.let { out += "  empty: $it" }
        out += "waiting"
        s.waiting.next.forEach { lane("next", it) }
        s.waiting.notRanked.forEach { lane("notranked", it) }
        s.waiting.onYou.forEach { lane("onyou", it) }
        s.emptyWords(PlanSections.Kind.WAITING)?.let { out += "  empty: $it" }
        if (PlanSections.Kind.ON_LANDED_TRAIN in s.shown) {
            out += "onlandedtrain (folded: ${PlanSections.Kind.ON_LANDED_TRAIN.startsFolded})"
            s.onLandedTrain.forEach { lane("lane", it) }
        }
        if (PlanSections.Kind.AGENTS in s.shown) {
            out += "agents"
            out += "  header: ${s.fleet.header(en)}"
            for (a in s.fleet.agents) {
                out += "  ${if (a.working) "working" else "idle"} ${a.who} | ${a.worksFor} | ${a.state(s.now)} | ${a.tokens(en) ?: "-"} | ${a.cost(en) ?: "-"}"
            }
        }
        out += "themes"
        for (t in s.themes.tiles) {
            out += "  tile ${t.name} | ${t.progress} | ${if (t.isMoving) "moving" else "still"} | ask: ${t.ownerAsk ?: "-"}"
            t.cards.forEach { out += "    card ${it.glyph.name.lowercase().replace("_", "")} ${it.key}" }
        }
        val title = s.themes.finishedTitle
        val words = s.themes.finishedWords
        if (title != null && words != null) out += "  $title | $words: " + s.themes.finished.joinToString(", ") { it.name }
        val glance = PlanGlanceSentence.of(s)
        out += "glance now: ${glance.nowLine}"
        out += "glance next: ${glance.nextLine}"
        out += "glance text: ${glance.text}"

        for ((word, filter) in listOf("open" to OneTree.Filter.OPEN, "inreview" to OneTree.Filter.IN_REVIEW, "all" to OneTree.Filter.ALL)) {
            out += "themetree $word"
            for (theme in ThemeTree.of(c.plan, filter, c.statuses).themes) {
                out += "  theme ${theme.name} | ${theme.progress}"
                for (card in theme.cards) {
                    out += "    card ${card.glyph.name.lowercase().replace("_", "")} ${card.key}" + (card.lane?.let { " | $it" } ?: "")
                }
            }
        }
        out += "lanetree"
        for (g in LaneTree.of(c.plan, s, en).groups) {
            out += "  group ${g.title} | ${g.detail ?: "-"} | ${if (g.folded) "folded" else "open"}"
            for (t in g.trains) {
                out += "    train ${t.label} | ${t.subtitle ?: "-"} | ${t.phase}"
                for (l in t.lanes) {
                    out += "      lane ${l.name} | ${l.state.ifEmpty { "-" }} | ${l.cards.joinToString(",") { it.key }}"
                    for (a in l.agents) out += "        agent ${a.who} | ${if (a.working) "working" else "done"} | ${a.tokens ?: "-"}"
                }
            }
            for (l in g.lanes) {
                out += "    lane ${l.name} | ${l.state} | ${l.cards.joinToString(",") { it.key }}"
                for (a in l.agents) out += "      agent ${a.who} | ${if (a.working) "working" else "done"} | ${a.tokens ?: "-"}"
            }
        }
        // Lowercased, and without trailing spaces, which a row with an empty last column leaves.
        return out.map { it.lowercase().trimEnd(' ') }
    }

    @Test
    fun `the sections, glance, grid and lists are the fixture's, case by case`() {
        val cases = fixture["cases"]!!.jsonArray
        assertTrue(cases.size >= 10)
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
