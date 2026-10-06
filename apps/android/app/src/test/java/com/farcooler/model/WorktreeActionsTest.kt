package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * A worktree's menu (ov-300): what the Worktrees tab's header offered, which
 * the One tree's rows offer now that the tree has taken the tab's place. One
 * rule, `WorktreeActions.of`, read by both.
 */
class WorktreeActionsTest {
    private val lane = Worktree(id = "w1", repository = "repo", task = "mac-vis", branch = "mac-vis", state = "ready", ordinal = 1)

    @Test
    fun `a worktree offers a new terminal, its stack, hiding and removal, as the Worktrees tab did`() {
        assertEquals(
            listOf(WorktreeAction.NEW_TERMINAL, WorktreeAction.STACK, WorktreeAction.HIDE, WorktreeAction.REMOVE),
            WorktreeActions.of(lane),
        )
    }

    @Test
    fun `a hidden one offers Unhide, the checkout never offers Remove, and no stack without a repository`() {
        assertEquals(WorktreeAction.UNHIDE, WorktreeActions.of(lane.copy(state = "hidden"))[2])
        assertEquals(false, WorktreeAction.REMOVE in WorktreeActions.of(lane.copy(isMainCheckout = true)))
        assertEquals(false, WorktreeAction.STACK in WorktreeActions.of(lane.copy(repository = null)))
    }

    @Test
    fun `the checkout hides only from the workspace that owns it`() {
        val checkout = lane.copy(isMainCheckout = true, workspace = "main")
        assertEquals(true, WorktreeAction.HIDE in WorktreeActions.of(checkout, workspace = "main"))
        assertEquals(false, WorktreeAction.HIDE in WorktreeActions.of(checkout, workspace = "billing"))
        assertEquals(true, WorktreeAction.HIDE in WorktreeActions.of(lane.copy(workspace = "main"), workspace = "billing"))
    }

    @Test
    fun `it moves only where the runner keeps an order and there is a row to pass`() {
        assertEquals(
            listOf(WorktreeAction.NEW_TERMINAL, WorktreeAction.STACK, WorktreeAction.MOVE_UP, WorktreeAction.MOVE_DOWN, WorktreeAction.HIDE, WorktreeAction.REMOVE),
            WorktreeActions.of(lane, above = "w0", below = "w2"),
        )
        assertEquals(listOf(WorktreeAction.MOVE_DOWN), WorktreeActions.of(lane, below = "w2").filter { it.name.startsWith("MOVE") })
        assertEquals(emptyList<WorktreeAction>(), WorktreeActions.of(lane.copy(ordinal = null), "w0", "w2").filter { it.name.startsWith("MOVE") })
    }

    @Test
    fun `a move passes its neighbor in the runner's order`() {
        val order = listOf("a", "w1", "b", "c")
        assertEquals(listOf("w1", "a", "b", "c"), WorktreeActions.moved(order, "w1", "a", up = true))
        assertEquals(listOf("a", "b", "w1", "c"), WorktreeActions.moved(order, "w1", "b", up = false))
        assertEquals(order, WorktreeActions.moved(order, "w1", "gone", up = true))
    }

    @Test
    fun `in the tree, a loose worktree moves among the loose ones, and a lane doesn't move`() {
        val plan = Plan()
        val workspace = WorkspaceSummary(id = "ws", name = "Billing", repository = "repo")
        val worktrees = listOf("x", "y", "z").map {
            Worktree(id = it, repository = "repo", task = it, branch = it, workspace = "ws", state = "ready", ordinal = 0)
        } + Worktree(id = "own", repository = "repo", task = "own", branch = "own", workspace = "ws", state = "ready")
        val board = TaskBoard(listOf(TaskBoardColumn(TaskStatus.IN_PROGRESS, listOf(TaskRow("t1", "ov-1", "One", TaskStatus.IN_PROGRESS, 0L, worktreeId = "own")))))
        val tree = OneTree.build(workspace, board, plan, worktrees, emptyList(), OneTree.Filter.OPEN)
        val loose = tree.below.single { it.id == "group:loose" }.children
        assertEquals(listOf("x", "y", "z"), loose.map { it.worktreeId })
        assertEquals(null to "y", OneTree.neighbors(tree, loose[0].id))
        assertEquals("x" to "z", OneTree.neighbors(tree, loose[1].id))
        val own = tree.all.single { it.target == OneTree.Target.Worktree("own") && it.kind == OneTree.Kind.LANE }
        assertEquals(null to null, OneTree.neighbors(tree, own.id))
        // And the checkout row carries its worktree, for New terminal there.
        val main = OneTree.build(workspace, board, plan, worktrees + Worktree(id = "m", repository = "repo", isMainCheckout = true, state = "ready"), emptyList(), OneTree.Filter.OPEN)
        assertEquals("m", main.below.single { it.id == "group:main" }.worktreeId)
    }
}
