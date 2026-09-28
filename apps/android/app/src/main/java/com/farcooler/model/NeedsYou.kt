package com.farcooler.model

/**
 * The front door: what needs a person, across every runner this phone is
 * connected to, and the workspaces under it (spec §6).
 *
 * A rendering of the rollup, and nothing more. Each runner's daemon decides
 * what an item is — a held ask, a blocked agent, a task in Needs Decision, a
 * task In Review (spec §2.2) — and `NeedsYouItems` decodes and merges them.
 * This file labels each item with its workspace, says what the front door may
 * claim when there are none, and lays out the Workspaces list beneath. Pure,
 * so `NeedsYouTest` can prove all of it without a device.
 *
 * It used to derive its own list from the fleet: a section per worktree, its
 * blocked and finished agents, its unread diff, and Board rows with decisions
 * beneath. Ruling 1 retired the finished agents and the diffs, a decision is
 * an item now, and the unit is the item rather than the worktree. What's left
 * of that model is the one rule it had right: every runner at once, merged by
 * rank, because an ask on a runner in another room is as urgent as one on
 * this desk.
 */

/**
 * One runner's reading: its items, and whether they were derived on this
 * phone from a runner too old to compute them ([NeedsYouItems.derived]).
 */
data class RunnerNeedsYou(val items: List<NeedsYouItem>, val derived: Boolean = false)

/** What the front door knows about one runner. */
data class NeedsYouRunner(
    val hostId: String,
    val label: String,
    /** Null until this connection has read anything. */
    val reading: RunnerNeedsYou?,
    val fleet: Fleet = Fleet.EMPTY,
    val repositories: List<Repository> = emptyList(),
)

/** One row on the front door: an item, where it is, and whose runner. */
data class NeedsYouRow(
    val entry: RunnerNeedsYouItem,
    /** The workspace's name, the repository's for an implicit one, or "Unclaimed". */
    val place: String,
    /** The runner's name, only when more than one runner is connected. */
    val runner: String?,
) {
    val item: NeedsYouItem get() = entry.item
    val hostId: String get() = entry.hostId
    val key: String get() = entry.key
}

/** A workspace's row in the Workspaces list. */
data class WorkspaceRow(
    val hostId: String,
    val workspace: WorkspaceSummary,
    /** Its name; an implicit workspace's is its repository's. */
    val name: String,
    /** The orchestrator's terminal, or null when none is running. */
    val orchestrator: Terminal?,
    /** Its items: what the row's count says. */
    val count: Int,
) {
    val key: String get() = "$hostId/${workspace.id}"
}

/** One repository's workspaces, then its Unclaimed and Hidden worktrees. */
data class RepositoryWorkspaces(
    val hostId: String,
    val repository: String,
    val name: String,
    val workspaces: List<WorkspaceRow>,
    /** Worktree ids no workspace owns, not hidden. */
    val unclaimed: List<String>,
    /** The items no listed workspace holds: the Unclaimed row's count. */
    val unclaimedCount: Int,
    /** Hidden worktree ids, whoever owns them. */
    val hidden: List<String>,
) {
    val key: String get() = "$hostId/$repository"
}

object NeedsYou {
    /**
     * Every runner's items, most urgent first, each labeled by workspace
     * (spec §6.2: "each item is a row labeled with its workspace").
     *
     * By rank across runners, which is safe because a rank is a duration and
     * not a clock reading; see [NeedsYouItems.merge]. A runner that hasn't
     * answered adds nothing yet, and nothing is invented for it.
     */
    fun rows(runners: List<NeedsYouRunner>): List<NeedsYouRow> {
        val byHost = runners.associateBy { it.hostId }
        val namesRunners = runners.size > 1
        return NeedsYouItems.merge(runners.associate { it.hostId to it.reading?.items.orEmpty() })
            .map { entry ->
                val runner = byHost.getValue(entry.hostId)
                NeedsYouRow(entry, place(entry.item, runner), if (namesRunners) runner.label else null)
            }
    }

    /**
     * Where an item is, in the words its workspace row uses: the name the
     * runner sent, else the name this phone has for its workspace, else — a
     * runner without workspaces — the repository's, else "Unclaimed".
     */
    fun place(item: NeedsYouItem, runner: NeedsYouRunner): String {
        item.workspaceName.takeIf { it.isNotBlank() }?.let { return it }
        val workspaces = runner.fleet.workspaces
        item.workspaceId?.let { id -> workspaces?.firstOrNull { it.id == id }?.name?.takeIf { it.isNotBlank() } }
            ?.let { return it }
        if (workspaces == null) {
            repositoryName(item.repositoryId ?: item.worktree?.let { w -> repositoryOf(w.id, runner.fleet) }, runner)
                ?.let { return it }
        }
        return UNCLAIMED
    }

