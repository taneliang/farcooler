package com.farcooler.model

import com.farcooler.data.Theme
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray

/** The bridge's arguments for `theme.upsert`. */
fun Theme.toJson(): JsonObject = buildJsonObject {
    put("name", name)
    put("dark", dark)
    put("background", background)
    put("foreground", foreground)
    put("cursor", cursor)
    putJsonArray("ansi") { ansi.forEach { add(JsonPrimitive(it)) } }
}
