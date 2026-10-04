package com.farcooler.net

import com.farcooler.model.RunnerRefusal

/**
 * Typed input the runner did not take, held so it can be sent again.
 *
 * A terminal write used to be `attempt { … }` with the result thrown away: a
 * keystroke the runner never answered was gone, and the screen looked the same
 * as when it worked (ov-238). This is the value that makes it visible. The
 * phone keeps the bytes, says in one quiet line why they are waiting, and
 * offers Try again, which sends them first and in order.
 *
 * Word for word with `UnsentInput` in AgentKit, because the same failure must
 * not be described two ways depending on which phone is in your hand.
 */
class UnsentInput(val bytes: ByteArray, val why: Why) {
    /** Why the runner did not take it, in the three ways a phone can tell apart. */
    enum class Why {
        /** No answer by the call's deadline. */
        TIMED_OUT,
        /** The link is gone. */
        DISCONNECTED,
        /** Anything else: a refusal, or an answer this build couldn't read. */
        OTHER,
    }

    /** This, with [more] held behind it and the latest reason. */
    fun holding(more: ByteArray, latest: Why) = UnsentInput(bytes + more, latest)

    /**
     * The one line a screen shows: why, then that nothing was lost. It never
     * quotes the core's own words and claims no cause it doesn't know.
     */
    val sentence: String
        get() = when (why) {
            Why.TIMED_OUT -> "The runner took too long to answer. Your typing is waiting."
            Why.DISCONNECTED ->
                "Far Cooler lost the connection to this runner. Your typing is waiting."
            Why.OTHER -> "The runner didn’t take that. Your typing is waiting."
        }

    companion object {
        /** The button beside the line. */
        const val RETRY = "Try again"

        /** The reason for a failed `terminal.write`, from the error it threw. */
        fun whyOf(error: Throwable): Why = when {
            error is com.farcooler.core.DisconnectedException -> Why.DISCONNECTED
            (error as? com.farcooler.core.CoreException)?.word == RunnerRefusal.TIMED_OUT_WORD ->
                Why.TIMED_OUT
            else -> Why.OTHER
        }

        /**
         * What to send now: whatever is held, then the new bytes — the order
         * they were typed in, whether or not the last attempt failed.
         */
        fun bytesToSend(held: UnsentInput?, new: ByteArray): ByteArray =
            (held?.bytes ?: ByteArray(0)) + new
    }
}