    /**
     * Whether the front door may say "Nothing needs you". Only with no item
     * at all, so a decision is never under that sentence the way the old
     * Board rows were (ov-56).
     */
    fun nothingNeedsYou(rows: List<NeedsYouRow>): Boolean = rows.isEmpty()

    /**
     * The runners whose items were derived on this phone, by name: each gets
     * [olderRunnerNote], because what they can't send — asks, decisions,
     * reviews — may be waiting anyway (spec §2.6).
     */
    fun olderRunners(runners: List<NeedsYouRunner>): List<String> =
        runners.filter { it.reading?.derived == true }.map { it.label }

    /**
     * An older runner's reading, derived from its fleet: its blocked agents
     * and nothing else (spec §2.6). A finished agent is not an item (ruling
     * 1), on this path as on the daemon's.
     */
    fun derivedReading(fleet: Fleet): RunnerNeedsYou =
        RunnerNeedsYou(NeedsYouItems.derived(fleet.worktrees), derived = true)

    fun olderRunnerNote(runner: String): String =
        "Update Far Cooler on $runner to see decisions and asks here."

    /**
     * One runner's Workspaces list: each repository's workspaces, Main first,
     * with their orchestrators and counts, then its Unclaimed and Hidden
     * worktrees (spec §6). A workspace is listed whether or not it has
     * anything in it: an empty Main is still somewhere to start (spec §8).
     *
     * A runner without workspaces lists one implicit workspace per
     * repository, named for the repository, holding every worktree.
     */
    fun workspaces(runner: NeedsYouRunner, items: List<RunnerNeedsYouItem>): List<RepositoryWorkspaces> {
        val fleet = runner.fleet
        val mine = items.filter { it.hostId == runner.hostId }.map { it.item }
        val groups = WorkspaceGrouping.groups(fleet).toMutableList()
        // A repository the runner has registered and nothing lives in yet
        // still has its board, and so its row.
        for (repository in runner.repositories) {
            if (groups.none { it.repository == repository.id }) {
                groups += WorkspaceGrouping.group(repository.id, emptyList(), emptyList(), emptyMap())
            }
        }
        val terminals = fleet.worktrees.flatMap { it.terminals }.associateBy { it.id }
        val hiddenIds = fleet.worktrees.filter { it.isHidden }.map { it.id }.toSet()
        return groups.filter { it.repository.isNotEmpty() || it.workspaces.isNotEmpty() }.map { group ->
            val repository = group.repository
            val listed = group.workspaces.map { it.id }.toSet()
            val inRepository = mine.filter { it.repositoryId == repository }
            RepositoryWorkspaces(
                hostId = runner.hostId,
                repository = repository,
                name = repositoryName(repository, runner) ?: repository.take(8),
                workspaces = group.workspaces.map { workspace ->
                    val summary = workspace.workspace
                    WorkspaceRow(
                        hostId = runner.hostId,
                        workspace = summary,
                        name = if (summary.isImplicit) repositoryName(repository, runner) ?: summary.name
                        else summary.name,
                        orchestrator = workspace.orchestrator?.let(terminals::get),
                        count = if (summary.isImplicit) inRepository.size
                        else inRepository.count { it.workspaceId == summary.id },
                    )
                },
                unclaimed = group.unclaimed.filter { it !in hiddenIds },
                unclaimedCount = if (group.workspaces.any { it.workspace.isImplicit }) 0
                else inRepository.count { it.workspaceId == null || it.workspaceId !in listed },
                hidden = fleet.worktrees.filter { it.isHidden && it.repository == repository }.map { it.id },
            )
        }
    }

    /**
     * A workspace's worktrees as its Worktrees tab lists them: the ones it
     * owns, in runner order, hidden ones left to the Hidden row. An implicit
     * workspace owns every worktree in its repository.
     */
    fun worktreesOf(workspace: WorkspaceSummary, fleet: Fleet): List<Worktree> =
        fleet.worktrees.filter { worktree ->
            !worktree.isHidden && if (workspace.isImplicit) worktree.repository == workspace.repository
            else worktree.workspace == workspace.id
        }

    const val UNCLAIMED = "Unclaimed"

    private fun repositoryName(repository: String?, runner: NeedsYouRunner): String? =
        runner.repositories.firstOrNull { it.id == repository }?.let { it.displayName.ifEmpty { it.short } }
            ?.takeIf { it.isNotBlank() }

