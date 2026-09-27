package com.farcooler.ui

import com.farcooler.model.TaskRow
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * The wall clock a card reads, while its screen is showing.
 *
 * The board recomposes only when a task changes, so a card that read the clock
 * once would still say "Added just now" five hours later on a quiet board.
 * So this reads [now] and hands it to [onTick] on [start], and again on each
 * minute boundary after, until [stop].
 *
 * Started and stopped with the screen (`rememberMinuteClock` in BoardScreen.kt
 * ties them to ON_START and ON_STOP), for two reasons. Nothing ticks while the
 * app is in the background. And [start] reads the clock before it returns, so
 * the first frame after the app comes back or the phone wakes is right: a
 * loop left running would owe up to a minute of AWAKE time on its pending
 * `delay`, which runs on uptime and stands still while the phone sleeps, and
 * a card hours old would read "Added just now" until it came due.
 *
 * Plain callbacks and a scope rather than Compose state, so `MinuteClockTest`
 * can drive it on the coroutine test scheduler.
 */
class MinuteClock(
    private val now: () -> Long,
    private val onTick: (Long) -> Unit,
) {
    private var ticking: Job? = null

    /**
     * Read the clock now, then on each minute boundary. Undispatched, so the
     * first reading is taken and handed over before this returns, whatever
     * [scope]'s dispatcher, rather than whenever that dispatcher next runs.
     */
    fun start(scope: CoroutineScope) {
        ticking?.cancel()
        ticking = scope.launch(start = CoroutineStart.UNDISPATCHED) {
            while (true) {
                val at = now()
                onTick(at)
                delay(TaskRow.untilNextMinuteMs(at))
            }
        }
    }

    fun stop() {
        ticking?.cancel()
        ticking = null
    }
}
