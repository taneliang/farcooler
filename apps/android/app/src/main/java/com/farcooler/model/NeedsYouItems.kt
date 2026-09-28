package com.farcooler.model

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.Transient

/**
 * The needs-you rollup: what each runner says a person has to act on, and the
 * one list this phone draws from all of them.
 *
 * The rule is the daemon's, not this file's (spec §2.3). Each runner computes
 * its own items — held asks, blocked agents, tasks in Needs Decision, tasks In
 * Review — and `Session::needs_you` hands them here as the JSON
 * `crates/client/src/needs_you_json.rs` writes. What this file does is decode
 * that shape, merge runners by rank, and derive what little an older runner
 * can say. AgentKit's `NeedsYou.swift` does the same for the Apple apps, and
 * both are pinned by `test/fixtures/needs-you.json`.
 *
 * Keys are snake_case, as the projection writes them. An unset optional
 * arrives as a present key holding null, and an empty id is null rather than
 * the nil uuid.
 */

/** One runner's reply to `needs_you`: `{"items": [...]}`, in the daemon's order. */
@Serializable
data class NeedsYouList(val items: List<NeedsYouItem> = emptyList())

/**
 * The four kinds, in rank order, and [UNKNOWN] for a word this build doesn't
 * define. An unknown kind is kept and sorted last, never dropped: a newer
 * runner's new reason to interrupt somebody is still a reason.
 */
enum class NeedsYouKind(val wire: String) {
    ASK("ask"),
    BLOCKED("blocked"),
    DECISION("decision"),
    REVIEW("review"),
    UNKNOWN("unknown");

    companion object {
        fun parse(raw: String?): NeedsYouKind = entries.firstOrNull { it.wire == raw } ?: UNKNOWN
    }
}

/** The terminal an item is about: `{id, worktree_id, label, role, pane_mode, chat_capable}`. */
@Serializable
data class NeedsYouTerminal(
    val id: String,
    @SerialName("worktree_id") val worktreeId: String? = null,
    val label: String = "",
    val role: String? = null,
    @SerialName("pane_mode") val paneMode: String? = null,
    @SerialName("chat_capable") val chatCapable: Boolean = false,
)

/** The worktree an item's work is in: `{id, name, branch, insertions, deletions}`. */
@Serializable
data class NeedsYouWorktree(
    val id: String,
    val name: String = "",
    val branch: String = "",
    val insertions: Int = 0,
    val deletions: Int = 0,
)

/**
 * One button on an item. [id] is what goes back to the runner: an ask's option
 * id for `terminal.agent_answer`, a decision option's text for `task.note`, or
 * `open`, which sends nothing.
 */
@Serializable
data class NeedsYouAction(
    val id: String,
    val title: String = "",
    val destructive: Boolean = false,
    val primary: Boolean = false,
)

/** One thing a person has to act on, as one runner sent it. */
@Serializable
data class NeedsYouItem(
    /** Stable across reads: `ask:<ask id>`, `blocked:<terminal>`, `decision:<task>` or `review:<task>`. */
    val id: String,
    /** The kind's wire word. Read it through [kindValue]. */
    val kind: String = "",
    /** The subject's other, less urgent signals, as kind words. */
    val also: List<String> = emptyList(),
    /**
     * On `Terminal.rank`'s scale: a tier per kind, then the oldest first. A
     * duration and never a clock reading, so two runners' items merge by it
     * without comparing clocks.
     */
    val rank: Long = Long.MAX_VALUE,
    /** Unix milliseconds the item began, on the runner's clock. Shown, never sorted by. */
    val since: Long? = null,
    @SerialName("workspace_id") val workspaceId: String? = null,
    @SerialName("workspace_name") val workspaceName: String = "",
    @SerialName("repository_id") val repositoryId: String? = null,
    val task: TaskRef? = null,
    val terminal: NeedsYouTerminal? = null,
    val worktree: NeedsYouWorktree? = null,
    /** One row wide, already redacted on the runner for a reader below Control scope. */
    val question: String = "",
    /** An ask's command, or a review's `+18 −40`. */
    val detail: String? = null,
    /** What `terminal.agent_answer` takes as its request id. */
    @SerialName("ask_id") val askId: String? = null,
    /** Empty for a reader below Control scope, who gets no buttons. */
    val actions: List<NeedsYouAction> = emptyList(),
    /**
     * Made on this phone by [NeedsYouItems.derived] for a runner without
     * `needs_you`, rather than sent by one. Never on the wire. What lets a
     * row hedge ("Update Far Cooler on <runner> to see decisions and asks
     * here.") without tracking which runner is which version.
     */
    @Transient val isDerived: Boolean = false,
) {
    val kindValue: NeedsYouKind get() = NeedsYouKind.parse(kind)

    val alsoValues: List<NeedsYouKind> get() = also.map(NeedsYouKind::parse)
}

