package com.farcooler.net

import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.JsonUnquotedLiteral

/** The JSON a call sends, built from pairs. [Connection.args] and [Connection.enrollArgs] are these. */
object CallArgs {
    /**
     * What one `client.enroll` sends.
     *
     * Split out so the one thing in it that can be wrong is testable
     * without a runner to write to, the way the Mac's
     * `Enrollment.arguments` is: `nodeKey` reaching the daemon is the whole
     * of what lets that runner's tunnel admit the new device later, and a
     * dropped field here fails ten seconds after somebody has already
     * scanned a code.
     *
     * `scope` is `control` and is not a parameter: `read` is a narrowing
     * done afterwards in Settings > Devices, and `host_admin` is a shell,
     * which no phone asks for. `shellAccess` is absent, which the client
     * core reads as false — the restricted line — and its absence being the
     * only shape available from here is the guard rail.
     *
     * The node key is sent even when empty, which the core reads as absent:
     * "this device asked for no tunnel". Not sending the key at all would
     * mean the same thing and say less.
     */
    fun enrollArgs(
        publicKey: String,
        label: String,
        clientId: String,
        nodeKey: String,
    ): JsonObject = args(
        "publicKey" to publicKey,
        "label" to label,
        "clientId" to clientId,
        "scope" to "control",
        "nodeKey" to nodeKey,
    )

    /**
     * A JSON object from string pairs, which is every call this makes.
     *
     * `ULong` is spelled out rather than falling through to `toString`,
     * because the difference is a quoted string versus a number and the
     * host reads these with `as_u64()` — which answers `None` for a string
     * and silently defaults. See [ScreenResponse.revision] for the value
     * that needs the full unsigned range.
     */
    @OptIn(ExperimentalSerializationApi::class)
    fun args(vararg pairs: Pair<String, Any>): JsonObject = JsonObject(
        pairs.associate { (key, value) ->
            key to when (value) {
                is Int -> JsonPrimitive(value)
                is Long -> JsonPrimitive(value)
                is Boolean -> JsonPrimitive(value)
                // Above `Long.MAX_VALUE` there is no `Number` that holds
                // the value, so it goes out as an unquoted literal — a JSON
                // number that Kotlin never had a type for.
                is ULong -> JsonUnquotedLiteral(value.toString())
                else -> JsonPrimitive(value.toString())
            }
        }
    )
}
