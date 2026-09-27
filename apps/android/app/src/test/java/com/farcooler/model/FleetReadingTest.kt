package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * What the front door and the drawer may say about a fleet whose runners are
 * not all answering.
 *
 * The Mac's `FleetReadingTests`, for the same rule: only a runner that is
 * connected right now has a count worth adding up. A runner that stays down
 * spends most of its outage reconnecting, and both of these screens counted its
 * last fleet as live the whole time.
 */
class FleetReadingTest {

    private fun runner(link: RunnerLink, count: Int, healthy: Boolean = true) =
        RunnerCount(link, count, healthy)

    /**
     * **The reason this exists.** A reconnecting runner's last count is left
     * out, and its last healthy tmux doesn't speak for the fleet either.
     *
     * Mutation: `fleetReading` filtering on `link != RunnerLink.CONNECTING`
     * instead of `== ANSWERING`. Red: 7 live, and tmux healthy.
     */
    @Test
    fun `only an answering runner is counted`() {
        val runners = listOf(
            runner(RunnerLink.ANSWERING, 3),
            runner(RunnerLink.AWAY, 4),
        )
        assertEquals(FleetReading.Live(3), fleetReading(runners))
        assertEquals("3 live · 1 of 2 runners", liveSummary(runners))

        val lostHealthy = listOf(
            runner(RunnerLink.ANSWERING, 0, healthy = false),
            runner(RunnerLink.AWAY, 4, healthy = true),
        )
        assertEquals(FleetReading.RuntimeDown, fleetReading(lostHealthy))
        assertEquals("tmux unavailable", liveSummary(lostHealthy))
    }

    /**
     * Nothing answering is neither a count nor a failure: "can't say".
     *
     * Mutation: the `Unsaid` arm returning `Live(0)`. Red.
     */
    @Test
    fun `a fleet with nothing answering says it cannot say`() {
        val lost = listOf(runner(RunnerLink.AWAY, 4, healthy = false))
        assertEquals(FleetReading.Unsaid, fleetReading(lost))
        assertEquals("Not connected", liveSummary(lost))
        assertEquals(
            "Can’t say what’s running until a runner answers.",
            reassurance(lost, " on studio", worktrees = 2),
        )

        val launching = listOf(runner(RunnerLink.CONNECTING, 0), runner(RunnerLink.CONNECTING, 0))
        assertEquals(FleetReading.Connecting, fleetReading(launching))
        assertEquals("Connecting…", liveSummary(launching))
        assertEquals("Connecting…", reassurance(launching, "", worktrees = 0))

        // One still on its first read and one lost: nothing will be known
        // soon about the lost one, so the fleet can't say.
        assertEquals(
            FleetReading.Unsaid,
            fleetReading(listOf(runner(RunnerLink.CONNECTING, 0), runner(RunnerLink.AWAY, 2))),
        )
    }

    /** The front door's sentences, over the answering runners only. */
    @Test
    fun `the front door counts working agents on answering runners`() {
        val mixed = listOf(runner(RunnerLink.ANSWERING, 1), runner(RunnerLink.AWAY, 5))
        assertEquals("One agent is working.", reassurance(mixed, "", worktrees = 3))
        assertEquals(
            "2 agents are working on studio.",
            reassurance(listOf(runner(RunnerLink.ANSWERING, 2)), " on studio", worktrees = 1),
        )
        assertEquals(
            "Nothing is running on studio yet.",
            reassurance(listOf(runner(RunnerLink.ANSWERING, 0)), " on studio", worktrees = 0),
        )
        assertEquals(
            "Nothing is running.",
            reassurance(listOf(runner(RunnerLink.ANSWERING, 0)), "", worktrees = 2),
        )
        assertEquals("Nothing is running.", reassurance(emptyList(), "", worktrees = 0))
        assertEquals("No runners", liveSummary(emptyList()))
    }

    /**
     * **The count says whose it is.** "3 live · 2 runners" read as three
     * across both, when the second runner was not answering and none of its
     * panes were in the three. With every runner answering, the plain count.
     *
     * Mutation: the runner count taken from every runner again. Red.
     */
    @Test
    fun `the footer names how many runners the count is from`() {
        val both = listOf(runner(RunnerLink.ANSWERING, 3), runner(RunnerLink.ANSWERING, 1))
        assertEquals("4 live · 2 runners", liveSummary(both))
        val one = listOf(runner(RunnerLink.ANSWERING, 3), runner(RunnerLink.CONNECTING, 0))
        assertEquals("3 live · 1 of 2 runners", liveSummary(one))
        assertEquals("1 live · 1 runner", liveSummary(listOf(runner(RunnerLink.ANSWERING, 1))))
    }

