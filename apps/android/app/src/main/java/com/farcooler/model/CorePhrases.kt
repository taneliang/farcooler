package com.farcooler.model

import com.farcooler.core.NativeClient
import com.farcooler.core.coreIsAvailable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * The phrases the shell's close question and the Agents card share with the plan, asked of the shared core
 * (`crates/client/src/plan_view/words.rs`, ov-1041, ruling R-112) through the bridge the Plan already uses. Sentence
 * case, as Material says it. Null where the core isn't in this build: a screen then leaves the phrase out rather than
 * write it again.
 */
object CorePhrases {
    private val core: (String) -> String? = { request -> if (coreIsAvailable) NativeClient.nativePlanView(request) else null }

    /** The core's bridge; a test can stand its own in, and [reset] puts the core back. */
    internal var bridge: (String) -> String? = core

    private var asked: JsonObject? = null

    internal fun reset() {
        bridge = core
        asked = null
    }

    @Synchronized
    private fun words(): JsonObject? {
        asked?.let { return it }
        val answer = bridge("""{"schema":1,"words":{"style":"material"}}""") ?: return null
        val words = try {
            Json.parseToJsonElement(answer).jsonObject["words"] as? JsonObject
        } catch (_: IllegalArgumentException) {
            null
        }
        asked = words
        return words
    }

    private fun word(key: String): String? = words()?.get(key)?.jsonPrimitive?.content

    /** The Agents card's title when nothing is running. */
    val agentsNone: String? get() = word("agents_none")

    /** What a blocked agent is doing, in the middle of a sentence. */
    val blockedDoing: String? get() = word("blocked_doing")
}
