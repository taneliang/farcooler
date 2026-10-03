package com.farcooler.model

/**
 * One pane's answer to its agent's permission ask, while it is sent and
 * after, for the card that offers it. A port of AgentKit's
 * `PermissionAnswering`, case for case.
 *
 * **The card stays up until the runner takes the answer.** It used to come
 * down on the tap, with `terminal.agent_answer`'s error dropped. For a claude
 * TUI ask the daemon holds the hook until an answer lands, so a refused answer
 * left the ask held on the runner and gone from the phone for good: its
 * permission was behind the stream's cursor and nothing else would show it
 * again. Now the buttons are off while the answer is out, and a failure leaves
 * the card up with a sentence saying so, to be tried again.
 *
 * **One refusal is not a failure.** `resource-conflict` is what the daemon
 * says for an ask nothing holds any more: answered at the keyboard, from the
 * watch or another phone, or withdrawn when its hold ran out. The ask is over,
 * so the card comes down without a word.
 */
data class PermissionAnswering(
    /** The request whose answer is out, if one is. */
    val sending: String? = null,
    /** The last answer that did not land, and what to say about it. */
    val failure: Failure? = null,
) {
    data class Failure(val request: String, val sentence: String)

    /** How one answer ended. */
    sealed interface Outcome {
        /** The runner took it. */
        data object Sent : Outcome

        /** Nothing holds that ask any more: it was answered elsewhere, or it ended. */
        data object AnsweredElsewhere : Outcome

        /** It did not land, and the ask may still be waiting. */
        data class Failed(val sentence: String) : Outcome
    }

    /** Take ownership of one answer, or null while another is out. */
    fun begin(request: String): PermissionAnswering? =
        if (sending != null) null else PermissionAnswering(sending = request, failure = null)

    /**
     * Say how the answer to [request] ended: the new state, and whether the
     * card for it comes down.
     */
    fun finish(request: String, outcome: Outcome): Pair<PermissionAnswering, Boolean> {
        val sending = if (sending == request) null else sending
        return when (outcome) {
            Outcome.Sent, Outcome.AnsweredElsewhere ->
                PermissionAnswering(sending, failure?.takeIf { it.request != request }) to true
            is Outcome.Failed ->
                PermissionAnswering(sending, Failure(request, outcome.sentence)) to false
        }
    }

    /** Whether the buttons for [request] are off, because an answer is out. */
    fun isSending(request: String): Boolean = sending == request

    /** What to say under the card for [request], if its last answer failed. */
    fun sentence(request: String): String? = failure?.takeIf { it.request == request }?.sentence

    companion object {
        /**
         * The outcome of an answer the runner did not take, from the word it
         * refused with. A blank word is a dropped link or a timeout, where the
         * answer may or may not have landed.
         *
         * Trying again is safe in every case: a second answer to a hook ask
         * that the first one settled is refused as `resource-conflict`, which
         * clears the card.
         */
        fun outcome(refusedWith: String?): Outcome {
            if (refusedWith == RunnerRefusal.RESOURCE_CONFLICT.word) return Outcome.AnsweredElsewhere
            if (refusedWith.isNullOrEmpty()) {
                return Outcome.Failed("Your answer may not have reached the runner. Try again.")
            }
            return Outcome.Failed(
                troubleAfter(refusedWith, null, "The runner didn’t take your answer.").sentence
            )
        }
    }
}
