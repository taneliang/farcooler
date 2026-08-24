package com.farcooler.net

import com.farcooler.model.Trouble

/**
 * What an agent pane can honestly say about itself.
 *
 * It had no such state, and that is the whole of "'Could not load this session'
 * shows up for quite a long time". [AgentStream] published one nullable
 * [Trouble] and [com.farcooler.ui.AgentScreen] asked one question of it — is it
 * set — and had two answers. But every one of the three things that set it is a
 * pane STILL TRYING: a daemon that has no session for this terminal yet, a link
 * that says in its own sentence that it is reconnecting, and a single poll that
 * did not come back out of one every 700 ms. Nothing here is ever actually
 * dead, so a screen with one bit could only be wrong — and it was.
 *
 * `40a6cd1` fixed this on iOS and recorded that Android had it identically.
 * This is that fix, and the words below are that fix's words, because the fact
 * being reported is the same fact on both phones.
 *
 * ## Why the two waits are two numbers
 *
 * "Should this say more" and "should this raise an alarm" are different
 * questions with different right answers, and one constant answering both is
 * the exact shape of the bug — the same shape iOS's
 * `TerminalSession.firstPaintGrace` was split off `firstByteDeadline` to end.
 * (Android's [TerminalSession] has no such pair: its screen poll backs off
 * rather than deadlines, so there is nothing here to follow, only the reason
 * to.) See [AgentPhases.PATIENCE_MS] and [AgentPhases.ALARM_MS].
 *
 * ## Where this differs from iOS
 *
 * iOS publishes `phase` and `waited` as two `@Published` properties and keeps
 * the sentence in a third. Here they are one value, for the reason `cd17a5e`
 * gives: several flows mutated together in one poll are several recomposition
 * triggers and real torn frames — a headline from one state under a sentence
 * from another, which is precisely the mismatch this file exists to end. So
 * [Failing] CARRIES its sentence, the way [TerminalSession.Phase.Failed]
 * already does, and there is no way to be [Starting] and hold a failure's words
 * at the same time.
 */
sealed interface AgentPhase {

    /**
     * The first poll has not come back. Nothing is known yet — not that there
     * is a session, not that there isn't.
     */
    data object Opening : AgentPhase

    /**
     * The daemon answered and holds no agent session for this terminal.
     *
     * Ordinary while a shim is coming up, and — from this side of an SSH link —
     * indistinguishable from a pane that will never have one. That
     * indistinguishability is exactly why it is a state and not an error: the
     * daemon calls an empty batch "the honest answer for one that has not run
     * an agent yet" (`crates/daemon/src/rpc.rs`, `terminal.agent_subscribe`),
     * and a client that renders it as a failure is disagreeing with the server
     * about what it just said.
     *
     * NEVER ALARMING, however long it lasts — see [Waited.TOO_LONG].
     */
    data class Starting(val waited: Waited) : AgentPhase

    /** A session is being served, whether or not it has any rows yet. */
    data object Live : AgentPhase

    /**
     * A poll failed. Still retrying, every 700 ms, forever — which is why this
     * alone is not enough to draw an alarm with.
     */
    data class Failing(val trouble: Trouble, val waited: Waited) : AgentPhase
}

/**
 * How long the current phase has been going on, in the only three widths that
 * change what a screen says.
 */
enum class Waited {
    /** Ordinary. A spinner, and no words about the waiting. */
    A_MOMENT,

    /**
     * Long enough to deserve a sentence saying what is being waited on. NOT
     * long enough to deserve an alarm.
     */
    A_WHILE,

    /**
     * Long enough that calling it a failure is the honest thing.
     *
     * Reached only from [AgentPhase.Failing]. A pane the daemon says has no
     * agent never gets here at all: that is not a failure however long it
     * lasts, and painting it red would be the original bug wearing a delay.
     */
    TOO_LONG,
}

/**
 * What one poll learned, which is all [AgentPhases] needs to be told.
 *
 * Three outcomes, because the daemon's epoch answers the only question that
 * matters here: a non-zero one means it is serving a session for this pane, and
 * zero is its own way of saying it has never seen one.
 */
sealed interface Poll {
    /** The daemon answered, and holds no session for this pane. */
    data object NoSession : Poll

    /** The daemon answered with a session, whether or not it had new events. */
    data object Served : Poll

    /** The read did not finish. [trouble] is what there is to say about that. */
    data class Failed(val trouble: Trouble) : Poll
}

/**
 * The phase machine: what each poll learned, plus a clock, becomes what the
 * screen may claim.
 *
 * Kept out of [AgentStream] and given the clock as a parameter for the reason
 * [WatchingClaim] is: this is the only part of that path with a decision in it,
 * everything around it is a round trip, and there is no emulator here — so the
 * decision is the part that has to be provable on a laptop.
 *
 * [nowMs] is the caller's clock and is `SystemClock.elapsedRealtime()` in the
 * app, which counts since boot and is immune to the clock being stepped. An NTP
 * correction against a wall clock reads as a phase that began in the future,
 * and every comparison below would then be false for as long as the correction
 * was worth.
 *
 * ## No timer
 *
 * A `StateFlow` does not emit because time passed, and a composable has no
 * clock. This rides [AgentStream]'s existing 700 ms poll — the very thing it is
 * describing — at the cost of being at most one poll late, which nobody can
 * see, and at a saving of one coroutine per mounted pane. Android needs that
 * saving more than iOS does: [com.farcooler.ui.WorkspaceScreen] keeps several
 * panes mounted at once (`e23718c`), so a timer here would be a timer per tab.
 */
