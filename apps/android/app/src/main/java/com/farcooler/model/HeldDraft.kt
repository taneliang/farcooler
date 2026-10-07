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
    enum class Status { WAITING, SENT, EXPIRED, LOST, GONE }

    /** [tracked], the hold this pane saw waiting, as [current] says it is now. */
    fun status(tracked: String, current: DraftHold?, seen: Boolean = true): Status {
        if (current == null) return if (seen) Status.LOST else Status.WAITING
        if (current.id != tracked) return Status.GONE
        return when (current.state) {
            "waiting" -> Status.WAITING
            "sent" -> Status.SENT
            "withdrawn" -> Status.GONE
            else -> Status.EXPIRED
        }
    }

    fun title(status: Status): String? = when (status) {
        Status.WAITING -> "Waiting for the dialog to close"
        Status.SENT -> "Sent"
        Status.EXPIRED, Status.LOST -> "Not sent"
        Status.GONE -> null
    }

    fun detail(status: Status): String? = when (status) {
        Status.WAITING -> "Your draft goes into the orchestrator’s box when the dialog in its pane closes."
        Status.SENT -> "Your draft is in the orchestrator’s box. Finish it there and press Return." // casing ok: the key is named Return on a keyboard
        Status.EXPIRED -> "The dialog stayed open for half an hour. Answer it, then try again."
        Status.LOST -> "The runner restarted before the dialog closed. Try again."
        Status.GONE -> null
    }

    const val WITHDRAW = "Withdraw"
}
