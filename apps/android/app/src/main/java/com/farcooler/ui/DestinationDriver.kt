package com.farcooler.ui

import com.farcooler.model.Destination
import com.farcooler.model.DestinationResolver
import com.farcooler.model.DestinationResolver.Arrival
import com.farcooler.model.DestinationResolver.Resolution

/**
 * The destination waiting to be opened, one at a time (ov-182, ov-183): a
 * tapped notification, or the place the last run was in.
 *
 * Pure, and asked again on every change until it stops answering [Step.Wait],
 * so a tap that comes before its runner does, on a cold launch, is held for it
 * (at most the arrival's deadline) and then opened or dropped. A tap outranks
 * a restore: whoever asked for something by name must not be moved by a
 * relaunch deciding late. [AppModel] owns the clock and the stack.
 */
class DestinationDriver(private val clock: () -> Long = System::currentTimeMillis) {
    private class Pending(val id: Long, val destination: Destination, val arrival: Arrival, val since: Long)

    private var pending: Pending? = null
    private var counter = 0L

    /** Whether something is waiting. */
    val isPending: Boolean get() = pending != null

    /** What a pending destination's arrival asks of the next step. */
    sealed interface Step {
        /** Nothing waiting, or not yet. */
        data object Wait : Step

        /** Open this, and nothing more is waiting. */
        data class Open(val destination: Destination, val arrival: Arrival) : Step

        /** Connect this runner, then ask again. */
        data class Connect(val host: String) : Step

        /** Leave the app where it is. A notification says why; a restore somebody moved past says nothing. */
        data class Stay(val note: DestinationResolver.Note?, val arrival: Arrival) : Step
    }

    /** Wait for [destination], in place of any before it. */
    fun request(destination: Destination, arrival: Arrival) {
        pending = Pending(++counter, destination, arrival, clock())
    }

    /**
     * Wait to go back to [destination] on a relaunch, unless there are no
     * [runners]: that launch is onboarding, which has nowhere to go back to
     * and no Back button to leave. True when it's waiting.
     */
    fun restore(destination: Destination, runners: Int): Boolean {
        if (runners == 0) return false
        request(destination, Arrival.RESTORE)
        return true
    }

    /** Drop what is waiting, as when the runners are all gone. */
    fun clear() {
        pending = null
    }

    /**
     * The next step given what the phone holds now. [moved] is whether
     * somebody has gone somewhere since the launch began: only a restore
     * yields to it.
     */
    fun step(world: DestinationResolver.World, moved: Boolean): Step {
        val waiting = pending ?: return Step.Wait
        val deadline = when (waiting.arrival) {
            Arrival.RESTORE -> DestinationResolver.Deadline.RESTORE_MS
            Arrival.NOTIFICATION -> DestinationResolver.Deadline.NOTIFICATION_MS
        }
        val resolution = DestinationResolver.resolve(
            waiting.destination, waiting.arrival, world, clock() - waiting.since, deadline, interrupted = moved,
        )
        return when (resolution) {
            Resolution.Wait -> Step.Wait
            is Resolution.Connect -> Step.Connect(resolution.host)
            is Resolution.Open -> {
                pending = null
                Step.Open(resolution.destination, waiting.arrival)
            }
            is Resolution.Stay -> {
                pending = null
                Step.Stay(resolution.note, waiting.arrival)
            }
        }
    }
}
