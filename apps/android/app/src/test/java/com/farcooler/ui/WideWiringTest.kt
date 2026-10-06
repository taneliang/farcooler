package com.farcooler.ui

import com.farcooler.model.Terminal
import com.farcooler.model.PlanPage
import com.farcooler.net.TerminalRef
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What `WorkspaceScreen` wires around the wide frame (ov-353 R2-3): the choice
 * of layout from its inputs, the panel's sync on a tab or width change, the
 * close-on-pick wrapper and where a jump goes. The screen calls [WideWiring]
 * for every one of these, so a break there goes red here.
 */
class WideWiringTest {
    private val two = WorkspaceLayout.Kind.TWO_PANE
    private val three = WorkspaceLayout.Kind.THREE_PANE
    private val orchestrator = Terminal(id = "o", state = "running", role = "orchestrator")
    private var focused = 0
    private val wiring = WideWiring(focusChat = { focused++ })

    private fun openPanel() {
        wiring.sync(two, WorkspaceTab.ORCHESTRATOR)
        assertFalse(wiring.panel.planTapped(WideDestination.PLAN, two))
        assertTrue(wiring.panel.isShown(two, WideDestination.PLAN))
    }

    /** The choice from the screen's inputs: its window's width and whether the workspace is implicit. */
    @Test
    fun `the wide choice follows the width and the workspace`() {
        assertEquals(WorkspaceLayout.Kind.PHONE, WorkspaceLayout.of(839, implicit = false))
        assertEquals(two, WorkspaceLayout.of(840, implicit = false))
        assertEquals(three, WorkspaceLayout.of(1200, implicit = false))
        assertEquals(WorkspaceLayout.Kind.PHONE, WorkspaceLayout.of(1280, implicit = true))
    }

    /** A tab changed from outside (a deep link, a push to the Board) closes an open panel. */
    @Test
    fun `sync closes the panel on a tab change`() {
        openPanel()
        wiring.sync(two, WorkspaceTab.BOARD)
        assertFalse(wiring.panel.open)
    }

    /** Orchestrator and Themes are one rail place, so moving between them leaves the panel alone; a width change closes it. */
    @Test
    fun `sync keeps the panel across tabs of one place and closes it on a width change`() {
        openPanel()
        wiring.sync(two, WorkspaceTab.WORKTREES)
        assertTrue("Themes is the plan's place too", wiring.panel.open)
        wiring.sync(three, WorkspaceTab.WORKTREES)
        assertFalse("crossing 1200 dp closes it", wiring.panel.open)
    }

    /** Each way out of the tree closes the panel first, then does what the screen's own navigation does. */
    @Test
    fun `a pick from the tree closes the panel and then navigates`() {
        val calls = mutableListOf<String>()
        fun note(what: String) = calls.add("$what:${if (wiring.panel.open) "open" else "closed"}")
        val nav = wiring.treeNavigation(
            TreeNavigation(
                onOpenTask = { note("task") },
                onOpenPlan = { note("plan") },
                onOpenTerminal = { _, _ -> note("terminal") },
                onOpenWorktree = { note("worktree") },
                onOpenLevel = { note("level") },
            ),
        )
        val ways = listOf<() -> Unit>(
            { nav.onOpenTask("t") },
            { nav.onOpenPlan(PlanPage.Lane("l")) },
            { nav.onOpenTerminal("w", "x") },
            { nav.onOpenWorktree("w") },
            { nav.onOpenLevel("l") },
        )
        for (way in ways) {
            openPanel()
            way()
        }
        assertEquals(listOf("task", "plan", "terminal", "worktree", "level").map { "$it:closed" }, calls)
    }

    /** The orchestrator's own terminal focuses the chat; any other terminal, or no orchestrator, is opened. */
    @Test
    fun `a jump focuses the chat for the orchestrator and opens anything else`() {
        val opened = mutableListOf<TerminalRef>()
        val own = TerminalRef("h", "main", "o")
        val other = TerminalRef("h", "w", "x")
        wiring.jump(own, orchestrator) { opened.add(it) }
        assertEquals(1, focused)
        assertTrue(opened.isEmpty())
        wiring.jump(other, orchestrator) { opened.add(it) }
        wiring.jump(own, null) { opened.add(it) }
        assertEquals(1, focused)
        assertEquals(listOf(other, own), opened)
    }
}
