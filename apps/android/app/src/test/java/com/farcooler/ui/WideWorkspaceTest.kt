package com.farcooler.ui

import androidx.compose.material3.adaptive.ExperimentalMaterial3AdaptiveApi
import androidx.compose.material3.adaptive.layout.PaneAdaptedValue
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The wide workspace's rules (ov-347): which windows get it, what each pane
 * holds at each rail place, and that the scaffold shows all three panes. The
 * composables draw these; the JVM holds them.
 */
@OptIn(ExperimentalMaterial3AdaptiveApi::class)
class WideWorkspaceTest {
    private val phone = WorkspaceLayout.Kind.PHONE
    private val two = WorkspaceLayout.Kind.TWO_PANE
    private val three = WorkspaceLayout.Kind.THREE_PANE

    /**
     * Phone below 840 dp (a folded foldable is there), rail, plan and chat
     * from 840 to 1199, and the tree as a column from 1200.
     */
    @Test
    fun `the breakpoints are 840 and 1200 dp`() {
        for ((width, kind) in listOf(360 to phone, 411 to phone, 600 to phone, 839 to phone, 840 to two, 1000 to two, 1199 to two, 1200 to three, 1280 to three)) {
            assertEquals("$width dp", kind, WorkspaceLayout.of(width, implicit = false))
        }
    }

    /** A runner without workspaces has no orchestrator or plan to put beside a tree, however wide the window. */
    @Test
    fun `an implicit workspace stays the phone's layout at any width`() {
        for (width in listOf(840, 1199, 1200, 1280)) assertEquals(phone, WorkspaceLayout.of(width, implicit = true))
    }

    /** Owner ruling: the plan is never given less than 360 dp, at the narrowest width of each layout. */
    @Test
    fun `the plan keeps 360 dp at each layout's narrowest width`() {
        val rail = WideRailDp
        val chat = WideChatWidth.value.toInt()
        val list = WideListWidth.value.toInt()
        val twoPane = WorkspaceLayout.EXPANDED_DP - rail - chat
        val threePane = WorkspaceLayout.THREE_PANE_DP - rail - chat - list
        assertTrue("two panes: $twoPane", twoPane >= WorkspaceLayout.MIN_PLAN_DP)
        assertTrue("three panes: $threePane", threePane >= WorkspaceLayout.MIN_PLAN_DP)
        // And the tree as a column at 840 dp would have broken it: why it folds into the rail.
        assertTrue(twoPane - list < WorkspaceLayout.MIN_PLAN_DP)
    }

    /** Three columns: tree, plan and chat; the board swaps only the main pane. */
    @Test
    fun `each rail place's panes at three columns`() {
        assertEquals(
            mapOf(WidePane.LIST to WideContent.TREE, WidePane.MAIN to WideContent.PLAN, WidePane.SUPPORTING to WideContent.CHAT),
            WideDestination.PLAN.contents(three),
        )
        assertEquals(
            mapOf(WidePane.LIST to WideContent.TREE, WidePane.MAIN to WideContent.BOARD, WidePane.SUPPORTING to WideContent.CHAT),
            WideDestination.BOARD.contents(three),
        )
    }

    /** Two panes: the plan and the chat, and the tree only once the rail's Plan place has opened it. */
    @Test
    fun `the tree is folded into the plan place at two panes`() {
        assertEquals(
            mapOf(WidePane.MAIN to WideContent.PLAN, WidePane.SUPPORTING to WideContent.CHAT),
            WideDestination.PLAN.contents(two, treeOpen = false),
        )
        assertEquals(
            mapOf(WidePane.LIST to WideContent.TREE, WidePane.MAIN to WideContent.PLAN, WidePane.SUPPORTING to WideContent.CHAT),
            WideDestination.PLAN.contents(two, treeOpen = true),
        )
        assertEquals(WideContent.BOARD, WideDestination.BOARD.contents(two).getValue(WidePane.MAIN))
    }

    /** The phone has no panes. */
    @Test
    fun `the phone has no wide panes`() {
        for (destination in WideDestination.entries) assertTrue(destination.contents(phone, treeOpen = true).isEmpty())
    }

    /** The chat is always shown: every destination, every wide layout, open tree or not. */
    @Test
    fun `the chat is in every wide layout`() {
        for (destination in WideDestination.entries) for (kind in listOf(two, three)) for (open in listOf(false, true)) {
            assertEquals(WideContent.CHAT, destination.contents(kind, open).getValue(WidePane.SUPPORTING))
        }
    }

    /**
     * A saved or pushed tab lands somewhere on a wide screen: Orchestrator and
     * Themes are both the plan's place, since the chat and the tree are always
     * up, and Board is its own.
     */
    @Test
    fun `every tab lands on a rail place`() {
        assertEquals(WideDestination.PLAN, WideDestination.of(WorkspaceTab.ORCHESTRATOR))
        assertEquals(WideDestination.PLAN, WideDestination.of(WorkspaceTab.WORKTREES))
        assertEquals(WideDestination.BOARD, WideDestination.of(WorkspaceTab.BOARD))
        // And what a rail tap stores comes back to the same place.
        for (destination in WideDestination.entries) {
            assertEquals(destination, WideDestination.of(destination.tab))
        }
    }

    /** The scaffold shows the list pane only as a column, and the partitions match the columns. */
    @Test
    fun `the scaffold's panes follow the layout`() {
        val columns = wideScaffoldValue(three)
        assertEquals(PaneAdaptedValue.Expanded, columns.primary)
        assertEquals(PaneAdaptedValue.Expanded, columns.secondary)
        assertEquals(PaneAdaptedValue.Expanded, columns.tertiary)
        assertEquals(3, wideScaffoldDirective(three).maxHorizontalPartitions)
        val folded = wideScaffoldValue(two)
        assertEquals(PaneAdaptedValue.Expanded, folded.primary)
        assertEquals(PaneAdaptedValue.Hidden, folded.secondary)
        assertEquals(PaneAdaptedValue.Expanded, folded.tertiary)
        assertEquals(2, wideScaffoldDirective(two).maxHorizontalPartitions)
    }
}
