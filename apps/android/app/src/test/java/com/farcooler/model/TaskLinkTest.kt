package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** Which task a pane names (spec §3.2), and that naming one doesn't make a shell its agent. */
class TaskLinkTest {
    private val invoice = TaskRef("t-9", "bil-9", "Invoice PDF export", "in_progress")
    private val retries = TaskRef("t-4", "bil-4", "Retry failed webhooks", "in_review")

    private fun worktree(vararg open: TaskRef, terminals: List<Terminal> = emptyList()) =
        Worktree(id = "w", terminals = terminals, openTasks = open.toList())

    @Test
    fun `a dispatched pane's task is its own, over its worktree's`() {
        val pane = Terminal(id = "p", preset = "claude", state = "running", taskId = "t-4")
        assertEquals(retries, TaskLink.task(pane, worktree(invoice, retries)))
        // Its own even when the worktree has one other open task.
        assertEquals("t-4", TaskLink.taskId(pane, worktree(invoice)))
    }

    @Test
    fun `one open task is the pane's task`() {
        val pane = Terminal(id = "p", preset = "claude", state = "running")
        assertEquals(invoice, TaskLink.task(pane, worktree(invoice)))
        assertEquals("bil-9 Invoice PDF export", TaskLink.task(pane, worktree(invoice))?.label)
    }

    @Test
    fun `an orchestrator shows no task`() {
        val orchestrator = Terminal(id = "o", preset = "claude", state = "running", role = "orchestrator")
        assertNull(TaskLink.task(orchestrator, worktree(invoice)))
        // An agent in the same worktree still shows it.
        assertEquals(invoice, TaskLink.task(orchestrator.copy(role = "agent"), worktree(invoice)))
    }

    @Test
    fun `two open tasks are none`() {
        val pane = Terminal(id = "p", preset = "claude", state = "running")
        assertNull(TaskLink.task(pane, worktree(invoice, retries)))
        assertNull(TaskLink.task(pane, worktree()))
        assertNull(TaskLink.task(pane, null))
    }

    @Test
    fun `a shell under a task is not its agent`() {
        val shell = Terminal(id = "s", preset = "zsh", state = "running")
        val wt = worktree(invoice, terminals = listOf(shell))
        // The chip names the task over the shell...
        assertEquals(invoice, TaskLink.task(shell, wt))
        // ...and the board still counts no agent on it.
        val board = TaskBoard(
            columns = listOf(
                TaskBoardColumn(
                    TaskStatus.IN_PROGRESS,
                    listOf(TaskRow("t-9", "bil-9", "Invoice PDF export", TaskStatus.IN_PROGRESS, 0L, worktreeId = "w")),
                )
            ),
        )
        assertEquals(0, board.tasksWithLiveAgents(wt.terminals))
    }

    // Which task an agent's own notifications fold into (ov-107): the same
    // cases as AgentKit's TaskLinkTests.

    @Test
    fun `an agent's notices fold into the task it was opened for, anywhere`() {
        val pane = Terminal(id = "p", preset = "claude", state = "running", taskId = "t-4")
        assertEquals("t-4", TaskLink.noticeTaskId(pane, worktree(invoice)))
        assertEquals("t-4", TaskLink.noticeTaskId(pane, worktree()))
        assertEquals("t-4", TaskLink.noticeTaskId(pane, worktree(invoice).copy(isMainCheckout = true)))
    }

    @Test
    fun `an agent opened by hand folds into its lane's one open task, never two`() {
        val pane = Terminal(id = "p", preset = "claude", state = "running")
        assertEquals("t-9", TaskLink.noticeTaskId(pane, worktree(invoice)))
        assertEquals("t-9", TaskLink.noticeTaskId(pane.copy(taskId = ""), worktree(invoice)))
        assertNull(TaskLink.noticeTaskId(pane, worktree(invoice, retries)))
        assertNull(TaskLink.noticeTaskId(pane, worktree()))
        assertNull(TaskLink.noticeTaskId(pane, null))
    }

    @Test
    fun `the main checkout is nobody's lane for notices, though its top bar names the task`() {
        val pane = Terminal(id = "p", preset = "claude", state = "running")
        val checkout = worktree(invoice).copy(isMainCheckout = true)
        assertEquals(invoice, TaskLink.task(pane, checkout))
        assertNull(TaskLink.noticeTaskId(pane, checkout))
    }

    @Test
    fun `an orchestrator's notices are its own`() {
        val lead = Terminal(id = "o", preset = "claude", state = "running", role = "orchestrator")
        assertNull(TaskLink.noticeTaskId(lead, worktree(invoice)))
        assertNull(TaskLink.noticeTaskId(lead.copy(taskId = "t-4"), worktree()))
    }

    @Test
    fun `a banner is left to the task only where the task's notice arrives`() {
        val agent = Terminal(id = "p", preset = "claude", state = "running", taskId = "t-4")
        val loose = Terminal(id = "l", preset = "claude", state = "running")
        org.junit.Assert.assertTrue(TaskLink.leavesBannerToTask(agent, worktree(), true))
        org.junit.Assert.assertFalse(TaskLink.leavesBannerToTask(agent, worktree(), false))
        org.junit.Assert.assertFalse(TaskLink.leavesBannerToTask(loose, worktree(), true))
    }

    @Test
    fun `a phone hears a task notice only from a paired runner that sends them, once registered`() {
        fun build(capabilities: Set<String>, paired: Boolean) =
            DaemonBuild("v", true, "linux", capabilities = capabilities, pushPaired = paired)
        val sends = setOf("tasks", "task_notices")
        org.junit.Assert.assertTrue(TaskLink.taskNoticeReachesPhone(build(sends, true), registered = true))
        org.junit.Assert.assertFalse(TaskLink.taskNoticeReachesPhone(build(sends, true), registered = false))
        org.junit.Assert.assertFalse(TaskLink.taskNoticeReachesPhone(build(sends, false), registered = true))
        org.junit.Assert.assertFalse(TaskLink.taskNoticeReachesPhone(build(setOf("tasks"), true), registered = true))
        org.junit.Assert.assertFalse(TaskLink.taskNoticeReachesPhone(null, registered = true))
    }
}
