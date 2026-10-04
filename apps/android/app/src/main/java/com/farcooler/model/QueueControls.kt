package com.farcooler.model

/**
 * Whether a runner can change a queued message, and what to say when it can't.
 *
 * Edit, Remove and Send now are three calls that a runner older than the
 * `agent_queue` capability refuses as an unknown method (ov-171). Matches
 * AgentKit's `QueueControls`.
 *
 * Conservative: a runner that serves the calls without advertising
 * `agent_queue` is gated too. A runner not yet heard from is not, because it
 * hasn't refused anything and a refusal is shown anyway.
 *
 * Dimmed with the sentence, never hidden: [DaemonBuild.can] states that rule
 * for this app, and a Material text action that is disabled still reads.
 */
sealed interface QueueControls {
    data object Available : QueueControls

    data class Unavailable(val sentence: String) : QueueControls

    val isAvailable: Boolean get() = this is Available

    /** The three calls, and the step each one says it couldn't finish. */
    enum class Action(val failed: String) {
        EDIT("Couldn’t save that edit."),
        STEER("Couldn’t send that into the running turn."),
        CANCEL("Couldn’t take that message back."),
    }

    companion object {
        const val OLDER_RUNNER_SENTENCE =
            "This runner can’t change queued messages. Update it to edit or cancel them."

        fun gate(daemon: DaemonBuild?): QueueControls = when {
            daemon == null -> Available
            daemon.can(Capability.AGENT_QUEUE) -> Available
            else -> Unavailable(OLDER_RUNNER_SENTENCE)
        }

        /** The sentence beside a queue call a runner refused anyway. */
        fun refusal(action: Action, word: String?, message: String?): String =
            troubleAfter(word, message, action.failed).sentence
    }
}
