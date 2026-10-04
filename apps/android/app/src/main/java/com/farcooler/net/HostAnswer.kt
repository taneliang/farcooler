package com.farcooler.net

import com.farcooler.model.DaemonBuild
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonPrimitive

/**
 * What the runner's `host` answer says it is: [Connection.loadDaemonBuild]
 * reads it once per link. Out of `Connection.kt`, which is past its size
 * budget, and a plain function a JVM test can feed a `host` body.
 */
internal fun hostDaemonBuild(body: JsonObject): DaemonBuild =
    DaemonBuild(
        version = body["daemonVersion"]?.jsonPrimitive?.contentOrNull ?: "unknown",
        matches = body["buildsMatch"]?.jsonPrimitive?.booleanOrNull ?: true,
        platform = body["platform"]?.jsonPrimitive?.contentOrNull.orEmpty(),
        // Absent from a daemon older than capabilities. `DaemonBuild.can`
        // reads an empty set as the features that existed then, so an old
        // runner keeps working rather than going dark.
        capabilities = body["capabilities"]?.jsonArray
            ?.mapNotNull { it.jsonPrimitive.contentOrNull }
            ?.toSet()
            .orEmpty(),
        // What THIS session may ask for, computed by the daemon from the
        // real grant and carried in the handshake, so it costs no round
        // trip on top of the one this call already makes.
        //
        // Absent — and "unspecified" — both mean "no answer", never "no
        // permission". `DaemonBuild.mayAdministerRunner` reads either as
        // "keep offering what we offer today", so a runner newer than this
        // build cannot silently strip controls off it.
        grantedScope = body["grantedScope"]?.jsonPrimitive?.contentOrNull ?: "unspecified",
        runnerId = body["runnerId"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotEmpty() },
        // Whether its task notices reach this phone as pushes (ov-107).
        pushPaired = body["pushPaired"]?.jsonPrimitive?.booleanOrNull ?: false,
        // Which of claude, codex and cursor-agent it found (ov-205). Null
        // from a runner too old to say, which offers every harness.
        agentsFound = (body["agentsFound"] as? kotlinx.serialization.json.JsonArray)
            ?.mapNotNull { it.jsonPrimitive.contentOrNull },
        // The runner's extra read-only folders, by name (ov-259). Null from a
        // runner too old to have them.
        readOnlyFolders = (body["readOnlyFolders"] as? kotlinx.serialization.json.JsonArray)
            ?.mapNotNull { it.jsonPrimitive.contentOrNull },
    )
