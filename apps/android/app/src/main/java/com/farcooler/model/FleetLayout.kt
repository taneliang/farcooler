package com.farcooler.model

// The worktree list's workspace level: a heading per workspace — its Board
// row, its orchestrator, then its worktrees — and an Unclaimed heading at the end of
// each repository for the worktrees no workspace owns.
//
// The list was flat, so this is new layout rather than a regrouping, and a
// runner without `workstreams` keeps the flat list ([FleetLayout.of] answers
// null for it). The grouping is [WorkspaceGrouping]'s; this is what the list
// DRAWS from it, the same rule as AgentKit's `ShellWorkspaces.swift`, so it is
// decided here where a JVM test reads it (`FleetLayoutTest`) and not in a
// composable.

/** One heading in a runner's stretch of the worktree list. */
data class FleetHeading(
    /** The workspace's id, or `unclaimed/<repository id>` for a repository's Unclaimed group. */
    val id: String,
    /** The workspace's name, or "Unclaimed". */
    val name: String,
    /** The repository's display name, when the runner has more than one; null otherwise. */
    val repository: String?,
    val isUnclaimed: Boolean,
    /**
     * The orchestrator's terminal id, drawn as this heading's own row and in no
     * worktree's rows — the daemon opens every orchestrator in its
     * repository's main checkout, which another workspace may own. Null when
     * none is running, when its pane is not in this fleet, and for Unclaimed.
     */
    val orchestrator: String?,
    /** Worktree ids, in the runner's order. */
    val worktrees: List<String>,
) {
    companion object {
        fun unclaimedId(repository: String) = "unclaimed/$repository"
    }
}

object FleetLayout {
    /**
     * [fleet] laid out by workspace, or null for a runner without
     * `workstreams`.
     *
     * - Repositories in [WorkspaceGrouping.groups]' order; within each, Main
     *   first, then by ordinal. Every workspace gets a heading, one with no
     *   worktrees too: it has a board and may have an orchestrator.
     * - A repository's Unclaimed heading comes after its workspaces, and only
     *   when something is unclaimed.
     * - [names] are repository display names by id; a heading names its
     *   repository only when the runner has more than one.
     */
    fun of(fleet: Fleet, names: Map<String, String>): List<FleetHeading>? {
        if (fleet.workspaces == null) return null
        val groups = WorkspaceGrouping.groups(fleet)
        val terminals = fleet.worktrees.flatMap { w -> w.terminals.map { it.id } }.toSet()
        val several = groups.size > 1
        return groups.flatMap { repository ->
            val label = if (several) names[repository.repository] else null
            val headings = repository.workspaces.map { group ->
                FleetHeading(
                    id = group.workspace.id,
                    name = group.workspace.name,
                    repository = label,
                    isUnclaimed = false,
                    orchestrator = group.orchestrator?.takeIf { it in terminals },
                    worktrees = group.worktrees,
                )
            }
            if (repository.unclaimed.isEmpty()) {
                headings
            } else {
                headings + FleetHeading(
                    id = FleetHeading.unclaimedId(repository.repository),
                    name = "Unclaimed",
                    repository = label,
                    isUnclaimed = true,
                    orchestrator = null,
                    worktrees = repository.unclaimed,
                )
            }
        }
    }

    /** Every orchestrator's terminal id: the panes no worktree's rows list. */
    fun orchestrators(headings: List<FleetHeading>?): Set<String> =
        headings.orEmpty().mapNotNull { it.orchestrator }.toSet()

    /**
     * The Board row under each workspace's heading, by heading id, as the
     * iPhone and the Mac draw one: a workspace's own row, where its board has
     * one ([RunnerBoards.rows] leaves out a board not read yet or empty).
     * Unclaimed is no workspace and keeps no board, so it never has one.
     */
    fun boardRows(headings: List<FleetHeading>?, rows: List<BoardRow>): Map<String, BoardRow> {
        val byKey = rows.associateBy { it.key }
        return headings.orEmpty()
            .filterNot { it.isUnclaimed }
            .mapNotNull { heading -> byKey[heading.id]?.let { heading.id to it } }
            .toMap()
    }

    /** An orchestrator's tab title: "Billing Orchestrator", as its row under Billing says. */
    fun orchestratorTitle(workspace: String) = "$workspace Orchestrator"

    /**
     * What each orchestrator's tab is called, by terminal id. Its pane is a
     * tab of the main checkout, which another workspace may own, and a tab
     * called after its program would read as one more of that worktree's
     * terminals.
     */
    fun orchestratorTitles(headings: List<FleetHeading>?): Map<String, String> =
        headings.orEmpty()
            .mapNotNull { heading -> heading.orchestrator?.let { it to orchestratorTitle(heading.name) } }
            .toMap()

    /**
     * What a worktree's rows say when they list no terminal, or null when
     * they list one. The rows leave an orchestrator out, since its heading
     * draws it, but its tab is still in this worktree's strip: a checkout
     * whose only pane is Billing's manager says so, rather than "No
     * terminals" over a strip with a terminal in it.
     */
    fun noTerminalsNote(worktree: Worktree, titles: Map<String, String>): String? {
        if (worktree.terminals.any { it.id !in titles }) return null
        val here = worktree.terminals.mapNotNull { titles[it.id] }
        return when (here.size) {
            0 -> "No terminals"
            1 -> "${here[0]} runs here"
            else -> "${here.size} orchestrators run here"
        }
    }
}
