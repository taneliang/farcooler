package com.farcooler.model

import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What Needs You shows across runners, and what it may claim when it shows
 * nothing, from the case table AgentKit's `PhoneInboxShownTests` reads too
 * (`test/fixtures/needs-you-shown.json`).
 *
 * The case it exists for: a connected runner whose `needs_you` read failed.
 * It used to contribute nothing and go unnamed, so the screen could say
 * "Nothing needs you" over an agent blocked on it (ov-102 phones finding 10).
 */
class NeedsYouShownTest {
    /** The same configuration `Connection` decodes a fleet with. */
    private val json = Json { ignoreUnknownKeys = true }

    private data class Case(
        val name: String,
        val runners: List<NeedsYouRunner>,
        val shown: List<String>,
        val derived: List<String>,
        val caveat: String?,
        val empty: String?,
    )

    private fun cases(): List<Case> {
        val root = json.parseToJsonElement(repositoryFile("test/fixtures/needs-you-shown.json")).jsonObject
        return root.getValue("cases").jsonArray.map { element ->
            val case = element.jsonObject
            Case(
                name = case.getValue("name").jsonPrimitive.content,
                runners = case.getValue("runners").jsonArray.map { runner(it.jsonObject) },
                shown = case.getValue("shown").jsonArray.map { it.jsonPrimitive.content },
                derived = case.getValue("derived").jsonArray.map { it.jsonPrimitive.content },
                caveat = case.getValue("caveat").jsonPrimitive.contentOrNull,
                empty = case.getValue("empty").jsonPrimitive.contentOrNull,
            )
        }
    }

    private fun runner(o: JsonObject): NeedsYouRunner {
        val name = o.getValue("runner").jsonPrimitive.content
        val list = o.getValue("needs_you").takeUnless { it is JsonNull }
        val fleet = o.getValue("fleet").takeUnless { it is JsonNull }
        return NeedsYouRunner(
            hostId = name,
            label = name,
            reading = list?.let { RunnerNeedsYou(json.decodeFromJsonElement(NeedsYouList.serializer(), it).items) },
            fleet = fleet?.let { json.decodeFromJsonElement(Fleet.serializer(), it) } ?: Fleet.EMPTY,
            answering = o.getValue("answering").jsonPrimitive.boolean,
        )
    }

    @Test
    fun `every case in the shared table holds`() {
        for (case in cases()) {
            val rows = NeedsYou.rows(case.runners)
            assertEquals(case.name, case.shown, rows.map { it.item.id })
            assertEquals(case.name, case.derived, rows.filter { it.item.isDerived }.map { it.item.id })
            assertEquals(case.name, case.caveat, NeedsYou.caveat(NeedsYou.unanswered(case.runners)))
            val empty = when {
                rows.isNotEmpty() -> null
                NeedsYou.nothingNeedsYou(case.runners, rows) -> "nothing"
                else -> "checking"
            }
            assertEquals(case.name, case.empty, empty)
            // A runner whose list wasn't read is never an older runner to update.
            assertEquals(case.name, emptyList<String>(), NeedsYou.olderRunners(case.runners))
        }
    }

    /** The table can't pass by being empty, or by leaving out the case it is for. */
    @Test
    fun `the table holds a failed read with a blocked agent`() {
        val cases = cases()
        assertTrue(cases.size >= 5)
        assertTrue(
            cases.any { case ->
                case.derived.isNotEmpty() && case.runners.any { it.reading == null && it.answering }
            }
        )
        assertTrue(cases.any { it.empty == "checking" })
        assertTrue(cases.any { it.empty == "nothing" && it.caveat != null })
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
