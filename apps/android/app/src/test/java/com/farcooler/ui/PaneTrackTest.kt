package com.farcooler.ui

import com.farcooler.model.ShellDirection
import com.farcooler.model.ShellTrack
import com.farcooler.model.Terminal
import com.farcooler.model.Workspace
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What the content track walks, and which panes it draws.
 *
 * The gesture arithmetic is `ShellTest`'s. This is the join between it and the
 * deck: that the sequence a swipe follows is the same one the strip shows, and
 * that every pane the track wants to draw is one the deck is actually holding.
 * The second is the load-bearing half — a track that asked for a pane the deck
 * had evicted would be a swipe that destroys a terminal to show it, which is
 * the one thing `PaneDeck` exists to prevent.
 */
class PaneTrackTest {

    private fun terminal(id: String, changes: Boolean = false) =
        Terminal(id = id, title = id, paneMode = if (changes) "changes" else "agent")

    private fun workspace(vararg terminals: Terminal) =
        Workspace(id = "w", short = "auth-refactor", terminals = terminals.toList())

    private val fleet = trackFleet(workspace(terminal("t1"), terminal("t2")), "w", "laptop")

    // ---- What the sequence is ----

    /**
     * The diff leads, then the panes in fleet order — the same order
     * `TerminalTabStrip` draws. A track whose sequence disagreed with the strip
     * would be two different answers to "what is next to this", and a person
     * would meet both within one screen.
     */
    @Test
    fun theDiffLeadsAndThenThePanesInFleetOrder() {
        val tabs = fleet.workspaces.single().tabs.map { it.id }
        assertEquals(
            listOf(Pane.CHANGES_ID, Pane.Terminal("t1").id, Pane.Terminal("t2").id),
            tabs,
        )
    }

    /**
     * A host-side `changes` pane gets no tab of its own: it IS the diff, and two
     * tabs onto one diff is a choice with no difference behind it. `Pane.of`
     * folds it the same way.
     */
    @Test
    fun aHostSideChangesPaneIsNotASecondTab() {
        val folded = trackFleet(
            workspace(terminal("t1"), terminal("diff", changes = true)), "w", "laptop")
        assertEquals(
            listOf(Pane.CHANGES_ID, Pane.Terminal("t1").id),
            folded.workspaces.single().tabs.map { it.id },
        )
    }

    /** A workspace the runner has not described yet still has its diff. */
    @Test
    fun theDiffStandsOnItsOwnBeforeTheRunnerAnswers() {
        val empty = trackFleet(null, "w", "laptop")
        assertEquals(listOf(Pane.CHANGES_ID), empty.workspaces.single().tabs.map { it.id })
        assertNotNull(empty.position(Pane.CHANGES_ID))
    }

    // ---- Which panes are drawn ----

    @Test
    fun theCurrentPaneIsTheMiddleAndItsNeighboursAreEitherSide() {
        val here = fleet.position(Pane.Terminal("t1").id)!!
        assertEquals(0, trackSlot(fleet, here, Pane.Terminal("t1")))
        assertEquals(-1, trackSlot(fleet, here, Pane.Changes))
        assertEquals(1, trackSlot(fleet, here, Pane.Terminal("t2")))
    }

    /**
     * At the ends there is no neighbour that way, and no pane may claim the
     * empty slot. A pane drawn at −1 with nothing behind it would slide in from
     * the left on a drag that is supposed to rubber-band.
     */
    @Test
    fun theEndsOfTheSequenceHaveOneNeighbourEach() {
        val first = fleet.position(Pane.CHANGES_ID)!!
        assertEquals(0, trackSlot(fleet, first, Pane.Changes))
        assertEquals(1, trackSlot(fleet, first, Pane.Terminal("t1")))
        assertNull("nothing is to the left of the diff",
            fleet.step(first, ShellDirection.PREVIOUS, ShellTrack.CONTENT))

        val last = fleet.position(Pane.Terminal("t2").id)!!
        assertEquals(-1, trackSlot(fleet, last, Pane.Terminal("t1")))
        assertTrue(fleet.rubberBands(last, ShellDirection.NEXT, ShellTrack.CONTENT))
    }