class AgentPhases(
    private val patienceMs: Long = PATIENCE_MS,
    private val alarmMs: Long = ALARM_MS,
) {
    var phase: AgentPhase = AgentPhase.Opening
        private set

    /** When [phase] began, for the two thresholds. */
    private var since = 0L

    /**
     * Fold one poll's outcome in, and answer what the screen may now claim.
     *
     * The phase is recomputed whole rather than nudged, so there is no
     * transition table to get wrong: what is true is a function of what the
     * daemon just said and how long it has been saying it.
     */
    fun read(poll: Poll, nowMs: Long): AgentPhase {
        // A clock that went backwards has made the old stamp meaningless, so
        // this starts over from it rather than clamping to zero — clamping
        // would hold the phase at its first width until the clock caught up
        // again, which for a wall clock stepped by an NTP correction is however
        // long the correction was worth. `SystemClock.elapsedRealtime()` cannot
        // do this; a caller that passed something else could.
        if (nowMs < since) since = nowMs
        val elapsed = nowMs - since
        val next = when (poll) {
            // Each arm asks whether the phase it is about to produce is the
            // one already standing, because `elapsed` is measured from the
            // phase that is standing NOW. A poll that changes the answer is a
            // phase that has lasted no time at all, and `since` is restamped
            // for it below.
            is Poll.NoSession -> AgentPhase.Starting(
                // Capped at A_WHILE on purpose, and not by the caller
                // remembering to. See [Waited.TOO_LONG].
                if (phase is AgentPhase.Starting && elapsed >= patienceMs) Waited.A_WHILE
                else Waited.A_MOMENT
            )

            is Poll.Served -> AgentPhase.Live

            is Poll.Failed -> AgentPhase.Failing(
                poll.trouble,
                // A DIFFERENT TROUBLE IS THE SAME FAILURE. `Connection` can
                // switch a pane's sentence from "the request didn't finish" to
                // "the connection dropped. Reconnecting…" between two polls,
                // and treating that as a fresh failure would restart the alarm
                // clock every time the link changed its mind — which is exactly
                // when the alarm is worth having.
                if (phase is AgentPhase.Failing && elapsed >= alarmMs) Waited.TOO_LONG
                else if (phase is AgentPhase.Failing && elapsed >= patienceMs) Waited.A_WHILE
                else Waited.A_MOMENT
            )
        }
        if (!sameKind(phase, next)) since = nowMs
        phase = next
        return next
    }

    /**
     * The poll is about to run again after being stopped.
     *
     * ANDROID HAS NO iOS COUNTERPART FOR THIS, and needs one. A pane here stops
     * polling whenever it is not the tab being read and whenever the app is
     * backgrounded ([AgentStream.stop]), while its transcript and this phase
     * stay alive in a mounted pane — that is the point of `e23718c`. So a phone
     * left in a pocket overnight comes back holding a phase from last night,
     * and the first poll after a resume is the one MOST likely to fail: the SSH
     * link was torn down while the process was frozen. Without this, that one
     * failure would land on an hours-old clock and paint the pane red
     * instantly, which is the original bug reproduced exactly.
     *
     * The phase itself is kept — the last thing this pane knew is still the
     * best thing it can say until a new poll says otherwise — and only the
     * clock starts over, because nothing was being waited on while nothing was
     * being asked.
     */
    fun resumed(nowMs: Long) {
        since = nowMs
    }

    /**
     * Whether two phases are the same ANSWER, ignoring how long it has been
     * given for.
     *
     * The clock must survive `A_MOMENT` becoming `A_WHILE`, or the phase would
     * reset itself the instant it aged and could never reach the second
     * threshold.
     */
    private fun sameKind(a: AgentPhase, b: AgentPhase): Boolean = when (a) {
        is AgentPhase.Opening -> b is AgentPhase.Opening
        is AgentPhase.Starting -> b is AgentPhase.Starting
        is AgentPhase.Live -> b is AgentPhase.Live
        is AgentPhase.Failing -> b is AgentPhase.Failing
    }

    companion object {
        /**
         * How long a phase may look like ordinary loading before it says out
         * loud that it is still waiting.
         *
         * A spinner that never ends is its own bug, so this is what stops one.
         * About seven polls.
         */
        const val PATIENCE_MS = 5_000L

        /**
         * How long before a screen that is still trying is allowed to look like
         * a failure.
         *
         * A SECOND number, deliberately, and far larger — see the class note on
         * [AgentPhase]. Thirty seconds is roughly forty consecutive failed
         * round trips, which is past any hiccup and past a
         * [Connection.refresh] that is going to succeed.
         */
        const val ALARM_MS = 30_000L
    }
}
