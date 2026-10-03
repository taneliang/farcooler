package com.farcooler.net

import com.farcooler.core.refusalWord
import com.farcooler.model.PermissionAnswering
import com.farcooler.model.troubleFor
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.json.JsonObject

/**
 * The two things a pane sends that must never fail silently: an answer to a
 * permission ask, and a prompt.
 *
 * Both went out through `attempt`, which threw the error away. An answer the
 * runner refused took its card down anyway, so the agent stayed blocked on an
 * ask the phone could no longer show. A prompt that never left drew as sent,
 * with an agent that never replied. iOS fixed both: AgentKit's
 * `PermissionAnswering`, and `SendFailure` in `apps/ios/FarCooler/AgentStream.swift`.
 *
 * Fed the call as a closure, so it is testable without a JNI client core.
 */
class AgentSending(private val call: suspend (method: String, args: JsonObject) -> Unit) {
    private val _answering = MutableStateFlow(PermissionAnswering())

    /** The answer to a pending ask, while it is out and after it failed. */
    val answering: StateFlow<PermissionAnswering> = _answering.asStateFlow()

    private val _sendFailure = MutableStateFlow<SendFailure?>(null)

    /**
     * The prompt that did not go, if one didn't.
     *
     * Not folded into the pane's phase: that is cleared by every good poll,
     * which would take the warning down a second after it appeared while the
     * undelivered message stayed on screen looking sent.
     */
    val sendFailure: StateFlow<SendFailure?> = _sendFailure.asStateFlow()

    /** A prompt that did not go: what to say, and what to send again. */
    data class SendFailure(val message: String, val args: JsonObject)

    /**
     * Send one answer. Returns whether the card for [requestId] comes down.
     *
     * False, and nothing sent, while another answer is out.
     */
    suspend fun answer(requestId: String, args: JsonObject): Boolean {
        _answering.value = _answering.value.begin(requestId) ?: return false
        // Until the call says otherwise. A cancelled call keeps this, so the
        // `finally` turns the buttons back on with the "may not have reached"
        // sentence, and the cancellation goes on.
        var outcome = PermissionAnswering.outcome(refusedWith = null)
        var down = false
        try {
            call("terminal.agent_answer", args)
            outcome = PermissionAnswering.Outcome.Sent
        } catch (e: Exception) {
            e.rethrowIfCancellation()
            outcome = PermissionAnswering.outcome(refusedWith = e.refusalWord)
        } finally {
            val (next, comesDown) = _answering.value.finish(requestId, outcome)
            _answering.value = next
            down = comesDown
        }
        return down
    }

    /** Send one prompt. Returns whether it went. */
    suspend fun prompt(args: JsonObject): Boolean {
        try {
            call("terminal.agent_prompt", args)
        } catch (e: Exception) {
            e.rethrowIfCancellation()
            _sendFailure.value = SendFailure(messageFor(e), args)
            return false
        }
        _sendFailure.value = null
        return true
    }

    /**
     * Send the failed prompt again.
     *
     * Only the call: the words are already on screen from the first try, and
     * drawing them twice would read as two messages.
     */
    suspend fun retry(): Boolean {
        val failed = _sendFailure.value ?: return false
        return prompt(failed.args)
    }

    /** Put the failure away without sending. */
    fun dismissSendFailure() {
        _sendFailure.value = null
    }

    companion object {
        /** The core's failure, as something worth putting on a phone screen. */
        fun messageFor(error: Throwable): String {
            // The size ceiling is still read off the prose. It is refused
            // inside the client core, before anything crosses to a runner, so
            // there is no word to read (`crates/client/src/actions.rs`).
            val text = error.message.orEmpty().lowercase()
            if (text.contains("too large") || text.contains("payload")) {
                return "That was too large to send. Try a smaller image."
            }
            return troubleFor(
                error.refusalWord, error.message,
                "Couldn’t reach this runner. Your message wasn’t sent.",
            ).sentence
        }
    }
}
