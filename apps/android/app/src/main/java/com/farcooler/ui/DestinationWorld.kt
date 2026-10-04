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

    /**
     * One [Source] per paired runner: the connected ones as [connected] reads
     * them (a live id over a remembered one), the rest seated idle with the id
     * they said when last connected ([known], by host id), so a push naming
     * one finds it and connects it (ov-231). A runner never connected here has
     * no id, and a tap never dials it to look.
     */
    fun sources(
        paired: List<String>,
        connected: Map<String, Source>,
        known: Map<String, String>,
        everyRunner: Boolean,
        selected: String?,
    ): List<Source> = paired.map { host ->
        connected[host]?.let { it.copy(runnerId = it.runnerId ?: known[host]) }
            ?: Source(
                hostId = host, runnerId = known[host], ready = false,
                idle = !everyRunner && host != selected, fleet = null,
            )
    }

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
