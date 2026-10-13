package com.farcooler.capture

import com.farcooler.model.Plan
import com.farcooler.model.TaskStatus
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/** A plan as the runner sends it, and what surrounds it on the board: the Needs You count and the cards' statuses. */
class PlanFixture(val plan: Plan, val needsYou: Int, val statuses: Map<String, TaskStatus>)

/**
 * The plans the Android views are drawn and tested from (ov-503): `test/fixtures/plan-sections-cases.json`'s cases,
 * which AgentKit reads too, and `plan-android-long.json`, the same board with long real titles. Decoded by the parser
 * the app reads them with.
 */
object PlanFixtures {
    private fun parse(relative: String): JsonObject = Json.parseToJsonElement(repositoryFile(relative)).jsonObject

    private fun fixture(o: JsonObject) = PlanFixture(
        Plan.decode(o["plan"]!!.jsonObject),
        o["needs_you"]?.jsonPrimitive?.intOrNull ?: 0,
        (o["statuses"] as? JsonObject).orEmpty().mapNotNull { (k, v) -> TaskStatus.parse(v.jsonPrimitive.content)?.let { k to it } }.toMap(),
    )

    private fun JsonObject?.orEmpty(): JsonObject = this ?: JsonObject(emptyMap())

    /** One of the shared cases, by name. */
    fun case(name: String): PlanFixture =
        fixture(parse("test/fixtures/plan-sections-cases.json")["cases"]!!.jsonArray.map { it.jsonObject }.first { it["name"]!!.jsonPrimitive.content == name })

    /** The board with long real titles, a train waiting for CI and ranked lanes. */
    fun long(): PlanFixture = fixture(parse("test/fixtures/plan-android-long.json"))

    fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}.")
    }
}
