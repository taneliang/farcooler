package com.farcooler.model

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive

// Workspaces as the fleet groups by them: repository, then workspace, then its
// worktrees, with the worktrees no workspace owns in an Unclaimed group last.
//
// The same rule as AgentKit's `WorkspaceGroups.swift`, case for case, so this
// phone, the iPhone and the Mac group one fleet one way. A runner without the
// `workstreams` capability has no workspaces, and gets one implicit workspace
// per repository holding every worktree: the flat layout from before.

/**
 * One workspace: a workstream with a name, a task prefix and a board.
 *
 * Decoded from the objects under the fleet's `workspaces`, which
 * `workspaces_json` in `crates/client` writes (and the CLI shares). Snake_case
 * keys, as that module's are. Every field but [id] is defaulted, so a key a
 * later runner drops costs a field and not the fleet.
 */
@Serializable
data class WorkspaceSummary(
    val id: String,
    val name: String = "",
    @SerialName("task_prefix") val taskPrefix: String = "",
    @SerialName("is_main") val isMain: Boolean = false,
    /** Order within the repository, as the runner stores it. Main is 0. */
    val ordinal: Int = 0,
    /** The repository's id, as a UUID string. */
    val repository: String? = null,
    /** The live orchestrator's terminal id, or null when none is running. */
    val orchestrator: String? = null,
    /**
     * Whether this stands in for a runner without workspaces: one per
     * repository, whose id is the repository's own. Never on the wire.
     */
    @kotlinx.serialization.Transient val isImplicit: Boolean = false,
) {
    /** The workspace to name when reading this board, or null for the whole repository. */
    val boardWorkspace: String? get() = if (isImplicit) null else id

    companion object {
        /**
         * The one workspace a runner without `workstreams` has in a repository.
         * Its id is the repository's, so a board keyed by workspace still has a
         * key, and that runner's board notices (which name no workspace) still
         * reach it through [BoardNotice.touches].
         */
        fun implicit(repository: String) = WorkspaceSummary(
            id = repository,
            name = "Main",
            isMain = true,
            ordinal = 0,
            repository = repository,
            isImplicit = true,
        )
    }
}

/** One workspace's place in the fleet: its orchestrator and its worktrees. */
data class WorkspaceGroup(
    val workspace: WorkspaceSummary,
    /** The orchestrator's terminal id, or null when none is running. */
    val orchestrator: String?,
    /** Worktree ids, in the order the runner listed them. */
    val worktrees: List<String>,
) {
    val id: String get() = workspace.id
}

/** One repository's workspaces, and the worktrees none of them owns. */
data class RepositoryGroups(
    val repository: String,
    /** Main first, then by ordinal. A workspace with no worktrees is still here. */
    val workspaces: List<WorkspaceGroup>,
    /** Worktree ids no listed workspace owns, in runner order. */
    val unclaimed: List<String>,
)

/** A worktree as [WorkspaceGrouping.group] needs it. */
data class WorktreeClaim(val id: String, val workspace: String?)

object WorkspaceGrouping {
    /**
     * Group one repository's worktrees by the workspace that owns each.
     *
     * - [workspaces] are THIS repository's; the caller filters. Empty means a
     *   runner without workspaces: one implicit workspace holds every worktree.
     * - A worktree whose workspace is null is unclaimed. So is one whose
     *   workspace is not listed — deleted since, or in another repository —
     *   rather than being dropped from the list.
     * - [orchestrators] maps a workspace id to its orchestrator's terminal id;
     *   a workspace missing from it falls back to its own `orchestrator`.
     */
    fun group(
        repository: String,
        workspaces: List<WorkspaceSummary>,
        worktrees: List<WorktreeClaim>,
        orchestrators: Map<String, String>,
    ): RepositoryGroups {
        if (workspaces.isEmpty()) {
            val only = WorkspaceSummary.implicit(repository)
            return RepositoryGroups(
                repository = repository,
                workspaces = listOf(WorkspaceGroup(only, null, worktrees.map { it.id })),
                unclaimed = emptyList(),
            )
        }
        // `sortedWith` is stable, so a tie keeps the runner's order.
        val ordered = workspaces.sortedWith(compareBy({ !it.isMain }, { it.ordinal }))
        val known = ordered.map { it.id }.toSet()
        val owned = mutableMapOf<String, MutableList<String>>()
        val unclaimed = mutableListOf<String>()
        for (worktree in worktrees) {
            val workspace = worktree.workspace
            if (workspace != null && workspace in known) {
                owned.getOrPut(workspace) { mutableListOf() } += worktree.id
            } else {
                unclaimed += worktree.id
            }
        }
        return RepositoryGroups(
            repository = repository,
            workspaces = ordered.map {
                WorkspaceGroup(it, orchestrators[it.id] ?: it.orchestrator, owned[it.id].orEmpty())
            },
            unclaimed = unclaimed,
        )
    }

