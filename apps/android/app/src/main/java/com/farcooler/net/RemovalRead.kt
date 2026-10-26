package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.model.RunnerTeam
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.json.JsonObject

/**
 * Whether the runner removed this phone's member from its team (ov-784). A removed device is refused every read
 * `not_yours` but `member.list`, which answers `(released)` (ov-758), so a fleet read refused that way asks it. What
 * puts "You were removed from this runner" on the runner's row, in place of a runner that answers nothing.
 *
 * Asked again at most every [STALE_AFTER_MS] while removed, so being added back is noticed without a call on every
 * poll; [checkAgain] asks on the next refusal. [now] is the clock, for a test.
 */
class RemovalRead(
    private val call: suspend (String, JsonObject) -> JsonObject,
    private val now: () -> Long = System::currentTimeMillis,
) {
    private val _removed = MutableStateFlow(false)
    val removed: StateFlow<Boolean> = _removed.asStateFlow()
    private var lastRead: Long? = null

    /** After a fleet read failed with [error]: whether it's because this phone's member was removed. */
    suspend fun notice(error: Throwable): Boolean {
        if ((error as? CoreException)?.what != "not_yours") return false
        val last = lastRead
        if (_removed.value && last != null && now() - last < STALE_AFTER_MS) return true
        val reply = try {
            call("member.list", JsonObject(emptyMap()))
        } catch (e: CoreException) {
            return _removed.value
        }
        lastRead = now()
        _removed.value = RunnerTeam.of(reply)?.isReleased == true
        return _removed.value
    }

    /** A fleet read landed: whoever this phone is, it isn't removed. */
    fun landed() {
        _removed.value = false
    }

    /** Check again, on the removed runner's row: the next refusal asks at once. */
    fun checkAgain() {
        lastRead = null
    }

    companion object {
        const val STALE_AFTER_MS = 30_000L
    }
}
