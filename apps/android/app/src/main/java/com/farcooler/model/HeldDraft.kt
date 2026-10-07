package com.farcooler.model

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

/**
 * A draft the runner holds behind a dialog (ov-385), as a fleet row's
 * `draftHold` carries it: its id, and `waiting`, `sent`, `withdrawn` or
 * `expired`. A state this build doesn't know reads as ended, never waiting.
 * The iPhone and the Mac say the same sentences from AgentKit's `HeldDraft`.
 */
@Serializable
data class DraftHold(
    val id: String,
    val state: String = "expired",
    val expiresMs: Long = 0,
) {
    val isWaiting: Boolean get() = state == "waiting"

    companion object {
        /** The hold in `terminal.draft_prompt`'s answer, `{"held": …}`, or null when it was pasted. */
        fun held(answer: JsonObject): DraftHold? {
            val held = answer["held"] as? JsonObject ?: return null
            val id = (held["id"] ?: return null).jsonPrimitive.content.ifEmpty { return null }
            return DraftHold(
                id = id,
                state = held["state"]?.jsonPrimitive?.content ?: "expired",
                expiresMs = held["expiresMs"]?.jsonPrimitive?.longOrNull ?: 0,
            )
        }
    }
}

/** What the orchestrator's pane says about a draft held in it. */
object HeldDraft {
    enum class Status { WAITING, SENT, EXPIRED, FAILED, LOST, GONE }

    /**
     * [tracked], the hold this pane saw, as [current] says it is now. With none
     * on the terminal, [last] is the state the pane last saw it in: the runner
     * forgets a hold some minutes after it ends, so only one last seen waiting
     * was lost (a restart); one that had ended keeps saying how.
     */
    fun status(tracked: String, current: DraftHold?, last: String?): Status {
        if (current == null) {
            return when (last) {
                null -> Status.WAITING
                "waiting" -> Status.LOST
                else -> of(last)
            }
        }
        if (current.id != tracked) return Status.GONE
        return of(current.state)
    }

    private fun of(state: String): Status = when (state) {
        "waiting" -> Status.WAITING
        "sent" -> Status.SENT
        "withdrawn" -> Status.GONE
        "failed" -> Status.FAILED
        else -> Status.EXPIRED
    }

    /** What a pane remembers: the hold it saw waiting, and the state it last saw it in. */
    data class Watch(val tracked: String? = null, val last: String? = null) {
        fun observe(hold: DraftHold?): Watch = when {
            hold == null -> this
            hold.isWaiting -> Watch(hold.id, "waiting")
            hold.id != tracked -> this
            // Withdrawn has nothing to say: let it go.
            hold.state == "withdrawn" -> Watch()
            else -> copy(last = hold.state)
        }

        fun status(current: DraftHold?): Status? =
            tracked?.let { status(it, current, last) }?.takeIf { it != Status.GONE }
    }

    fun title(status: Status): String? = when (status) {
        Status.WAITING -> "Waiting for the dialog to close"
        Status.SENT -> "Sent"
        Status.EXPIRED, Status.FAILED, Status.LOST -> "Not sent"
        Status.GONE -> null
    }

    fun detail(status: Status): String? = when (status) {
        Status.WAITING -> "Your draft goes into the orchestrator’s box when the dialog in its pane closes."
        Status.SENT -> "Your draft is in the orchestrator’s box. Finish it there and press Return." // casing ok: the key is named Return on a keyboard
        Status.EXPIRED -> "It couldn’t go in within half an hour."
        Status.FAILED -> "Far Cooler couldn’t paste it. Try again."
        Status.LOST -> "The runner restarted before the dialog closed. Try again."
        Status.GONE -> null
    }

    const val WITHDRAW = "Withdraw"
}
