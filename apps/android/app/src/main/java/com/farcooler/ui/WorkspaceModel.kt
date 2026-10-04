package com.farcooler.ui

import com.farcooler.model.AgentHarness
import com.farcooler.model.BoardDone
import com.farcooler.model.BoardReads
import com.farcooler.model.BoardSectionCut
import com.farcooler.model.BoardSummary
import com.farcooler.model.Fleet
import com.farcooler.model.Repository
import com.farcooler.model.StateKind
import com.farcooler.model.WorkspaceSummary
import com.farcooler.model.TaskBoard
import com.farcooler.model.TaskRow
import com.farcooler.model.TaskStatus
import com.farcooler.model.Terminal
import com.farcooler.model.Worktree

// The workspace screen's rules, apart from the drawing of them, so a JVM test
// can hold them: which rows the Board tab lists, which rows a task links to,
// and what the Orchestrator tab says.

/** One line of the Board tab: a status's header, or a card under it. */
sealed interface BoardListEntry {
    val key: String

    /**
     * A status and its count. An empty status is a header reading
     * "Backlog 0" that can't be expanded (spec §5, owner decision 3): never
     * hidden, never a gap.
     */
    data class Header(val status: TaskStatus, val count: Int, val expanded: Boolean) : BoardListEntry {
        val expandable: Boolean get() = count > 0
        override val key: String get() = "header/${status.wire}"
    }

    data class Card(val row: TaskRow) : BoardListEntry {
        override val key: String get() = "task/${row.id}"
    }

    /** Under a long section: shows the rest ([hidden] of them), or, showing all, puts them away again. */
    data class ShowMore(val status: TaskStatus, val hidden: Int, val showingAll: Boolean = false) : BoardListEntry {
        override val key: String get() = "show-more/${status.wire}"
        val title: String get() = if (showingAll) "Show Fewer" else BoardSectionCut.showMoreTitle(hidden)
    }

    /** Under Done or Canceled: "All Done  94 ›", the History page (ov-103). */
    data class History(val status: TaskStatus, val total: Int) : BoardListEntry {
        override val key: String get() = "history/${status.wire}"
        val title: String get() = BoardDone.historyTitle(status)
    }

    // Unread (ov-113): the board's first section, the Mac's `BoardSummary`. Each
    // group is cut at five with "and N more".

    /** "Unread" and how many lines it lists; Mark all as read is offered while it lists any. */
    data class UnreadHeader(val count: Int) : BoardListEntry {
        override val key: String get() = "unread/header"
        val offersMarkAll: Boolean get() = count > 0
    }

    /** "You’re all caught up.", when nothing is unread. */
    data object UnreadNothing : BoardListEntry {
        override val key: String get() = "unread/nothing"
        val text: String get() = BoardSummary.NOTHING
    }

    /** A group's title and how many lines it has in all: Finished, Needs you or review, New, Activity. */
    data class UnreadGroup(val title: String, val count: Int) : BoardListEntry {
        override val key: String get() = "unread/group/$title"
    }

    /** A finish, a move or a new ticket, opening its task. */
    data class UnreadLine(val item: BoardSummary.Item) : BoardListEntry {
        override val key: String get() = "unread/line/${item.id}"
    }

    /** A ticket's newest unread note, opening its task. */
    data class UnreadNote(val activity: BoardSummary.Activity) : BoardListEntry {
        override val key: String get() = "unread/note/${activity.id}"
    }

    /** "and 2 more", under a group cut at five. */
    data class UnreadMore(val group: String, val count: Int) : BoardListEntry {
        override val key: String get() = "unread/more/$group"
        val text: String get() = "and $count more"
    }
}

object BoardList {
    /** The Unread section's entries, for [summary]: its header, then its groups, or "all caught up". */
    fun unreadEntries(summary: BoardSummary): List<BoardListEntry> {
        val out = mutableListOf<BoardListEntry>(BoardListEntry.UnreadHeader(summary.count))
        if (summary.isEmpty) return out + BoardListEntry.UnreadNothing
        fun <T> group(title: String, items: List<T>, line: (T) -> BoardListEntry) {
            if (items.isEmpty()) return
            val (shown, more) = BoardSummary.capped(items)
            out += BoardListEntry.UnreadGroup(title, items.size)
            shown.forEach { out += line(it) }
            if (more > 0) out += BoardListEntry.UnreadMore(title, more)
        }
        group("Finished", summary.finished, BoardListEntry::UnreadLine)
        group("Needs you or review", summary.moved, BoardListEntry::UnreadLine)
        group("New", summary.created, BoardListEntry::UnreadLine)
        group("Activity", summary.activity, BoardListEntry::UnreadNote)
        return out
    }

    /** Statuses that start collapsed when they have tasks: work that has stopped. */
    val COLLAPSED_AT_FIRST: Set<TaskStatus> = setOf(TaskStatus.DONE, TaskStatus.CANCELLED)