    /**
     * A fleet grouped by repository and then by workspace, repositories in the
     * order their first worktree appears; a repository with workspaces and no
     * worktrees comes after, since it still has boards. The orchestrator is
     * found by its terminal's role.
     */
    fun groups(fleet: Fleet): List<RepositoryGroups> {
        val order = mutableListOf<String>()
        val rows = mutableMapOf<String, MutableList<WorktreeClaim>>()
        for (worktree in fleet.worktrees) {
            val repository = worktree.repository ?: ""
            if (repository !in rows) order += repository
            rows.getOrPut(repository) { mutableListOf() } += WorktreeClaim(worktree.id, worktree.workspace)
        }
        val workspaces = fleet.workspaces.orEmpty()
        for (workspace in workspaces) {
            val repository = workspace.repository ?: ""
            if (repository !in rows) {
                order += repository
                rows[repository] = mutableListOf()
            }
        }
        val orchestrators = mutableMapOf<String, String>()
        for (worktree in fleet.worktrees) {
            for (terminal in worktree.terminals) {
                val workspace = terminal.workspace
                if (terminal.isOrchestrator && workspace != null) orchestrators[workspace] = terminal.id
            }
        }
        return order.map { repository ->
            group(
                repository = repository,
                workspaces = workspaces.filter { it.repository == repository },
                worktrees = rows[repository].orEmpty(),
                orchestrators = orchestrators,
            )
        }
    }
}

/**
 * A board moved: what a `task` notice says, and which boards must re-read.
 *
 * The line is `{"event": "task", "repository", "workspace", "from_workspace",
 * "actor"}` (`event_line` in `crates/client/src/ffi.rs`). A board keyed by
 * workspace re-reads on its own board's news and not on another's.
 */
data class BoardNotice(
    val repository: String,
    /** The board the task is on now; null from a runner without `workstreams`. */
    val workspace: String?,
    /** The board it just left, set only on a move. */
    val fromWorkspace: String? = null,
    val actor: String? = null,
) {
    /**
     * Whether [board] must be read again: the board named and the board left,
     * or — with no workspace named — every board in the repository, which on a
     * runner without workspaces is its one implicit board.
     *
     * An implicit board is the whole repository's, so any task in the
     * repository moves it, whichever workspace the notice names: a runner
     * upgraded under a connected app, whose fleet has not listed its
     * workspaces yet, or a board a route saved before workspaces restored.
     */
    fun touches(board: WorkspaceSummary): Boolean {
        if (board.isImplicit) return board.repository == repository
        val workspace = workspace ?: return board.repository == repository
        return board.id == workspace || board.id == fromWorkspace
    }

    companion object {
        /** Read a notice the client core queued, or null when it is not board news. */
        fun of(notice: JsonObject): BoardNotice? {
            fun word(key: String) = notice[key]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotEmpty() }
            if (word("event") != "task") return null
            val repository = word("repository") ?: return null
            return BoardNotice(repository, word("workspace"), word("from_workspace"), word("actor"))
        }
    }
}
