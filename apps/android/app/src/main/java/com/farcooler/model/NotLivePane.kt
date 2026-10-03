package com.farcooler.model

/**
 * A pane the host says isn't running, and when it's worth asking again.
 *
 * **A port of `AgentKit/NotLivePane.swift`'s `revives`**, replayed by both
 * sides from `test/fixtures/lost-pane.json`. A `NotLive` session is
 * deliberately not polled (`TerminalSession.open`): asking every second would
 * spend a round trip to be told the same true thing. So nothing brought the
 * pane back once it ran again, and a lost pane's Restart led to "Not live"
 * with nothing to press (ov-191 review). The fleet poll carries the new state;
 * this says which changes of it are worth one re-attach.
 *
 * **An edge, not a level.** The state is re-derived on every poll, so the
 * caller asks this of a change and never of a value, or it would re-attach on
 * every poll for as long as the host and the session disagreed.
 */
object NotLivePane {
    fun revives(was: StateKind, now: StateKind): Boolean {
        if (now == was) return false
        return when (now) {
            StateKind.RUNNING, StateKind.STARTING -> true
            StateKind.EXITED, StateKind.ERROR, StateKind.LOST, StateKind.UNKNOWN -> false
        }
    }
}
