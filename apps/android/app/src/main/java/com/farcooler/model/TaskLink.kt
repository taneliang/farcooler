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
}
