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
 * Not thread-safe: called only from the session's own single-threaded scope.
 */
class StreamRetry(
    private val scope: CoroutineScope,
    private val wanted: () -> Boolean,
) {
    private var job: Job? = null

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
}
