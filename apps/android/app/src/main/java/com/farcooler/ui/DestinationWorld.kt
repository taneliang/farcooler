package com.farcooler.ui

import com.farcooler.model.DestinationResolver.World
import com.farcooler.model.Fleet
import com.farcooler.model.TaskBoard
import com.farcooler.model.WorkspaceSummary

/**
 * What this phone holds, as the resolver reads it (ov-182, ov-183): one
 * [Seat] per runner, built from plain values so a test builds the same
 * thing a connection does.
 */
object DestinationWorld {
    /** One runner's state, as [AppModel] reads it off a connection. */
    data class Source(
        val hostId: String,
        /** Its `Host.runner_id`, once its daemon build has been read. */
        val runnerId: String?,
        /** Connected, and its daemon build read: [runnerId] is what it says. */
        val ready: Boolean,
        /** Configured, and no connection for it: nothing is dialing it. */
        val idle: Boolean,
        /** Its fleet, once read. */
        val fleet: Fleet?,
        /** Its boards, each repository's workspaces or implicit ones: empty until its repositories are read. */
        val boardList: List<WorkspaceSummary> = emptyList(),
        /** Boards read so far, by workspace id. */
        val boards: Map<String, TaskBoard> = emptyMap(),
    )

    fun world(sources: List<Source>, lastWorkspace: Pair<String, String>? = null): World = World(
        seats = sources.map(::seat),
        lastWorkspace = lastWorkspace?.let { World.Last(it.first, it.second) },
    )

    private fun seat(source: Source): World.Seat {
        val fleet = source.fleet.takeIf { source.ready }
        return World.Seat(
            host = source.hostId,
            runnerId = source.runnerId,
            ready = source.ready,
            idle = source.idle && !source.ready,
            // Not read is not empty: an empty list of boards is a claim that
            // none exists, and a runner whose repositories haven't come yet
            // has made no such claim.
            workspaces = source.boardList.takeIf { fleet != null && it.isNotEmpty() }
                ?.map { World.Workspace(it.id, orchestrator = it.orchestrator != null) },
            worktrees = fleet?.let { read ->
                read.worktrees.map { worktree ->
                    val orchestrator = worktree.terminals.firstOrNull { it.isOrchestrator }
                    World.Worktree(
                        id = worktree.id,
                        workspace = worktree.workspace?.ifEmpty { null }
                            ?: orchestrator?.workspace?.ifEmpty { null }
                            ?: if (read.workspaces == null) worktree.repository else null,
                        terminals = worktree.terminals.map { World.Terminal(it.id, it.isOrchestrator) },
                    )
                }
            },
            boards = source.boards.mapValues { (_, board) -> board.rows.map { World.Task(id = it.id, key = it.key) } },
        )
    }
}
