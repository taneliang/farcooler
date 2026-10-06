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
    /** Material's expanded width begins at 840 dp, and a folded foldable's window is below it. */
    @Test
    fun `the wide layout begins at 840 dp`() {
        assertEquals(WorkspaceLayout.Kind.PHONE, WorkspaceLayout.of(411, implicit = false))
        // A folded foldable's cover and inner windows, and a tablet in a narrow split.
        assertEquals(WorkspaceLayout.Kind.PHONE, WorkspaceLayout.of(360, implicit = false))
        assertEquals(WorkspaceLayout.Kind.PHONE, WorkspaceLayout.of(600, implicit = false))
        assertEquals(WorkspaceLayout.Kind.PHONE, WorkspaceLayout.of(839, implicit = false))
        assertEquals(WorkspaceLayout.Kind.WIDE, WorkspaceLayout.of(840, implicit = false))
        assertEquals(WorkspaceLayout.Kind.WIDE, WorkspaceLayout.of(1280, implicit = false))
    }

    /** A runner without workspaces has no orchestrator or plan to put beside a tree, however wide the window. */
    @Test
    fun `an implicit workspace stays the phone's layout at any width`() {
        assertEquals(WorkspaceLayout.Kind.PHONE, WorkspaceLayout.of(1280, implicit = true))
        assertEquals(WorkspaceLayout.Kind.PHONE, WorkspaceLayout.of(840, implicit = true))
    }

    /** Both rail places keep the tree in the list pane and the chat in the supporting pane; only the main pane changes. */
    @Test
    fun `each rail place's panes`() {
        assertEquals(
            mapOf(
                WidePane.LIST to WideContent.TREE,
                WidePane.MAIN to WideContent.PLAN,
                WidePane.SUPPORTING to WideContent.CHAT,
            ),
            WideDestination.PLAN.contents(),
        )
        assertEquals(
            mapOf(
                WidePane.LIST to WideContent.TREE,
                WidePane.MAIN to WideContent.BOARD,
                WidePane.SUPPORTING to WideContent.CHAT,
            ),
            WideDestination.BOARD.contents(),
        )
    }

    /** The chat is always shown: no destination leaves the supporting pane without it. */
    @Test
    fun `the chat is in every destination`() {
        for (destination in WideDestination.entries) {
            assertEquals(WideContent.CHAT, destination.contents().getValue(WidePane.SUPPORTING))
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

    /** The scaffold is given all three panes expanded, and room for three columns. */
    @Test
    fun `the scaffold shows all three panes`() {
        assertEquals(PaneAdaptedValue.Expanded, WideScaffoldValue.primary)
        assertEquals(PaneAdaptedValue.Expanded, WideScaffoldValue.secondary)
        assertEquals(PaneAdaptedValue.Expanded, WideScaffoldValue.tertiary)
        assertEquals(3, WideScaffoldDirective.maxHorizontalPartitions)
        // At the narrowest wide window, less the 80 dp rail and the two columns,
        // the plan still has a column of its own (200 dp, the floor).
        assertTrue(WorkspaceLayout.EXPANDED_DP - 80 - WideListWidth.value - WideChatWidth.value >= 200)
    }
}
