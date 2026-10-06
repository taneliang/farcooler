package com.farcooler.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The tree panel's rules at two panes (ov-347 review): it opens from the rail's
 * Plan place, and closes on Back, on a pick, on a rail or layout change, and
 * never shows where the tree is a column. The frame draws these; the JVM holds them.
 */
class TreePanelStateTest {
    private val two = WorkspaceLayout.Kind.TWO_PANE
    private val three = WorkspaceLayout.Kind.THREE_PANE
    private val phone = WorkspaceLayout.Kind.PHONE

    private fun opened() = TreePanelState().also {
        it.sync(two, WideDestination.PLAN)
        assertFalse("tapping the place that is up opens the tree, it doesn't reselect", it.planTapped(WideDestination.PLAN, two))
    }

    /** Tapping Plan while it is up toggles the panel; from the board it selects Plan and leaves the panel shut. */
    @Test
    fun `the plan place opens the tree`() {
        val panel = opened()
        assertTrue(panel.isShown(two))
        assertFalse(panel.planTapped(WideDestination.PLAN, two))
        assertFalse("a second tap closes it", panel.isShown(two))
        assertTrue("from the board it selects", panel.planTapped(WideDestination.BOARD, two))
        assertFalse(panel.isShown(two))
    }

    /** Back dismisses the panel before it leaves the workspace; with none open it isn't handled. */
    @Test
    fun `back closes the panel first`() {
        val panel = opened()
        assertTrue(panel.back(two))
        assertFalse(panel.isShown(two))
        assertFalse("with nothing open, back is the screen's", panel.back(two))
    }

    /** A pick from the panel closes it, so Back from the pushed screen returns to the plan. */
    @Test
    fun `a pick closes the panel`() {
        val panel = opened()
        panel.picked()
        assertFalse(panel.isShown(two))
    }

    /** A rail place changed from outside, a deep link or a push to the board, closes it. */
    @Test
    fun `a tab change closes the panel`() {
        val panel = opened()
        panel.sync(two, WideDestination.BOARD)
        assertFalse(panel.isShown(two))
        // And an unchanged sync leaves an open panel alone.
        val again = opened()
        again.sync(two, WideDestination.PLAN)
        assertTrue(again.isShown(two))
    }

    /** Crossing a breakpoint closes it, and a stale flag never shows it where it can't be: three columns or the phone. */
    @Test
    fun `a width change closes the panel`() {
        val panel = opened()
        assertFalse("never shown as a column layout", panel.isShown(three))
        assertFalse(panel.isShown(phone))
        panel.sync(three, WideDestination.PLAN)
        assertFalse("closed when it crossed 1200 dp", panel.open)
        // Back at two panes it is still shut.
        panel.sync(two, WideDestination.PLAN)
        assertFalse(panel.isShown(two))
    }

    /** Where the tree is a column, the Plan place just selects. */
    @Test
    fun `three columns never open the panel`() {
        val panel = TreePanelState()
        assertTrue(panel.planTapped(WideDestination.PLAN, three))
        assertFalse(panel.open)
    }
}
