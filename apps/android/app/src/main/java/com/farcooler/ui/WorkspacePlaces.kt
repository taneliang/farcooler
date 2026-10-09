package com.farcooler.ui

/**
 * Where each workspace was left (ov-442).
 *
 * Opening a workspace from the front door or the drawer used to show its tab
 * and nothing over it, however deep into it somebody had been: the open task
 * was closed with the workspace it was over. This keeps, per workspace, the
 * screens that were open over it, and [Backstack.goToWorkspace] puts them back
 * when another workspace is switched to it. The tab already has its own
 * memory ([AppModel.selectTab]).
 *
 * Over the ground only: a pane is a ground route of its own, and a screen
 * naming another workspace is not this one's place.
 */
class WorkspacePlaces(private val store: Store) {
    /** Where the stacks are written, by runner and workspace. */
    interface Store {
        fun get(hostId: String, workspaceId: String): String?

        fun set(hostId: String, workspaceId: String, stack: String)
    }

    /**
     * Keep what [stack] has open over its last workspace. A stack with no
     * workspace (the front door) keeps nothing and changes nothing: leaving a
     * place is not clearing it. Closing the task is, since that is where the
     * workspace was left.
     */
    fun remember(stack: List<Route>) {
        val at = stack.indexOfLast { it is Route.Workspace }
        if (at < 0) return
        val workspace = stack[at] as Route.Workspace
        val over = stack.drop(at + 1).takeWhile { names(it, workspace) }
        store.set(workspace.hostId, workspace.workspaceId, Backstack.encodeStack(over))
    }

    /** The screens to put over that workspace when it is opened, if any were. */
    fun over(hostId: String, workspaceId: String): List<Route> {
        val workspace = Route.Workspace(hostId, workspaceId)
        return Backstack.decodeStack(store.get(hostId, workspaceId))?.filter { names(it, workspace) }
            ?: emptyList()
    }

    private fun names(route: Route, workspace: Route.Workspace): Boolean = when (route) {
        is Route.BoardTask -> route.hostId == workspace.hostId && route.workspaceId == workspace.workspaceId
        is Route.BoardHistory -> route.hostId == workspace.hostId && route.workspaceId == workspace.workspaceId
        is Route.PlanPage -> route.hostId == workspace.hostId && route.workspaceId == workspace.workspaceId
        is Route.TreeLevel -> route.hostId == workspace.hostId && route.workspaceId == workspace.workspaceId
        else -> false
    }
}
