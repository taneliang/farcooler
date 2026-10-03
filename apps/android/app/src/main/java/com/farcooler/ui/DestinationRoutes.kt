package com.farcooler.ui

import com.farcooler.model.Destination

/**
 * A [Destination] as this phone's stack, and a stack as a [Destination]
 * (ov-182, ov-183): the one adapter between the shared model and [Route].
 *
 * A notification tap, a relaunch and (later) a task link all arrive as a
 * resolved destination, and [stack] lays out the screens for it as
 * [Backstack.chain] always has: the front door, the workspace, the task when
 * there is one, then the pane, so Back walks up that chain.
 */
object DestinationRoutes {
    /** What landing a destination puts on screen. */
    data class Landing(
        val stack: List<Route>,
        /** The pane to point the worktree at, by terminal id; null leaves the rule to choose. */
        val pane: String? = null,
    )

    /**
     * The screens for [destination], which the resolver has already opened:
     * its runner is a seat's, and its place is one that exists.
     *
     * [rememberedTab] is the tab a workspace last showed on this phone, for a
     * workspace opened with no segment named. [taskOf] is the task a pane
     * belongs to, so a pane with a task opens over it.
     */
    fun stack(
        destination: Destination,
        rememberedTab: (hostId: String, workspaceId: String) -> WorkspaceTab? = { _, _ -> null },
        taskOf: (hostId: String, terminalId: String) -> String? = { _, _ -> null },
    ): Landing {
        val host = destination.runner.host ?: return Landing(listOf(Backstack.ROOT))
        return when (val place = destination.place) {
            is Destination.Place.NeedsYou, is Destination.Place.Terminal -> Landing(listOf(Backstack.ROOT))
            is Destination.Place.Workspace -> {
                val tab = destination.segment?.let(::tab) ?: rememberedTab(host, place.id) ?: WorkspaceTab.ORCHESTRATOR
                Landing(Backstack.goToWorkspace(listOf(Backstack.ROOT), Route.Workspace(host, place.id, tab)))
            }
            is Destination.Place.Orchestrator ->
                Landing(Backstack.chain(host, place.workspace, null, null, orchestrator = true))
            is Destination.Place.History ->
                Landing(
                    listOf(
                        Backstack.ROOT,
                        Route.Workspace(host, place.workspace, WorkspaceTab.BOARD),
                        Route.BoardHistory(host, place.workspace, place.status),
                    ),
                )
            is Destination.Place.Task -> {
                val id = place.task.id
                val workspace = place.workspace
                if (id == null || workspace == null) Landing(listOf(Backstack.ROOT))
                else Landing(Backstack.chain(host, workspace, id, null))
            }
            is Destination.Place.Worktree -> {
                val pane = Route.Terminal(host, place.id)
                val task = destination.pane?.let { taskOf(host, it) }
                Landing(Backstack.chain(host, place.workspace, task, pane), destination.pane)
            }
        }
    }

    /** The place routes a saved destination can be made from, skipping what is only over them. */
    private fun placeOf(route: Route): Boolean = when (route) {
        is Route.Workspace, is Route.BoardTask, is Route.BoardHistory, is Route.Terminal, is Route.NeedsYou -> true
        else -> false
    }

    /**
     * Where [stack] is, to be kept for the next launch: its deepest place,
     * with the pane the worktree was showing when [paneOf] knows it. A stack
     * with settings or a sign-in over it is where it was under them.
     * Null for a stack that's only onboarding, which is no place to return to.
     */
    fun destination(stack: List<Route>, paneOf: (hostId: String, worktreeId: String) -> String? = { _, _ -> null }): Destination? {
        val at = stack.indexOfLast(::placeOf)
        if (at < 0) return null
        fun workspaceBefore(index: Int, host: String): String? =
            stack.take(index).lastOrNull { it is Route.Workspace && it.hostId == host }
                ?.let { (it as Route.Workspace).workspaceId }
        return when (val route = stack[at]) {
            is Route.NeedsYou -> Destination.NEEDS_YOU
            is Route.Workspace -> Destination(
                runner = Destination.Runner(host = route.hostId),
                place = Destination.Place.Workspace(route.workspaceId),
                segment = segment(route.tab),
            )
            is Route.BoardTask -> Destination(
                runner = Destination.Runner(host = route.hostId),
                place = Destination.Place.Task(route.workspaceId, Destination.TaskRef(id = route.taskId)),
            )
            is Route.BoardHistory -> Destination(
                runner = Destination.Runner(host = route.hostId),
                place = Destination.Place.History(route.workspaceId, route.status),
            )
            is Route.Terminal -> Destination(
                runner = Destination.Runner(host = route.hostId),
                place = Destination.Place.Worktree(route.worktreeId, workspaceBefore(at, route.hostId)),
                pane = paneOf(route.hostId, route.worktreeId),
            )
            else -> null
        }
    }

    private fun tab(segment: Destination.Segment): WorkspaceTab = when (segment) {
        Destination.Segment.ORCHESTRATOR -> WorkspaceTab.ORCHESTRATOR
        Destination.Segment.BOARD -> WorkspaceTab.BOARD
        Destination.Segment.WORKTREES -> WorkspaceTab.WORKTREES
    }

    internal fun segment(tab: WorkspaceTab): Destination.Segment = when (tab) {
        WorkspaceTab.ORCHESTRATOR -> Destination.Segment.ORCHESTRATOR
        WorkspaceTab.BOARD -> Destination.Segment.BOARD
        WorkspaceTab.WORKTREES -> Destination.Segment.WORKTREES
    }
}
