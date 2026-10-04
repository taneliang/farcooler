package com.farcooler.model

import java.io.File
import java.util.Locale
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What a card says about who is on it and when it starts, read from the ONE
 * fixture AgentKit's `TaskStartLinesTests` reads too: a row as `task list
 * --json` sends it, plus the panes and the clock, in; the control's words, the
 * quiet line and the Waiting on sentence, out. Whole strings, so the two
 * phones cannot drift by a word. The file is the iPhone's test resource, read
 * in place rather than copied: a copy is a second file that can disagree.
 */
class TaskStartLinesTest {
    private class Case(val name: String, val panes: Int, val records: Boolean, val task: JsonObject, val expect: JsonObject)

    private val fixture: JsonObject = run {
        // Gradle runs unit tests from the module (apps/android/app).
        val file = listOf("../../shared/AgentKit/Tests/AgentKitTests/task_start_lines.json", "apps/shared/AgentKit/Tests/AgentKitTests/task_start_lines.json")
            .map(::File).firstOrNull { it.exists() }
            ?: error("task_start_lines.json is not where AgentKit keeps it")
        Json.parseToJsonElement(file.readText()).jsonObject
    }

    private val now: Long = fixture["now"]!!.jsonPrimitive.long()

    private fun kotlinx.serialization.json.JsonPrimitive.long(): Long = longOrNull!!

    private val cases: List<Case> = fixture["cases"]!!.jsonArray.map {
        val o = it.jsonObject
        Case(
            o["name"]!!.jsonPrimitive.content,
            o["panes"]!!.jsonPrimitive.intOrNull!!,
            o["records_tasks"]!!.jsonPrimitive.booleanOrNull!!,
            o["task"]!!.jsonObject,
            o["expect"]!!.jsonObject,
        )
    }

    private fun row(case: Case): TaskRow =
        TaskBoard.decode(JsonObject(mapOf("tasks" to JsonArray(listOf(case.task))))).rows.single()

    private fun panes(case: Case, row: TaskRow): List<Terminal> =
        List(case.panes) { Terminal(id = "p$it", preset = "claude", state = "running", paneMode = null, taskId = row.id) }

    private fun expected(case: Case, name: String): String? =
        case.expect[name]?.takeIf { it !is kotlinx.serialization.json.JsonNull }?.jsonPrimitive?.contentOrNull

    @Test
    fun theFixtureHasCasesToCheck() {
        assertTrue(cases.size > 30)
    }

    @Test
    fun theControlSaysTheSameWordsOnEveryCase() {
        val wrong = cases.mapNotNull { case ->
            val r = row(case)
            val got = r.agentPresence(r.livePanes(panes(case, r)).size, case.records).title
            val want = (case.expect["chip"] as? JsonObject)?.get("android")?.jsonPrimitive?.contentOrNull
            if (got == want) null else "${case.name}: $got != $want"
        }
        assertEquals(emptyList<String>(), wrong)
    }

    @Test
    fun theBlocksSayTheSameSentenceOnEveryCase() {
        val wrong = cases.mapNotNull { case ->
            val got = row(case).blockedSummary
            val want = expected(case, "blocked")
            if (got == want) null else "${case.name}: $got != $want"
        }
        assertEquals(emptyList<String>(), wrong)
    }

    @Test
    fun theStartLineSaysTheSameSentenceOnEveryCase() {
        val wrong = cases.mapNotNull { case ->
            val got = row(case).startLine(now, case.records, java.time.ZoneId.of("UTC"), Locale.US)
            val want = expected(case, "line")
            if (got == want) null else "${case.name}: $got != $want"
        }
        assertEquals(emptyList<String>(), wrong)
    }
}
