package com.farcooler.data

import kotlinx.serialization.builtins.MapSerializer
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.json.Json

/**
 * The `runner_id` each paired runner last said, by this phone's own id for it
 * ([Runner.id]), kept beside the runner list (ov-231).
 *
 * A runner's id is learned only from its daemon build on connect, and the
 * pairing reply never carries one. Kept, a push naming a runner this phone
 * isn't connected to can still find it. A separate map rather than a field
 * on [Runner], so the runner list's custom serializer stays as it is. Pure
 * values in and out; [RunnerStore] holds the string.
 */
object RunnerIds {
    private val serializer = MapSerializer(String.serializer(), String.serializer())

    /** What was stored; empty for nothing, or for text that doesn't read. */
    fun decode(stored: String?): Map<String, String> =
        stored?.let { runCatching { Json.decodeFromString(serializer, it) }.getOrNull() } ?: emptyMap()

    fun encode(ids: Map<String, String>): String = Json.encodeToString(serializer, ids)

    /** [ids] with [runnerId] for [host], over any earlier one; unchanged for an id that is null or empty. */
    fun remember(ids: Map<String, String>, host: String, runnerId: String?): Map<String, String> =
        if (runnerId.isNullOrEmpty()) ids else ids + (host to runnerId)

    /** Whether an id outlives an edit: not when it re-points the runner or changes its user, which makes it another runner. */
    fun survivesEdit(reachChanged: Boolean, userChanged: Boolean): Boolean = !reachChanged && !userChanged

    fun forget(ids: Map<String, String>, host: String): Map<String, String> = ids - host
}
