package com.farcooler.net

/**
 * The fingerprint question, and every answer it has.
 *
 * Not a [Connection.Failure]. Nothing failed — a runner presented a key this
 * device has never seen, which is the ordinary first contact — and it is
 * deliberately not headlined as though something had. It sits in this package
 * because of what one of its answers PRODUCES: backing out lands on
 * [Connection.Failure.KEY_NOT_TRUSTED], whose one useful move is back to this
 * same question, and the two halves of that loop drifting apart is what
 * [Connection.Failure.Said] exists to stop.
 *
 * **The list is the point, and this app shipped it one short.** The row offered
 * "Trust it" and "Edit", so the ways past a fingerprint somebody cannot vouch
 * for were agreeing to it and force-quitting the app — "Edit" is not a third:
 * a person who cannot recognize a key has nothing to correct, because the
 * address is right and that is exactly what makes the key worth asking about.
 * [Connection.declineHostKey] was written, with a comment naming the screen it
 * should land on, and called from nowhere. iOS shipped the same defect on the
 * same row and `53c6c57` is where it was fixed.
 *
 * So the answers are a list here rather than a hand of `TextButton`s in a
 * composable, and `RunnerStatusRow` iterates it. Dropping one is now an edit to
 * this file, in front of `HostKeyQuestionTest` — which matters more on this
 * platform than on the other, because a composable is something CI compiles and
 * never runs, and a button that is simply absent from a screen leaves no trace
 * in a build at all.
 */
object HostKeyQuestion {

    /**
     * In the order they are offered.
     *
     * Trusting first because it is the answer; the other two are ways out, and
     * a way out that came first would read as the recommendation.
     */
    enum class Answer {
        /**
         * Record the fingerprint on screen, which is also the connect. See
         * `RunnerStore.trust`, and the retry the screens fire beside it — the
         * approved copy of the runner has to reach the dial, or the next
         * attempt asks the same question again.
         */
        TRUST_IT,

        /**
         * Back out without answering. **Not a synonym for [EDIT]**: leaving
         * somebody only a destructive edit and an agreement is leaving them the
         * agreement. Lands on [Connection.Failure.KEY_NOT_TRUSTED] by way of
         * [Connection.declineHostKey].
         */
        NOT_NOW,

        /**
         * Correct this runner's details, for the case where the address really
         * is wrong and the key on screen belongs to somebody else's machine.
         */
        EDIT,
        ;

        /**
         * Sentence case, as every button on this screen is.
         *
         * Not iOS's words. That app says "Trust This Runner" and "Not Now"
         * because its buttons are title-cased; this one already says "Trust
         * it", "Try again" and "Reconnect now", and matching the file beats
         * matching the other platform.
         */
        val label: String
            get() = when (this) {
                TRUST_IT -> "Trust it"
                NOT_NOW -> "Not now"
                EDIT -> "Edit"
            }

        /**
         * Whether this is the answer, as opposed to a way out of the question.
         *
         * One of them, and the row draws it with the weight: three buttons at
         * one emphasis give a person three things to weigh when only one of
         * them answers what was asked.
         */
        val isTheAnswer: Boolean get() = this == TRUST_IT
    }

    /**
     * Backing out leaves this runner in this state.
     *
     * Named here so the circle is closed in one place: [Connection.Failure.Said.declined]
     * is the sentence, this is what it classifies to, and "Show the key again"
     * is the move back. `HostKeyQuestionTest` walks the whole circle, because
     * every link in it is a string match or a table and none of them fails
     * loudly on its own.
     */
    val declining: Connection.Failure get() = Connection.Failure.KEY_NOT_TRUSTED
}