    private fun repositoryOf(worktree: String, fleet: Fleet): String? =
        fleet.worktrees.firstOrNull { it.id == worktree }?.repository
}

/** What a row offers, in the order it draws them (spec §2.5). */
sealed interface NeedsYouButton {
    /** Sends [action]: an ask's option, or a decision's option as the answer. */
    data class Answer(val action: NeedsYouAction) : NeedsYouButton

    /** A decision's options past the third, behind one "More" menu. */
    data class More(val actions: List<NeedsYouAction>) : NeedsYouButton

    /** A decision with no options: the answer is typed. "Answer…" */
    data object Write : NeedsYouButton

    /** Goes to it and sends nothing: "Open", or "Review" for a review. */
    data class Open(val title: String) : NeedsYouButton
}

object NeedsYouAnswer {
    /** Decisions show this many options as buttons; the rest go in a menu. */
    const val OPTION_BUTTONS = 3

    /**
     * An item's buttons. [mayAnswer] false is a reader below Control scope,
     * who sees the item and Open, and nothing that writes (spec §2.5) — the
     * runner already sent it no actions, and "Answer…" would be refused.
     *
     * A review only ever opens: the inbox opens a review and never approves
     * one (ruling 2).
     */
    fun buttons(item: NeedsYouItem, mayAnswer: Boolean): List<NeedsYouButton> {
        val answers = item.actions.filter { it.id != OPEN }
        return when (item.kindValue) {
            NeedsYouKind.ASK ->
                if (mayAnswer && answers.isNotEmpty() && item.askId != null) answers.map(NeedsYouButton::Answer)
                else listOf(NeedsYouButton.Open(OPEN_TITLE))
            NeedsYouKind.DECISION -> when {
                !mayAnswer -> listOf(NeedsYouButton.Open(OPEN_TITLE))
                answers.isEmpty() -> listOf(NeedsYouButton.Write)
                answers.size <= OPTION_BUTTONS -> answers.map(NeedsYouButton::Answer)
                else -> answers.take(OPTION_BUTTONS).map(NeedsYouButton::Answer) +
                    NeedsYouButton.More(answers.drop(OPTION_BUTTONS))
            }
            NeedsYouKind.REVIEW -> listOf(NeedsYouButton.Open(REVIEW_TITLE))
            NeedsYouKind.BLOCKED, NeedsYouKind.UNKNOWN -> listOf(NeedsYouButton.Open(OPEN_TITLE))
        }
    }

    /**
     * The one line a refused answer leaves on its row (spec §2.5), from the
     * runner's `what`. Anything else — a dropped link, a runner too old to
     * say which — is the generic line; never the runner's own words.
     */
    fun refusal(what: String?, agent: String): String = when (what) {
        "not_held" -> "Someone already answered this."
        "not_delivered" -> "Couldn’t reach $agent. Try again."
        else -> "Couldn’t send that answer. Try again."
    }

    const val OPEN = "open"
    const val OPEN_TITLE = "Open"
    const val REVIEW_TITLE = "Review"
}

/** Which worktrees a list shows: one workspace's, or a repository's Unclaimed or Hidden ones. */
sealed interface WorktreeScope {
    val hostId: String

    /** Whether [worktree], on a runner whose fleet is [fleet], belongs in this list. */
    fun includes(worktree: Worktree, fleet: Fleet): Boolean

    /** What an empty list says. */
    val emptySentence: String

    /** A workspace's own worktrees: its Worktrees tab. See [NeedsYou.worktreesOf]. */
    data class OfWorkspace(override val hostId: String, val workspace: WorkspaceSummary) : WorktreeScope {
        override fun includes(worktree: Worktree, fleet: Fleet): Boolean =
            !worktree.isHidden && if (workspace.isImplicit) worktree.repository == workspace.repository
            else worktree.workspace == workspace.id

        override val emptySentence: String
            get() = "No worktrees yet. The orchestrator makes them as it dispatches tasks."
    }

    /**
     * A repository's worktrees no listed workspace owns ([hidden] false), or
     * its hidden ones, whoever owns them ([hidden] true).
     */
    data class OfRepository(
        override val hostId: String,
        val repository: String,
        val hidden: Boolean,
    ) : WorktreeScope {
        override fun includes(worktree: Worktree, fleet: Fleet): Boolean {
            if (worktree.repository != repository) return false
            if (hidden) return worktree.isHidden
            if (worktree.isHidden) return false
            val listed = fleet.workspaces ?: return false
            return worktree.workspace == null || listed.none { it.id == worktree.workspace }
        }

        override val emptySentence: String
            get() = if (hidden) "Nothing is hidden." else "Every worktree here belongs to a workspace."
    }
}
