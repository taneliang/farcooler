package com.farcooler.model

/**
 * Which task a pane shows as its own (spec §3.2, item 2). The same rule as
 * AgentKit's `TaskLink`.
 *
 * - [Terminal.taskId] when it's set: the pane was dispatched for that task.
 * - Otherwise its worktree's task, when [Worktree.openTasks] holds exactly
 *   one. With two there's no telling which the pane is about, so it shows
 *   none rather than a guess.
 *
 * **A display rule only.** The worktree fallback names a task over a person's
 * shell in that task's worktree, and that shell is not the task's agent:
 * [TaskAgentLink.isWorking] reads [Terminal.taskId] alone, and nothing here
 * feeds it or dispatch's one-agent-per-task check.
 */
object TaskLink {
    /**
     * The id of [pane]'s task in [worktree], or null. Never for an
     * orchestrator: it leads the workspace rather than working one task, even
     * in a worktree with exactly one open task (coordinator ruling, ov-55).
     */
    fun taskId(pane: Terminal, worktree: Worktree?): String? {
        if (pane.isOrchestrator) return null
        pane.taskId?.takeIf { it.isNotEmpty() }?.let { return it }
        return worktree?.openTasks?.singleOrNull()?.id
    }

    /**
     * [pane]'s task, named when the worktree's [Worktree.openTasks] names it.
     *
     * A dispatched pane whose task isn't open here — its card is Done, or the
     * runner is too old to send `open_tasks` — comes back with only its id, and
     * [TaskRef.label] is empty. The chip isn't drawn then; see
     * `ui.topBarTaskChip`.
     */
    fun task(pane: Terminal, worktree: Worktree?): TaskRef? {
        val id = taskId(pane, worktree) ?: return null
        return worktree?.openTasks?.firstOrNull { it.id == id } ?: TaskRef(id = id)
    }

    /**
     * The id of the task [pane]'s own notifications fold into, or null when
     * it notifies as itself (ov-94, ov-107). Not [taskId]: that names a top
     * bar, and this decides whether an agent's banner is left to its task's
     * notice, so it gives the runner's answer (`task_link::task_of`). The same
     * cases as AgentKit's `TaskLink.noticeTask`.
     *
     * - Never an orchestrator's.
     * - The task it was opened for, when it was opened for one.
     * - Otherwise its lane's one open task; with two, none.
     * - Never by lane in the repository's main checkout, where ad hoc agents
     *   run and one dispatched task would take in all of them. The runner's
     *   `task_of` still folds there (ov-107 report), so an agent there gets
     *   both banners until it doesn't, never neither.
     */
    fun noticeTaskId(pane: Terminal, worktree: Worktree?): String? {
        if (pane.isOrchestrator) return null
        pane.taskId?.takeIf { it.isNotEmpty() }?.let { return it }
        if (worktree == null || worktree.isMainCheckout) return null
        return worktree.openTasks.singleOrNull()?.id
    }

    /**
     * Whether [pane]'s own banner is left to its task's notice: it folds into
     * a task, and that task's notice reaches this phone ([noticeReachesHere],
     * from [taskNoticeReachesPhone]). Otherwise its own banner is all there is.
     */
    fun leavesBannerToTask(pane: Terminal, worktree: Worktree?, noticeReachesHere: Boolean): Boolean =
        noticeReachesHere && noticeTaskId(pane, worktree) != null

    /**
     * Whether a task notice from the runner [daemon] describes reaches this
     * phone, which hears one only as a push: the runner composes them
     * (`task_notices`), is paired with the relay ([DaemonBuild.pushPaired]),
     * and this phone is [registered] with the relay.
     */
    fun taskNoticeReachesPhone(daemon: DaemonBuild?, registered: Boolean): Boolean =
        daemon != null && daemon.can("task_notices") && daemon.pushPaired && registered

    /**
     * Every pane in [fleet] as [com.farcooler.notify.Notifier.report] takes
     * it, read from the runner [daemon] describes (ov-107). The fold is the
     * one decision in the app's fleet loop, so it is made here, where a test
     * reaches it, and the loop only hands each report on.
     */
    fun agentReports(fleet: Fleet, daemon: DaemonBuild?, registered: Boolean): List<AgentReport> {
        val reaches = taskNoticeReachesPhone(daemon, registered)
        return fleet.worktrees.flatMap { worktree ->
            worktree.terminals.map { terminal ->
                AgentReport(terminal, worktree.task, leavesBannerToTask(terminal, worktree, reaches))
            }
        }
    }
}

/**
 * One pane's change, as the local notifier hears it: the pane, its
 * worktree's name for the body, and whether its banner is left to its task's
 * push (ov-107). See [TaskLink.agentReports].
 */
data class AgentReport(val terminal: Terminal, val worktree: String, val leftToTask: Boolean)

