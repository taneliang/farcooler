package com.farcooler.model

import com.farcooler.data.Theme
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject

/** What a handshake said. */
sealed interface AdapterTestOutcome {
    data class Worked(val reported: String) : AdapterTestOutcome

    data class Failed(val why: String) : AdapterTestOutcome
}

/**
 * One `[adapters.<name>]` table, plus where it came from.
 *
 * Mirrors the Mac's and iOS's own `AdapterInfo`. Carried whole rather than as a
 * diff against the built-in, because the editor has to be able to show what an
 * override actually says — including a deliberately emptied array, which a diff
 * could not tell apart from "unset".
 */
@Serializable
data class AdapterInfo(
    val preset: String,
    val program: String = "",
    val args: List<String> = emptyList(),
    val env: Map<String, String> = emptyMap(),
    /**
     * Detection, as opposed to launch. A wrong value here does not fail loudly:
     * it stops the agent being recognized, which surfaces later as "chat mode
     * broke" from somewhere else. `adapter.test` cannot check these.
     */
    val commands: List<String> = emptyList(),
    val identity: List<String> = emptyList(),
    val blocked: List<String> = emptyList(),
    val working: List<String> = emptyList(),
    val origin: String = "user",
) {
    /**
     * Whether Far Cooler can host this agent as a chat at all.
     *
     * An adapter with no program is a recognized agent that stays a terminal,
     * which is a real supported state rather than a gap.
     */
    val chatCapable: Boolean get() = program.isNotBlank()

    /** How this reads under its name in a list. */
    fun subtitle(): String {
        val where = when (origin) {
            "builtIn" -> "Shipped"
            "override" -> "Overriding a shipped agent"
            "user" -> "Yours"
            else -> "Unknown"
        }
        if (!chatCapable) return "$where — terminal only"
        return "$where — ${(listOf(program) + args).joinToString(" ")}"
    }

    /**
     * The bridge's arguments for `adapter.upsert` and `adapter.test`.
     *
     * Built by hand rather than serialized from this class, because `origin` is
     * the daemon's to decide on the way back out — a client claiming one would
     * be claiming something only the daemon can know.
     */
    fun toJson(): JsonObject = buildJsonObject {
        put("preset", preset)
        put("program", program)
        putJsonArray("args") { args.forEach { add(JsonPrimitive(it)) } }
        putJsonObject("env") { env.forEach { (key, value) -> put(key, value) } }
        putJsonArray("commands") { commands.forEach { add(JsonPrimitive(it)) } }
        putJsonArray("identity") { identity.forEach { add(JsonPrimitive(it)) } }
        putJsonArray("blocked") { blocked.forEach { add(JsonPrimitive(it)) } }
        putJsonArray("working") { working.forEach { add(JsonPrimitive(it)) } }
    }
}

/** The bridge's arguments for `theme.upsert`. */
fun Theme.toJson(): JsonObject = buildJsonObject {
    put("name", name)
    put("dark", dark)
    put("background", background)
    put("foreground", foreground)
    put("cursor", cursor)
    putJsonArray("ansi") { ansi.forEach { add(JsonPrimitive(it)) } }
}

/**
 * One entry per line, blank lines dropped.
 *
 * Deliberately not trimmed further: a detection string can legitimately begin or
 * end with a space — several built-ins do, because that is what the agent
 * actually draws — and trimming would break exactly the ones that matter.
 */
fun linesToList(text: String): List<String> =
    text.split("\n").filter { it.isNotBlank() }

/**
 * `KEY=value` per line, splitting on the FIRST `=` only.
 *
 * A value can contain them, and a token or a URL routinely does.
 */
fun linesToEnv(text: String): Map<String, String> =
    text.split("\n").mapNotNull { line ->
        val at = line.indexOf('=')
        if (at <= 0) return@mapNotNull null
        val key = line.substring(0, at).trim()
        if (key.isEmpty()) null else key to line.substring(at + 1)
    }.toMap()

fun envToLines(env: Map<String, String>): String =
    env.toSortedMap().entries.joinToString("\n") { "${it.key}=${it.value}" }