    /**
     * **tmux not answering is not "nothing is running".** It put RuntimeDown on
     * Live's arm with a count of zero, so a runner whose panes could not be
     * read at all said none were running.
     *
     * Mutation: RuntimeDown back on Live's arm. Red: "Nothing is running on studio."
     */
    @Test
    fun `the front door cannot say what runs when tmux is down`() {
        val down = listOf(runner(RunnerLink.ANSWERING, 0, healthy = false))
        assertEquals(
            "Can’t say what’s running on studio: tmux isn’t answering.",
            reassurance(down, " on studio", worktrees = 2),
        )
        assertEquals(
            "Can’t say what’s running: tmux isn’t answering.",
            reassurance(down + runner(RunnerLink.ANSWERING, 0, healthy = false), "", worktrees = 0),
        )
    }

    /**
     * **A link that has not read the fleet yet does not vouch for it** (ov-22
     * M3). A reconnect is Connected a host read and a fleet read before it has
     * heard anything, and the fleet on screen until then is the last link's —
     * agents that may have exited since. So: away, with that fleet; and on a
     * first link, which has no fleet to show, still connecting.
     */
    @Test
    fun `a connected runner answers only once this link has read its fleet`() {
        assertEquals(RunnerLink.AWAY, RunnerLink.ANSWERING.given(FleetRead.EARLIER_LINK))
        assertEquals(RunnerLink.CONNECTING, RunnerLink.ANSWERING.given(FleetRead.NEVER))
        assertEquals(RunnerLink.ANSWERING, RunnerLink.ANSWERING.given(FleetRead.THIS_LINK))
        // Nothing weaker is made stronger by a read.
        assertEquals(RunnerLink.AWAY, RunnerLink.AWAY.given(FleetRead.THIS_LINK))
        assertEquals(RunnerLink.CONNECTING, RunnerLink.CONNECTING.given(FleetRead.THIS_LINK))
    }

    /**
     * **A pane's dot says "can't say" while its runner isn't answering**
     * (ov-22 M9). An exited gray dot, a lost red ring, or no dot for running,
     * is each a claim about now from a fleet read before the link went.
     */
    @Test
    fun `a process dot says only what an answering runner says`() {
        assertEquals(StateKind.UNKNOWN, StateKind.EXITED.said(answering = false))
        assertEquals(StateKind.UNKNOWN, StateKind.RUNNING.said(answering = false))
        assertEquals(StateKind.UNKNOWN, StateKind.LOST.said(answering = false))
        assertEquals(StateKind.LOST, StateKind.LOST.said(answering = true))
        assertEquals(StateKind.RUNNING, StateKind.RUNNING.said(answering = true))
    }

    /**
     * **A first fleet read that fails, without the link dropping, is not
     * "Connecting…" forever** (ov-26 review). A decode error, say: the core
     * does not call it a disconnect, the phase stays Connected, and with no
     * fleet on this link the runner read as connecting while every poll
     * failed the same way. It reads as away ("can't say") until a read
     * lands, and the poll goes on retrying. A new link starts over.
     *
     * Mutation: `failed` leaving NEVER alone. Red: CONNECTING.
     */
    @Test
    fun `a first fleet read that fails reads as away, not connecting`() {
        assertEquals(RunnerLink.AWAY, RunnerLink.ANSWERING.given(FleetRead.NEVER.failed()))
        // A failed poll after a good one is not a disconnection: still answering.
        assertEquals(FleetRead.THIS_LINK, FleetRead.THIS_LINK.failed())
        assertEquals(FleetRead.EARLIER_LINK, FleetRead.EARLIER_LINK.failed())
        // A new link has read nothing: connecting again, or away with old rows.
        assertEquals(FleetRead.NEVER, FleetRead.NEVER.failed().onNewLink())
        assertEquals(FleetRead.EARLIER_LINK, FleetRead.THIS_LINK.onNewLink())
        assertEquals(FleetRead.EARLIER_LINK, FleetRead.EARLIER_LINK.onNewLink())
    }
}
