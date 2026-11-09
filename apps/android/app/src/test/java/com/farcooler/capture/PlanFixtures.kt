package com.farcooler.capture

import com.farcooler.model.Plan
import com.farcooler.model.TaskStatus
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long

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

    /** The board with long real titles, a landing waiting for CI and ranked tickets. */
    fun long(): PlanFixture = fixture(parse("test/fixtures/plan-android-long.json"))

    /**
     * [long] with one ticket that landed an hour before the plan's clock, under a long real title: what Landed today
     * draws, which no other fixture has (ov-1033).
     */
    fun longWithLandedToday(alone: Boolean = false): PlanFixture {
        val o = parse("test/fixtures/plan-android-long.json")
        val plan = o["plan"]!!.jsonObject
        val now = plan["now_ms"]!!.jsonPrimitive.long
        val landed = JsonObject(
            plan["lines"]!!.jsonArray.first().jsonObject + mapOf(
                "task" to JsonPrimitive("t-ov-300"),
                "key" to JsonPrimitive("ov-300"),
                "title" to JsonPrimitive("Mac: a resized window keeps the row the reader was on, whatever the transcript did meanwhile"),
                "stage" to JsonPrimitive("landed"),
                "stage_since" to JsonPrimitive(now - 3_600_000),
                "landed_sha" to JsonPrimitive("ac840108f2"),
                "landing" to JsonNull,
                "branch" to JsonPrimitive("landed-resize"),
                "worktree_path" to JsonNull,
            ),
        )
        if (alone) {
            // Just that ticket under the board's clock: the section at the top of the picture.
            val bare = plan + mapOf("lines" to JsonArray(listOf(landed)), "landings" to JsonArray(emptyList()), "themes" to JsonArray(emptyList()), "cards" to JsonArray(emptyList()), "agents" to JsonArray(emptyList()))
            return fixture(JsonObject(o + ("plan" to JsonObject(bare))))
        }
        val lines = JsonArray(plan["lines"]!!.jsonArray + landed)
        return fixture(JsonObject(o + ("plan" to JsonObject(plan + ("lines" to lines)))))
    }

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
