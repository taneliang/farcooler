package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.core.DisconnectedException
import com.farcooler.model.RunnerRefusal
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** What became of one `terminal.write`, in the three ways that matter to typed input (ov-238). */
sealed interface WriteOutcome {
    /** The runner answered that it wrote the bytes. */
    data object Written : WriteOutcome

    /** The bytes provably never reached the runner, so sending them again can't type them twice. */
    data class NeverSent(val reason: InputHold.Reason) : WriteOutcome

    /**
     * They may have arrived. A deadline "cannot unsend" a key already on the wire
     * (`crates/client/src/deadlines.rs`), and a link that drops mid-call may have
     * carried it, so these are never sent again.
     */
    data object MaybeSent : WriteOutcome

    companion object {
        /**
         * The outcome of a failed call. [notSent] is the answer line's `not_sent`,
         * which only a call with no session to go on carries. A runner's own
         * refusal means it wrote nothing. Everything else is doubt, and doubt is
         * not resent. AgentKit's `WriteOutcome.failed`, rule for rule.
         */
        fun failed(word: String?, disconnected: Boolean, notSent: Boolean): WriteOutcome = when {
            notSent -> NeverSent(InputHold.Reason.DISCONNECTED)
            disconnected || word == null || word == RunnerRefusal.TIMED_OUT_WORD -> MaybeSent
            else -> NeverSent(InputHold.Reason.REFUSED)
        }

        /** [failed], from what a call threw. */
        fun of(error: Throwable): WriteOutcome {
            val lost = error as? DisconnectedException
            return failed(
                word = (error as? CoreException)?.word,
                disconnected = lost != null,
                notSent = lost?.notSent == true,
            )
        }
    }
}

/**
 * Typed input a terminal could not send, held until the person decides, and the
 * order it goes in (ov-238). AgentKit's `InputHold`, rule for rule:
 *
 * - **Only input that provably never left the phone is held**, and it goes out
 *   again only on Try again. Discard drops it. A new key never flushes it.
 * - **A timed-out or dropped write is not held and not resent.** The line says
 *   some typing may not have arrived, and that is all.
 * - **Order is kept.** Writes go one at a time: keys typed while one is in
 *   flight, or while input is held, queue behind it, so a later key can't
 *   overtake an earlier failed one. Held input is cleared only once an answer
 *   confirms it was written.
 * - **It is bounded**: [CAP] bytes, the earliest kept, and the line says so. It
 *   is dropped when the pane goes away ([paneClosed]).
 *
 * Used from one thread, the session's own; nothing here locks.
 */
class InputHold {
    /** Why input is held. */
    enum class Reason { DISCONNECTED, REFUSED }

    /** The line to draw: [sentence], with Try again and Discard when [holding]. */
    data class Line(val sentence: String, val holding: Boolean)

    private var held = ByteArray(0)
    private var reason = Reason.REFUSED
    private var truncated = false
    private var maybeLost = false
    private var pending = ByteArray(0)
    /** Typed while a Try again was in flight, past [CAP], and dropped. */
    private var droppedPending = false
    private var sending = false
    private var epoch = 0

    private val _line = MutableStateFlow<Line?>(null)

    /** Null when there is nothing to say. */
    val line: StateFlow<Line?> = _line.asStateFlow()

    /** What is held, for tests. */
    val heldBytes: ByteArray get() = held.copyOf()

    /** Type [bytes]: behind anything held or in flight, else sent now. */
    suspend fun type(bytes: ByteArray, send: suspend (ByteArray) -> WriteOutcome) {
        if (bytes.isEmpty()) return
        if (held.isNotEmpty() && !sending) {
            absorb(bytes)
            publish()
            return
        }
        keepPending(bytes)
        if (sending) return
        drain(send)
    }

    /** Try again: send what is held, then what was typed behind it. */
    suspend fun retry(send: suspend (ByteArray) -> WriteOutcome) {
        if (held.isEmpty() || sending) return
        sending = true
        val mine = epoch
        val outcome = try {
            send(held)
        } finally {
            // A cancelled call must not leave the hold believing one is in flight.
            if (mine == epoch) sending = false
        }
        if (mine != epoch) return
        when (outcome) {
            WriteOutcome.Written -> clearHeld()
            is WriteOutcome.NeverSent -> {
                reason = outcome.reason
                absorb(pending)
                pending = ByteArray(0)
                if (droppedPending) truncated = true
                droppedPending = false
                publish()
                return
            }
            WriteOutcome.MaybeSent -> {
                clearHeld()
                maybeLost = true
            }
        }
        publish()
        drain(send)
        // Keys past the cap are gone whatever the retry did; say so.
        if (droppedPending) maybeLost = true
        droppedPending = false
        publish()
    }

    /** Discard: drop what is held. */
    fun discard() {
        clearHeld()
        pending = ByteArray(0)
        droppedPending = false
        publish()
    }

    /** Dismiss the "may not have reached" line. */
    fun dismissMaybeLost() {
        maybeLost = false
        publish()
    }

    /** The pane closed or went away: nothing held or queued is sent, ever. */
    fun paneClosed() {
        epoch++
        sending = false
        clearHeld()
        pending = ByteArray(0)
        droppedPending = false
        maybeLost = false
        publish()
    }

    private suspend fun drain(send: suspend (ByteArray) -> WriteOutcome) {
        sending = true
        val mine = epoch
        try {
            while (pending.isNotEmpty() && held.isEmpty()) {
                val batch = pending
                pending = ByteArray(0)
                val outcome = send(batch)
                if (mine != epoch) return
                when (outcome) {
                    WriteOutcome.Written -> maybeLost = false
                    is WriteOutcome.NeverSent -> {
                        reason = outcome.reason
                        held = ByteArray(0)
                        truncated = false
                        absorb(batch)
                        absorb(pending)
                        pending = ByteArray(0)
                    }
                    WriteOutcome.MaybeSent -> maybeLost = true
                }
                publish()
            }
        } finally {
            if (mine == epoch) sending = false
        }
    }

    /** Queue typed keys behind a write in flight, at most [CAP] bytes, the earliest kept. */
    private fun keepPending(bytes: ByteArray) {
        val room = maxOf(0, CAP - pending.size)
        if (bytes.size > room) droppedPending = true
        pending += bytes.copyOf(minOf(room, bytes.size))
    }

    private fun absorb(bytes: ByteArray) {
        val room = maxOf(0, CAP - held.size)
        if (bytes.size > room) truncated = true
        held += bytes.copyOf(minOf(room, bytes.size))
    }

    private fun clearHeld() {
        held = ByteArray(0)
        truncated = false
    }

    private fun publish() {
        _line.value = when {
            held.isNotEmpty() -> {
                val why = if (reason == Reason.DISCONNECTED) {
                    "Far Cooler lost the connection before your typing reached the runner."
                } else {
                    "The runner didn’t take your typing."
                }
                Line(if (truncated) "$why Only the first 4 KB is kept." else why, holding = true) // casing ok: sentence after $why
            }
            maybeLost -> Line("Some typing may not have reached the runner.", holding = false)
            else -> null
        }
    }

    companion object {
        /** The most input kept for Try again. */
        const val CAP = 4096
        const val RETRY = "Try again"
        const val DISCARD = "Discard"
        const val DISMISS = "OK"
    }
}
