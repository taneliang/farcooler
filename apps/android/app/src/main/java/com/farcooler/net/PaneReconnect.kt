package com.farcooler.net

/**
 * A pane whose link dropped and is being put back: a wait, never a failure.
 * The terminal pane's, word for word with iOS's `PaneReconnect`.
 */
object PaneReconnect {
    /** The sentence a dropped link is recorded as. */
    const val SENTENCE = "The connection to this runner dropped. Reconnecting…"

    /** The headline of the calm state. */
    const val TITLE = "Reconnecting…"

    /** Whether a recorded sentence is this wait rather than a failure. */
    fun isReconnecting(sentence: String?): Boolean = sentence == SENTENCE
}