    /**
     * The list form (spec §5), from [TaskBoard.sections]: every status,
     * Needs Decision first. A status with tasks is expanded, except Done and
     * Canceled, which start collapsed; [toggled] are the statuses the person
     * flipped from that. An empty one is a collapsed header with a 0.
     */
    fun entries(
        board: TaskBoard,
        toggled: Set<TaskStatus>,
        nowMs: Long = System.currentTimeMillis(),
        reads: BoardReads = BoardReads.firstLook(nowMs),
        showingMore: Set<TaskStatus> = emptySet(),
        /** What's unread, to list first (ov-113); null leaves the section out. */
        unread: BoardSummary? = null,
    ): List<BoardListEntry> =
        unread?.let(::unreadEntries).orEmpty() + board.sections.flatMap { section ->
            val count = section.rows.size
            val expanded = count > 0 && ((section.status !in COLLAPSED_AT_FIRST) != (section.status in toggled))
            // Done and Canceled draw the unread and today's, newest first,
            // with the rest on the History page; a long section draws ten.
            val cut = BoardSectionCut.cut(section, reads, nowMs, showingAll = section.status in showingMore)
            listOf(BoardListEntry.Header(section.status, count, expanded)) +
                if (expanded) {
                    cut.rows.map(BoardListEntry::Card) +
                        listOfNotNull(
                            cut.hidden.takeIf { it > 0 }?.let { BoardListEntry.ShowMore(section.status, it) }
                                ?: BoardListEntry.ShowMore(section.status, 0, showingAll = true).takeIf {
                                    section.status in showingMore && !section.status.isFinished &&
                                        count > BoardSectionCut.LIMIT
                                },
                            cut.history?.let { BoardListEntry.History(section.status, it) },
                        )
                } else {
                    emptyList()
                }
        }
}

/** A row on the task screen that pushes somewhere other than an agent. */
sealed interface TaskLinkRow {
    val worktreeId: String

    /** The worktree's changes: the diff, as its Changes tab shows it. */
    data class Changes(override val worktreeId: String, val insertions: Int?, val deletions: Int?) : TaskLinkRow

    /** The worktree itself: its panes. */
    data class Worktree(override val worktreeId: String, val name: String) : TaskLinkRow
}

object TaskLinks {
    /**
     * A task's Changes and Worktree rows, through `Task.worktree_id` (spec
     * §3.2, item 3) — with or without a live agent, so a task in review whose
     * agent has gone still reaches its changes. None when the task names no
     * worktree, or one this runner's fleet doesn't have (removed since).
     */
    fun rows(task: TaskRow, worktrees: List<Worktree>): List<TaskLinkRow> {
        val id = task.worktreeId?.takeIf { it.isNotEmpty() } ?: return emptyList()
        val worktree = worktrees.firstOrNull { it.id == id } ?: return emptyList()
        return listOf(
            TaskLinkRow.Changes(id, null, null),
            TaskLinkRow.Worktree(id, worktree.task.ifBlank { worktree.branch }),
        )
    }
}

/** What a workspace's Orchestrator tab shows (spec §8). */
sealed interface OrchestratorSeat {
    /** No orchestrator. [canStart] is false on a runner without workspaces, or below Control scope. */
    data class Empty(val canStart: Boolean) : OrchestratorSeat

    /**
     * Asked for and not yet confirmed: "Starting Orchestrator…", then after
     * [SLOW_AFTER_MS] "This is taking longer than usual." with Replace…,
     * since the seat can stick.
     */
    data class Starting(val slow: Boolean) : OrchestratorSeat

    /** Running in [worktreeId]: the tab is its pane. */
    data class Live(val terminal: Terminal, val worktreeId: String) : OrchestratorSeat

    /** Its pane was lost, or its process ended: Restart and Replace…. */
    data class Lost(val terminal: Terminal, val worktreeId: String) : OrchestratorSeat

    companion object {
        const val SLOW_AFTER_MS = 30_000L

        /**
         * [seated] is the workspace's orchestrator terminal id as the runner
         * last said, [startedAt] when this phone asked for one and hasn't seen
         * it since (null if it didn't ask).
         */
        fun of(
            seated: String?,
            worktrees: List<Worktree>,
            startedAt: Long?,
            now: Long,
            canStart: Boolean,
        ): OrchestratorSeat {
            val found = seated?.let { id ->
                worktrees.firstNotNullOfOrNull { w -> w.terminals.firstOrNull { it.id == id }?.let { it to w.id } }
            }
            if (found != null) {
                val (terminal, worktree) = found
                return when (StateKind.parse(terminal.state)) {
                    StateKind.LOST, StateKind.EXITED, StateKind.ERROR -> Lost(terminal, worktree)
                    else -> Live(terminal, worktree)
                }
            }
            // A seat the runner names with no pane in the fleet was never
            // confirmed. This phone can't say how long ago it was taken, so
            // Replace… is offered at once; one it asked for itself waits its
            // thirty seconds first.
            if (startedAt != null) return Starting(slow = now - startedAt >= SLOW_AFTER_MS)
            if (seated != null) return Starting(slow = true)
            return Empty(canStart)
        }
    }
}

