package com.farcooler.net

import com.farcooler.core.CoreException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive

/**
 * Who this phone is on a runner (ov-597), from `member.list`'s `you`: empty
 * for the owner, a member's handle, or `(released)` for a device that was a
 * member's and is nobody's now. What lets a pane say "Uses your Claude login"
 * rather than naming the viewer to themselves.
 *
 * Read on demand, once a pane that names a login is on screen, not on every
 * connect: a runner with only the owner's login never asks. Null until it has
 * answered, and when it can't (an older runner, a dropped link), which reads
 * as "name the person" and never as "yours".
 */
class ViewerRead(private val call: suspend (String, JsonObject) -> JsonObject) {
    private val _you = MutableStateFlow<String?>(null)
    val you: StateFlow<String?> = _you.asStateFlow()

    /** Ask the runner, unless it already has said. */
    suspend fun read() {
        if (_you.value != null) return
        val reply = try {
            call("member.list", JsonObject(emptyMap()))
        } catch (_: CoreException) {
            return
        }
        (reply["you"] as? JsonPrimitive)?.contentOrNull?.let { _you.value = it }
    }
}
