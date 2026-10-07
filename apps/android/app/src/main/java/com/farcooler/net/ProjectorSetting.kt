package com.farcooler.net

import com.farcooler.core.refusalWord
import com.farcooler.model.RunnerRefusal
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull

/**
 * Whether the runner's projector, and so the conversation view of its claude
 * panes, is on (ov-374): null from a runner too old to say. Read from `host`.
 */
class ProjectorSetting {
    private val _on = MutableStateFlow<Boolean?>(null)
    val on: StateFlow<Boolean?> = _on.asStateFlow()

    internal fun read(host: JsonObject) {
        _on.value = (host["projector"] as? JsonPrimitive)?.booleanOrNull
    }

    internal fun took(now: Boolean) {
        _on.value = now
    }
}

/**
 * Turn the runner's projector on or off (`settings.set_projector`), then
 * reconnect: a hello offers `agent_rows` only while it's on, and this link's
 * hello was made before. Null when it took; the sentence to show when it didn't.
 */
suspend fun Connection.setProjector(on: Boolean): String? {
    val failure = attempt { core.call("settings.set_projector", Connection.args("on" to on)) }.exceptionOrNull()
    if (failure != null) {
        return if (failure.refusalWord == RunnerRefusal.SCOPE_DENIED.word) {
            "This device can’t change this runner’s settings."
        } else {
            "That runner didn’t take the change. Try again in a moment."
        }
    }
    projector.took(on)
    reconnectNow()
    return null
}