/** The harnesses an orchestrator runs on, in the Mac's order (`OrchestratorHarness`). */
enum class OrchestratorHarness(val wire: String, val title: String) {
    CLAUDE("claude", "Claude Code"),
    CODEX("codex", "Codex"),
    CURSOR("cursor", "Cursor"),
    ;

    /** The same harness as first run knows it, which says which programs a runner has. */
    val agent: AgentHarness get() = AgentHarness.entries.first { it.wire == wire }
}

/**
 * Why an orchestrator didn't start, in this app's words: the Mac's
 * `orchestratorRefusal` (ov-60), read from the runner's word and the `what`
 * it names — `orchestrator_taken` or `orchestrator_home` on an
 * `invalid-argument` (`service.rs`). Never the runner's own sentence, and
 * never a match on it.
 */
fun orchestratorRefusal(word: String?, what: String?, name: String, replace: Boolean): String = when {
    what == "orchestrator_taken" -> "$name already has an orchestrator. Choose Replace to start a new one."
    what == "orchestrator_home" -> "The runner couldn’t make $name’s folder, so no orchestrator started."
    word == "not-found" -> "$name isn’t on this runner anymore."
    word == "capability-unsupported" ->
        "This runner’s Far Cooler is too old to start an orchestrator. Update it there, then try again."
    word == "scope-denied" -> "This runner lets Far Cooler see its workspaces but not change them."
    word == "resource-conflict" -> "$name changed while its orchestrator was starting. Try again."
    word != null ->
        "This runner couldn’t start an orchestrator for $name. That’s a problem in the app, not in anything you did."
    replace -> "Couldn’t replace $name’s orchestrator. Check that the runner is reachable, then try again."
    else -> "Couldn’t start an orchestrator for $name. Check that the runner is reachable, then try again."
}

/** What the Orchestrator tab offers besides its pane. */
enum class SeatAction { START, RESTART, REPLACE }

/**
 * The buttons [seat] shows. None of them for a reader below Control scope, or
 * on a runner without workspaces ([mayControl] false): the runner would
 * refuse every one.
 */
fun seatActions(seat: OrchestratorSeat, mayControl: Boolean): Set<SeatAction> {
    if (!mayControl) return emptySet()
    return when (seat) {
        is OrchestratorSeat.Empty -> if (seat.canStart) setOf(SeatAction.START) else emptySet()
        is OrchestratorSeat.Starting -> if (seat.slow) setOf(SeatAction.REPLACE) else emptySet()
        is OrchestratorSeat.Lost -> setOf(SeatAction.RESTART, SeatAction.REPLACE)
        is OrchestratorSeat.Live -> setOf(SeatAction.REPLACE)
    }
}

/** Whether a workspace route still names a workspace on its runner. */
sealed interface WorkspacePresence {
    data class Found(val workspace: WorkspaceSummary) : WorkspacePresence

    /** The runner hasn't said yet: its fleet or its repositories are still to come. */
    data object Loading : WorkspacePresence

    /** The runner has answered and has no such workspace: deleted, or a stale saved stack. */
    data object Gone : WorkspacePresence

    companion object {
        /**
         * [id] on a runner whose fleet is [fleet], once it's [answered] on
         * this link. On a runner without workspaces the id is a repository's,
         * its implicit workspace. A repository's id on a runner that now has
         * workspaces — a board saved before they existed — is its Main.
         * Never invented: an unknown id is Gone, not a workspace called Main.
         */
        fun of(id: String, fleet: Fleet, repositories: List<Repository>, answered: Boolean): WorkspacePresence {
            val workspaces = fleet.workspaces
            workspaces?.firstOrNull { it.id == id }?.let { return Found(it) }
            workspaces?.firstOrNull { it.isMain && it.repository == id }?.let { return Found(it) }
            if (workspaces == null && repositories.any { it.id == id }) return Found(WorkspaceSummary.implicit(id))
            if (!answered || (workspaces == null && repositories.isEmpty())) return Loading
            return Gone
        }
    }
}

/**
 * What a workspace is called on a sheet made from its screen: its name, or
 * for a runner without workspaces its repository's — the name its row and
 * its screen use — rather than the implicit workspace's "Main".
 */
fun workspacePlace(workspace: WorkspaceSummary, repositories: List<Repository>): String =
    if (!workspace.isImplicit) workspace.name
    else repositories.firstOrNull { it.id == workspace.repository }
        ?.let { it.displayName.ifEmpty { it.short } }?.takeIf { it.isNotBlank() }
        ?: workspace.name