    /**
     * A mounted pane that is not a neighbour has no slot, and is therefore not
     * drawn — while staying composed, alive and holding its scrollback. This is
     * the difference between "hidden" and "gone" stated at the track level.
     */
    @Test
    fun aMountedPaneThatIsNotANeighbourIsNotDrawn() {
        val wide = trackFleet(
            workspace(terminal("t1"), terminal("t2"), terminal("t3"), terminal("t4")),
            "w", "laptop",
        )
        val here = wide.position(Pane.Terminal("t1").id)!!
        assertNull(trackSlot(wide, here, Pane.Terminal("t3")))
        assertNull(trackSlot(wide, here, Pane.Terminal("t4")))
    }

    /** A pane the workspace no longer has is on no slot rather than on slot 0. */
    @Test
    fun aPaneTheWorkspaceHasLostIsOnNoSlot() {
        val here = fleet.position(Pane.Terminal("t1").id)!!
        assertNull(trackSlot(fleet, here, Pane.Terminal("gone")))
    }

    // ---- The join with the deck ----

    /**
     * **Every pane the track wants to draw is one the deck is holding**, and it
     * still is after walking the whole sequence.
     *
     * This is the reason `PaneDeck.MOUNT_LIMIT` went to five. If it were ever
     * too small, a swipe would ask for a neighbour that had just been evicted —
     * which is a terminal destroyed and rebuilt to animate a page turn, the
     * exact loss the deck exists to prevent. Asserted by walking rather than by
     * arithmetic, because the arithmetic is what would be wrong.
     */
    @Test
    fun walkingTheWholeSequenceNeverAsksForAnEvictedPane() {
        val wide = trackFleet(
            workspace(terminal("t1"), terminal("t2"), terminal("t3"), terminal("t4")),
            "w", "laptop",
        )
        val order = wide.workspaces.single().tabs.map { Pane.parse(it.id) }

        var deck = PaneDeck.opening(order.first())
        for (pane in order) {
            deck = deck.select(pane)
            val here = wide.position(pane.id)!!
            // Mount both neighbours, which is what the track draws.
            listOf(ShellDirection.PREVIOUS, ShellDirection.NEXT).forEach { direction ->
                wide.step(here, direction, ShellTrack.CONTENT)?.let { step ->
                    deck = deck.select(Pane.parse(wide.tab(step.position)!!.id))
                }
            }
            // Back to the pane we are actually on — selecting a neighbour moved
            // `current`, and the track is drawn around where you are.
            deck = deck.select(pane)

            for (slotted in order) {
                if (trackSlot(wide, here, slotted) == null) continue
                assertTrue(
                    "at ${pane.id} the track wants ${slotted.id}, which is not mounted",
                    deck.isMounted(slotted),
                )
            }
        }
    }

    /**
     * The sequence reverses exactly, so a swipe back lands where a swipe forward
     * came from. Stated here as well as in `ShellTest` because this is the
     * version over real panes: a mismatch would show up as a page turn that goes
     * somewhere and cannot be undone.
     */
    @Test
    fun swipingForwardAndBackReturnsYouToThePaneYouLeft() {
        val start = fleet.position(Pane.Terminal("t1").id)!!
        val forward = fleet.step(start, ShellDirection.NEXT, ShellTrack.CONTENT)!!
        val back = fleet.step(forward.position, ShellDirection.PREVIOUS, ShellTrack.CONTENT)!!
        assertEquals(start, back.position)
        assertEquals(Pane.Terminal("t1").id, fleet.tab(back.position)!!.id)
    }

    /**
     * Within one workspace nothing crosses anything. The flags exist for the day
     * the fleet handed to the track spans workspaces and runners, and a track
     * that reported a crossing inside one worktree would have the UI announcing
     * a move that did not happen.
     */
    @Test
    fun aSwipeInsideOneWorkspaceCrossesNothing() {
        val here = fleet.position(Pane.Terminal("t1").id)!!
        val step = fleet.step(here, ShellDirection.NEXT, ShellTrack.CONTENT)!!
        assertTrue(!step.crossesWorkspace)
        assertTrue(!step.crossesRunner)
    }
}
