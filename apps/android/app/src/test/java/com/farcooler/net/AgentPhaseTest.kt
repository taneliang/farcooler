package com.farcooler.net

import com.farcooler.model.Trouble
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * What a pane may claim about itself, and when.
 *
 * There is no emulator in this program, so this file is the whole proof. Every
 * state it walks through used to be reachable only by owning a runner whose
 * shim was slow or whose link was down — which is most of why they were all
 * drawn as one red failure and nobody could see that they were.
 */
class AgentPhaseTest {

    private val dropped = Trouble("The connection to this runner dropped. Reconnecting…")
    private val unfinished = Trouble("The request that reads it didn’t finish.", "ssh: timed out")

    /** Nothing has come back. Not that there is a session, not that there isn't. */
    @Test
    fun `a stream that has not polled claims nothing`() {
        assertEquals(AgentPhase.Opening, AgentPhases().phase)
    }

    /**
     * The bug, stated as a test.
     *
     * An epoch-0 empty batch is the daemon's own honest answer for a terminal
     * that has not run an agent yet, and the client used to render it as a red
     * failure — which is a client disagreeing with the server about what the
     * server just said.
     */
    @Test
    fun `a pane with no agent is never alarming, however long it waits`() {
        val phases = AgentPhases()
        phases.read(Poll.NoSession, 0)
        // A minute, two minutes, an hour. `TOO_LONG` is unreachable from here.
        for (elapsed in listOf(0L, 5_000L, 30_000L, 60_000L, 3_600_000L)) {
            val phase = phases.read(Poll.NoSession, elapsed)
            assertEquals(
                "at ${elapsed}ms",
                AgentPhase.Starting(if (elapsed >= 5_000L) Waited.A_WHILE else Waited.A_MOMENT),
                phase,
            )
        }
    }

    /**
     * Patience and alarm are two numbers because they answer two questions.
     *
     * One constant answering both is the exact shape of the bug this file is
     * about: five seconds is "should this say more", thirty is "should this
     * raise an alarm", and everything between them is a screen that says what it
     * is waiting on without calling it dead.
     */
    @Test
    fun `a failing poll says more at five seconds and raises an alarm at thirty`() {
        val phases = AgentPhases()
        assertEquals(AgentPhase.Failing(dropped, Waited.A_MOMENT), phases.read(Poll.Failed(dropped), 0))
        assertEquals(
            AgentPhase.Failing(dropped, Waited.A_MOMENT), phases.read(Poll.Failed(dropped), 4_999))
        assertEquals(
            AgentPhase.Failing(dropped, Waited.A_WHILE), phases.read(Poll.Failed(dropped), 5_000))
        assertEquals(
            AgentPhase.Failing(dropped, Waited.A_WHILE), phases.read(Poll.Failed(dropped), 29_999))
        assertEquals(
            AgentPhase.Failing(dropped, Waited.TOO_LONG), phases.read(Poll.Failed(dropped), 30_000))
    }

    /**
     * A different sentence about the same outage is the same outage.
     *
     * [Connection] can switch a pane from "the request didn't finish" to "the
     * connection dropped. Reconnecting…" between two polls. Restarting the alarm
     * clock every time the link changed its mind would mean the alarm never
     * rang, which is when it is worth most.
     */
    @Test
    fun `a changed sentence does not restart the alarm clock`() {
        val phases = AgentPhases()
        phases.read(Poll.Failed(unfinished), 0)
        phases.read(Poll.Failed(dropped), 20_000)
        assertEquals(
            AgentPhase.Failing(unfinished, Waited.TOO_LONG),
            phases.read(Poll.Failed(unfinished), 31_000),
        )
    }

    /** One good batch ends it, and the next failure starts over from calm. */
    @Test
    fun `a session that comes back clears the alarm and the clock`() {
        val phases = AgentPhases()
        phases.read(Poll.Failed(dropped), 0)
        assertEquals(
            AgentPhase.Failing(dropped, Waited.TOO_LONG), phases.read(Poll.Failed(dropped), 40_000))
        assertEquals(AgentPhase.Live, phases.read(Poll.Served, 40_700))
        assertEquals(
            AgentPhase.Failing(dropped, Waited.A_MOMENT), phases.read(Poll.Failed(dropped), 41_400))
    }

    /** A shim that comes up mid-wait is a live session, not a late failure. */
    @Test
    fun `a pane whose agent arrives goes live`() {
        val phases = AgentPhases()
        phases.read(Poll.NoSession, 0)
        assertEquals(AgentPhase.Starting(Waited.A_WHILE), phases.read(Poll.NoSession, 9_000))
        assertEquals(AgentPhase.Live, phases.read(Poll.Served, 9_700))
    }

    /**
     * A shim that GOES AWAY mid-conversation is starting again, not live.
     *
     * `AgentSupervisor::replay` answers epoch 0 once it holds no session for the
     * terminal, and [AgentStream] resets the transcript to match. Nothing on iOS
     * covers this case: it keeps a published epoch for the badge and lets the
     * phase stay `.live`, so the pane would sit there inviting a message into a
     * session that had just ended. Here one rule — the daemon's epoch is what
     * says a session exists — answers it without a second field.
     */
    @Test
    fun `a session that goes away stops being live`() {
        val phases = AgentPhases()
        assertEquals(AgentPhase.Live, phases.read(Poll.Served, 0))
        assertEquals(AgentPhase.Starting(Waited.A_MOMENT), phases.read(Poll.NoSession, 700))
        assertEquals(AgentPhase.Starting(Waited.A_WHILE), phases.read(Poll.NoSession, 6_000))
    }

    /**
     * Android's own case: the poll stops when the pane is not the tab being
     * read and when the app is backgrounded, while the phase stays alive in a
     * mounted pane.
     *
     * The first poll after a resume is the one most likely to fail — the SSH
     * link was torn down while the process was frozen — and without
     * [AgentPhases.resumed] that single failure lands on a clock that has been
     * running all night and paints the pane red instantly. Which is the original
     * bug, reproduced exactly, by the fix for it.
     */
    @Test
    fun `a poll resumed after a night in a pocket is not instantly a failure`() {
        val phases = AgentPhases()
        phases.read(Poll.Failed(dropped), 0)
        val overnight = 8 * 3_600_000L
        phases.resumed(overnight)
        assertEquals(
            AgentPhase.Failing(dropped, Waited.A_MOMENT),
            phases.read(Poll.Failed(dropped), overnight),
        )
        // And it still gets there on its own thirty seconds later.
        assertEquals(
            AgentPhase.Failing(dropped, Waited.TOO_LONG),
            phases.read(Poll.Failed(dropped), overnight + 30_000),
        )
    }

    /**
     * A clock that goes backwards must not freeze the screen.
     *
     * `SystemClock.elapsedRealtime()` counts since boot and cannot be stepped,
     * which is why the app passes it — but a caller that passed a wall clock
     * would otherwise get a negative elapsed and a phase stuck at `A_MOMENT`
     * for as long as the correction was worth.
     */
    @Test
    fun `a clock that steps backwards does not stick`() {
        val phases = AgentPhases()
        phases.read(Poll.Failed(dropped), 100_000)
        assertEquals(
            AgentPhase.Failing(dropped, Waited.A_MOMENT), phases.read(Poll.Failed(dropped), 1_000))
        // Measured from where the clock now is, not held until it catches up.
        assertEquals(
            AgentPhase.Failing(dropped, Waited.A_WHILE), phases.read(Poll.Failed(dropped), 6_000))
    }
}
