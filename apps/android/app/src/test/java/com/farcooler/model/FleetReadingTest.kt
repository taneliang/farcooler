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
        assertEquals("3 live · 2 runners", liveSummary(runners))

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
            reassurance(lost, " on studio", workspaces = 2),
        )

        val launching = listOf(runner(RunnerLink.CONNECTING, 0), runner(RunnerLink.CONNECTING, 0))
        assertEquals(FleetReading.Connecting, fleetReading(launching))
        assertEquals("Connecting…", liveSummary(launching))
        assertEquals("Connecting…", reassurance(launching, "", workspaces = 0))

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
        assertEquals("One agent is working.", reassurance(mixed, "", workspaces = 3))
        assertEquals(
            "2 agents are working on studio.",
            reassurance(listOf(runner(RunnerLink.ANSWERING, 2)), " on studio", workspaces = 1),
        )
        assertEquals(
            "Nothing is running on studio yet.",
            reassurance(listOf(runner(RunnerLink.ANSWERING, 0)), " on studio", workspaces = 0),
        )
        assertEquals(
            "Nothing is running.",
            reassurance(listOf(runner(RunnerLink.ANSWERING, 0)), "", workspaces = 2),
        )
        assertEquals("Nothing is running.", reassurance(emptyList(), "", workspaces = 0))
        assertEquals("No runners", liveSummary(emptyList()))
    }
}
