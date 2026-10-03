package com.farcooler.net

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * The one re-attach a terminal pane has waiting, held so that a stop can cancel
 * it rather than race it.
 *
 * It used to be `delay(…); open()` inline at the end of `streamEnded`, with
 * nothing holding it and nothing asking afterwards whether anybody still wanted
 * the pane. A stop that landed during the wait — the tab moved on, the app went
 * to the background — tore the pane down and closed its stream, and then the
 * wait ended and opened a fresh SSH channel for a pane nobody was looking at.
 * Nothing closed that channel until the pane was opened again, and a default
 * sshd gives this whole phone ten of them. The iPhone's
 * `TerminalSession.reattach` is the same rule.
 *
 * Two guards, because a cancel and a wake-up can cross: [cancel] stops a wait
 * that has not ended, and [wanted] is asked once it has, so a retry already
 * resuming when the stop arrived still opens nothing.
 *
 * **It never gives up.** A pane used to stop asking after three dead attaches
 * and poll for good, which on a link that hiccuped three times stranded the
 * pane on the slower path until somebody switched tabs and back. What kept
 * those retries from being a load generator was the cap; the widening wait
 * ([nextDelayMs]) does that now, from [FLOOR_MS] to [CEILING_MS], and the
 * channel is released before every wait (the `before` of [schedule]), so
 * retries cannot pile up channels on the runner. The iPhone's
 * `scheduleStreamRetry` is the same rule.
 *
 * Not thread-safe: called only from the session's own single-threaded scope.
 */
class StreamRetry(
    private val scope: CoroutineScope,
    private val wanted: () -> Boolean,
) {
    private var job: Job? = null
    private var delayMs = FLOOR_MS

    /**
     * How long the next re-attach waits, widening the one after it.
     *
     * Geometric on the poll loop's own factor, with a ceiling thirty times the
     * poll's: an attach on a runner too old for the `terminal_stream` method
     * execs a cold `farcoolerd --stream` there, and retrying that once a second
     * would be a load generator aimed at a runner already having a bad time.
     * Thirty seconds outlasts one attach's worst case, so attempts cannot queue
     * behind each other, and a link that comes back is streaming again inside
     * half a minute with nobody tapping anything.
     */
    fun nextDelayMs(): Long {
        val now = delayMs
        delayMs = minOf((delayMs * FACTOR).toLong(), CEILING_MS)
        return now
    }

    /**
     * Back to [FLOOR_MS]: a stream delivered, or a fresh visit began. A tally
     * of what came before is no evidence about this link.
     */
    fun resetBackoff() {
        delayMs = FLOOR_MS
    }

    /** Whether a re-attach is waiting. */
    val pending: Boolean get() = job?.isActive == true

    /**
     * Run [before], wait [afterMs], then [reattach] if the pane is still
     * [wanted]. Replaces any retry already waiting, so there is only ever one.
     */
    fun schedule(afterMs: Long, before: suspend () -> Unit = {}, reattach: suspend () -> Unit) {
        job?.cancel()
        job = scope.launch {
            before()
            delay(afterMs)
            // No longer pending: this is the one running, and [reattach] goes
            // through an `open` that cancels whatever is pending on its way in,
            // which must not be this.
            job = null
            if (!wanted()) return@launch
            reattach()
        }
    }

    fun cancel() {
        job?.cancel()
        job = null
    }

    companion object {
        /** The common failure is one dropped channel on a link that is otherwise fine. */
        const val FLOOR_MS = 500L
        const val CEILING_MS = 30_000L
        const val FACTOR = 1.6
    }
}
