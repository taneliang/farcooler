package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.model.RunnerLink
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

/**
 * Who this phone is on a runner (ov-597), from `member.list`'s `you`: empty
 * for the owner, a member's handle, or `(released)` for a device that was a
 * member's and is nobody's now. What lets a pane say "Uses your Claude login"
 * rather than naming the viewer to themselves.
 *
 * One per [Connection] (ov-598), not one per pane: every pane on the runner
 * shares the answer, so opening a second pane asks nothing. It is read on
 * demand, once a pane that names a login is on screen, not on every connect: a
 * runner with only the owner's login never asks.
 *
 * Null until it has answered. A read that fails (a dropped link, a runner
 * busy restarting) is tried again after a growing wait, and only while the
 * connection is answering: a connection that is down makes no calls and waits
 * for it to come back. A runner that answers without a `you` (an older one)
 * has said all it will, so that is not retried. Until it answers the label
 * names the person, which never reads as "yours".
 *
 * The read runs in [scope], the connection's, not in the pane that asked, so it
 * ends with the connection and a pane leaving does not restart it at attempt
 * zero. [link] is the connection's: reads wait for [RunnerLink.ANSWERING], and
 * a retry after an outage goes out the moment the link is back rather than
 * after the rest of its wait. [waits] gives the wait before retry number
 * `attempt` (from zero), in milliseconds.
 */
class ViewerRead(
    private val call: suspend (String, JsonObject) -> JsonObject,
    private val link: StateFlow<RunnerLink> = MutableStateFlow(RunnerLink.ANSWERING),
    private val scope: CoroutineScope,
    private val waits: (attempt: Int) -> Long = { attempt -> minOf(2_000L shl minOf(attempt, 4), 30_000L) },
) {
    private val _you = MutableStateFlow<String?>(null)
    val you: StateFlow<String?> = _you.asStateFlow()

    private var job: Job? = null

    @Volatile
    private var settled = false

    /**
     * Start reading, unless it is known, settled or already under way. Called
     * from any pane that names a login; the first starts it and the rest return
     * at once. Returns the running read, for a test to wait on.
     */
    @Synchronized
    fun ask(): Job? {
        if (_you.value != null || settled || job?.isActive == true) return job
        return scope.launch { keepReading() }.also { job = it }
    }

    private suspend fun keepReading() {
        var attempt = 0
        while (_you.value == null && !settled) {
            link.first { it == RunnerLink.ANSWERING }
            if (readOnce()) return
            // Down now: wait for the link, then go at once. Up: back off.
            if (link.value == RunnerLink.ANSWERING) delay(waits(attempt++))
        }
    }

    /** One try; false when the call failed and is worth repeating. */
    private suspend fun readOnce(): Boolean {
        val reply = try {
            call("member.list", JsonObject(emptyMap()))
        } catch (_: CoreException) {
            return false
        }
        val you = (reply["you"] as? JsonPrimitive)?.contentOrNull
        if (you == null) settled = true else _you.value = you
        return true
    }
}