/** An item and the runner it came from, which is half its identity: ids are minted per daemon. */
data class RunnerNeedsYouItem(val hostId: String, val item: NeedsYouItem) {
    /** What a `LazyColumn` keys a row on. */
    val key: String get() = "$hostId/${item.id}"
}

object NeedsYouItems {
    /**
     * One tier's width on the rank scale, `farcooler_core::feed`'s and the
     * daemon's `needs_you.rs`'s alike.
     */
    const val TIER_SPAN: Long = 100_000_000L

    /**
     * Every runner's items as one list, most urgent first.
     *
     * By [NeedsYouItem.rank], because it's a duration: an ask four minutes old
     * on one runner and a review an hour old on another compare correctly
     * however far apart the two machines' clocks are. Then by runner and item
     * id, so equal ranks — two runners each with an agent stuck for the same
     * number of seconds — hold still between reads instead of swapping under a
     * finger. An [NeedsYouKind.UNKNOWN] item goes after every known one.
     */
    fun merge(runners: Map<String, List<NeedsYouItem>>): List<RunnerNeedsYouItem> =
        runners.flatMap { (host, items) -> items.map { RunnerNeedsYouItem(host, it) } }
            .sortedWith(
                compareBy<RunnerNeedsYouItem>(
                    { it.item.kindValue == NeedsYouKind.UNKNOWN },
                    { it.item.rank },
                    { it.hostId },
                    { it.item.id },
                )
            )

    /**
     * A workspace's count: its items, not its signals. An ask with a decision
     * in [NeedsYouItem.also] is one thing to do, and counts once.
     */
    fun count(items: List<RunnerNeedsYouItem>, hostId: String, workspaceId: String?): Int =
        items.count { it.hostId == hostId && it.item.workspaceId == workspaceId }

    /**
     * What a runner without `needs_you` still says (spec §2.6): its blocked
     * agents, as blocked items. Nothing else is guessed — no asks, decisions or
     * reviews, because this phone can't see any of them from a fleet.
     *
     * Each rank is re-tiered into the blocked tier. `Terminal.rank`'s tier 0
     * is Blocked while the item scale's tier 0 is ask, so a rank copied as it
     * stands would put this agent above a real ask on another runner. A
     * terminal with no rank sorts last in the blocked tier.
     *
     * Hidden worktrees still count: an orchestrator whose main checkout is
     * hidden is still waiting.
     */
    fun derived(worktrees: List<Worktree>): List<NeedsYouItem> =
        worktrees.flatMap { worktree ->
            worktree.terminals
                .filter { it.agent == AgentActivity.BLOCKED }
                .map { terminal -> derivedItem(worktree, terminal) }
        }

    private fun derivedItem(worktree: Worktree, terminal: Terminal): NeedsYouItem {
        val age = terminal.rank?.let { Math.floorMod(it, TIER_SPAN) } ?: (TIER_SPAN - 1)
        return NeedsYouItem(
            id = "blocked:${terminal.id}",
            kind = NeedsYouKind.BLOCKED.wire,
            rank = TIER_SPAN + age,
            since = terminal.activitySince?.toLong(),
            workspaceId = terminal.workspace ?: worktree.workspace,
            repositoryId = worktree.repository,
            // Only a task the pane was dispatched for: the daemon's subject
            // rule, which never takes a worktree's task for a bare pane.
            task = terminal.taskId?.takeIf { it.isNotEmpty() }?.let { id ->
                worktree.openTasks.firstOrNull { it.id == id } ?: TaskRef(id = id)
            },
            terminal = NeedsYouTerminal(
                id = terminal.id,
                worktreeId = worktree.id,
                label = terminal.label,
                role = terminal.role,
                paneMode = terminal.paneMode,
                chatCapable = terminal.chatCapable == true,
            ),
            worktree = NeedsYouWorktree(id = worktree.id, name = worktree.task, branch = worktree.branch),
            question = terminal.blockedQuestion?.ifBlank { null }
                ?: "${Terminal.name(terminal.preset)} needs you",
            actions = listOf(NeedsYouAction(id = "open", title = "Open")),
            isDerived = true,
        )
    }
}
