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
        assertEquals("t-4", TaskLink.noticeTaskId(pane, worktree(invoice, retries)))
        assertEquals("t-4", TaskLink.noticeTaskId(pane, worktree(retries).copy(isMainCheckout = true)))
    }

    @Test
    fun `an agent's own task the phone doesn't know folds nothing`() {
        // The runner folds only into a task it still has; one missing here may
        // be gone, so the banner stays: a duplicate at worst, never silence.
        val pane = Terminal(id = "p", preset = "claude", state = "running", taskId = "t-4")
        assertNull(TaskLink.noticeTaskId(pane, worktree()))
        assertNull(TaskLink.noticeTaskId(pane, worktree(invoice)))
    }

    @Test
    fun `a lane's task takes in an agent only when its workspace can't differ`() {
        // The runner refuses another workspace's task, and `open_tasks`
        // doesn't say whose a task is.
        val pane = Terminal(id = "p", preset = "claude", state = "running", workspace = "ws-b")
        assertNull(TaskLink.noticeTaskId(pane, worktree(invoice)))
        assertEquals("t-9", TaskLink.noticeTaskId(pane.copy(workspace = null), worktree(invoice)))
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
        // An older runner (no `notice_task`): the mirror above decides.
        org.junit.Assert.assertTrue(TaskLink.leavesBannerToTask(agent, worktree(retries), true, false))
        org.junit.Assert.assertFalse(TaskLink.leavesBannerToTask(agent, worktree(retries), false, false))
        org.junit.Assert.assertFalse(TaskLink.leavesBannerToTask(loose, worktree(), true, false))
    }

    // The runner's own answer (`Terminal.noticeTaskId`, ov-112): once a runner
    // sends `notice_task`, the field decides and the mirror is not asked.

    @Test
    fun `a runner that answers decides by the field alone`() {
        val lane = worktree(invoice)
        // The field names a task: fold, though the mirror would not (a pane
        // that names a workspace, in a lane whose task it was not opened for).
        val told = Terminal(id = "p", preset = "claude", state = "running", workspace = "ws-a", noticeTaskId = "t-9")
        assertNull(TaskLink.noticeTaskId(told, lane))
        org.junit.Assert.assertTrue(TaskLink.leavesBannerToTask(told, lane, true, true))
        // No field: the runner said it notifies as itself, though the mirror
        // would fold a pane in a single-task lane outside the main checkout.
        val own = Terminal(id = "p", preset = "claude", state = "running")
        assertEquals("t-9", TaskLink.noticeTaskId(own, lane))
        org.junit.Assert.assertFalse(TaskLink.leavesBannerToTask(own, lane, true, true))
        // An empty id is no id.
        val empty = Terminal(id = "p", preset = "claude", state = "running", taskId = "t-4", noticeTaskId = "")
        org.junit.Assert.assertFalse(TaskLink.leavesBannerToTask(empty, worktree(retries), true, true))
        // The field is read only where the notice arrives at all.
        org.junit.Assert.assertFalse(TaskLink.leavesBannerToTask(told, lane, false, true))
    }

    @Test
    fun `a runner that doesn't answer is not read through the field`() {
        // Absent means "notifies as itself" from a runner that answers and
        // "too old to say" from one that doesn't; only the capability tells them apart.
        val stray = Terminal(id = "p", preset = "claude", state = "running", workspace = "ws-a", noticeTaskId = "t-9")
        org.junit.Assert.assertFalse(TaskLink.leavesBannerToTask(stray, worktree(invoice), true, false))
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

    /** The app's fleet loop: the reports [TaskLink.agentReports] hands the notifier. */
    @Test
    fun `the fleet's reports leave a task-bound agent to its task's push`() {
        val agent = Terminal(id = "a", preset = "claude", state = "running", activity = "blocked", taskId = "t-4")
        val loose = Terminal(id = "l", preset = "claude", state = "running", activity = "blocked")
        val fleet = Fleet(worktrees = listOf(worktree(invoice, retries, terminals = listOf(agent, loose)).copy(task = "lane")))
        val paired = DaemonBuild("v", true, "linux", capabilities = setOf("tasks", "task_notices"), pushPaired = true)
        val reports = TaskLink.agentReports(fleet, paired, registered = true).associateBy { it.terminal.id }
        org.junit.Assert.assertTrue(reports.getValue("a").leftToTask)
        org.junit.Assert.assertFalse(reports.getValue("l").leftToTask)
        assertEquals("lane", reports.getValue("a").worktree)
        org.junit.Assert.assertFalse(TaskLink.agentReports(fleet, null, registered = true).single { it.terminal.id == "a" }.leftToTask)
    }

    /** A runner that sends `notice_task` is believed over this app's mirror, in the fleet loop too. */
    @Test
    fun `the fleet's reports follow the runner's notice task when it sends one`() {
        val told = Terminal(id = "t", preset = "claude", state = "running", activity = "blocked", noticeTaskId = "t-9")
        val own = Terminal(id = "o", preset = "claude", state = "running", activity = "blocked")
        val checkout = worktree(invoice, terminals = listOf(told, own)).copy(task = "lane", isMainCheckout = true)
        val fleet = Fleet(worktrees = listOf(checkout))
        val answers = DaemonBuild(
            "v", true, "linux", capabilities = setOf("tasks", "task_notices", "notice_task"), pushPaired = true,
        )
        val reports = TaskLink.agentReports(fleet, answers, registered = true).associateBy { it.terminal.id }
        org.junit.Assert.assertTrue(reports.getValue("t").leftToTask)
        org.junit.Assert.assertFalse(reports.getValue("o").leftToTask)
    }
}
