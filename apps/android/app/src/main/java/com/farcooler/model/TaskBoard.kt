package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

// A repository's task board, as everything except the drawing of it.
//
// The Android twin of AgentKit's `TaskBoardModel.swift`, `TaskBoardAgents.swift`
// and `RunnerBoards.swift`, which the Mac and the iPhone both draw from. Every
// rule and every sentence here is one of theirs, transcribed, so a card reads
// the same on all three: which column it is in, what it asks of you, how much of
// its acceptance holds, and which agent is on it. The producer is the client
// core's `task.list` (`crates/client/src/tasks_json.rs`), whose keys are the
// CLI's `task list --json` — snake_case, because agents parse that output too.
//
// Pure, like `NeedsYou.kt`: a JVM can prove all of it, and `TaskBoardTest`
// does.

/**
 * Where a task sits. The wire word verbatim, and the column heading in this
 * app's voice — US English, so "Canceled" on screen while the wire keeps
 * `cancelled`.
 *
 * Closed over the seven this build knows. A runner newer than this app can send
 * an eighth; see [UnreadableTaskRow].
 */
enum class TaskStatus(val wire: String, val title: String) {
    NEEDS_DECISION("needs_decision", "Needs Decision"),
    BACKLOG("backlog", "Backlog"),
    TODO("todo", "To Do"),
    IN_PROGRESS("in_progress", "In Progress"),
    IN_REVIEW("in_review", "In Review"),
    DONE("done", "Done"),
    CANCELLED("cancelled", "Canceled");

    /** Done or canceled: work stopped for good. See [TaskRow.isStale] for where staleness applies. */
    val isFinished: Boolean get() = this == DONE || this == CANCELLED

    companion object {
        /**
         * The order the board lists statuses in. Needs Decision leads because
         * it is the one status waiting on the person reading; the rest are in
         * the order work moves. The declaration order above IS this order.
         */
        val ORDER: List<TaskStatus> = entries.toList()

        fun parse(word: String): TaskStatus? = entries.firstOrNull { it.wire == word }
    }
}

/** One checkable thing, and whether it holds. */
data class TaskAcceptanceLine(val id: String, val text: String, val met: Boolean)

/** How much of a card's acceptance holds. */
data class TaskAcceptanceProgress(val met: Int, val total: Int) {
    /** Every line holds. Drawn in the accent color: "ready to look at". */
    val isComplete: Boolean get() = total > 0 && met == total

    /** `2 of 5`, or `All 5 met` once they all are, and `Met` for one line. */
    val sentence: String
        get() = when {
            !isComplete -> "$met of $total"
            total == 1 -> "Met"
            else -> "All $total met"
        }
}

/** One task, as a board draws it. */
data class TaskRow(
    val id: String,
    val key: String,
    val title: String,
    val status: TaskStatus,
    /** Unix milliseconds the task entered [status]. */
    val statusSince: Long,
    val intent: String = "",
    val labels: List<String> = emptyList(),
    val acceptance: List<TaskAcceptanceLine> = emptyList(),
    val workspaceId: String? = null,
    /** Unix milliseconds the task was filed, or null from a runner too old to say. */
    val createdAt: Long? = null,
    /**
     * Unix milliseconds anything on the card last changed — a move, a note, an
     * edit — or null from a runner too old to say. Equal to [createdAt] on a
     * card nothing has happened to.
     */
    val updatedAt: Long? = null,
) {
    /** How long it has sat where it is. Never negative: a runner ahead of this clock. */
    fun stoppedForMs(nowMs: Long): Long = maxOf(0L, nowMs - statusSince)

    /**
     * Stopped moving when it should be moving: a day in In Progress or In
     * Review, the two statuses where an agent is meant to be working. Backlog
     * and To Do wait their turn, Needs Decision waits on the person reading
     * (who asked not to be nagged), and Done and Canceled have stopped for
     * good. The one place this rule lives on Android; AgentKit's
     * `TaskRow.staleness(at:)` is its twin. Exhaustive, so a new status has to
     * be decided rather than inherited.
     */
    fun isStale(nowMs: Long): Boolean = when (status) {
        TaskStatus.IN_PROGRESS, TaskStatus.IN_REVIEW -> stoppedForMs(nowMs) >= STALE_AFTER_MS
        TaskStatus.NEEDS_DECISION, TaskStatus.BACKLOG, TaskStatus.TODO,
        TaskStatus.DONE, TaskStatus.CANCELLED -> false
    }

    /** The sentence under a stale card, or null for one still moving. */
    fun stalenessNote(nowMs: Long): String? {
        if (!isStale(nowMs)) return null
        val days = stoppedForMs(nowMs) / DAY_MS
        return if (days <= 1) "Hasn’t moved in a day" else "Hasn’t moved in $days days"
    }

    /**
     * The quiet line on every card: "Updated 2h ago" when anything changed
     * after it was filed, else "Added 3d ago". Null on a stale card, whose
     * [stalenessNote] already says how long, and from a runner that sent no
     * `created_at`. Equal clocks are "Added": the runner sends them equal for
     * a card nothing has happened to.
     */
    fun timeNote(nowMs: Long): String? {
        if (stalenessNote(nowMs) != null) return null
        val created = createdAt ?: return null
        val updated = updatedAt
        return if (updated != null && updated > created) "Updated ${ago(nowMs - updated)}"
        else "Added ${ago(nowMs - created)}"
    }

    /** What the card asks of the person reading it: only Needs Decision asks. */
    val callToAction: String?
        get() = if (status == TaskStatus.NEEDS_DECISION) "Answer to unblock this" else null

    /** Null rather than `0 of 0`: a line on every card whether or not it says anything is skipped. */
    val acceptanceProgress: TaskAcceptanceProgress?
        get() = if (acceptance.isEmpty()) null
        else TaskAcceptanceProgress(acceptance.count { it.met }, acceptance.size)

    /** The panes working this task, in the order handed in. */
    fun livePanes(panes: List<Terminal>): List<Terminal> =
        panes.filter { TaskAgentLink.isWorking(it, id) }

    /**
     * What the card says about its agent. `runnerRecordsTasks` false is "can't
     * say": no control and no remark, never a guess.
     */
    fun agentPresence(livePanes: Int, runnerRecordsTasks: Boolean): TaskAgentPresence = when {
        !runnerRecordsTasks -> TaskAgentPresence.Unsaid
        livePanes > 0 -> TaskAgentPresence.Agents(livePanes)
        status == TaskStatus.IN_PROGRESS -> TaskAgentPresence.NoAgent
        else -> TaskAgentPresence.Unsaid
    }

    companion object {
        const val DAY_MS = 24L * 60 * 60 * 1000
        const val STALE_AFTER_MS = DAY_MS

        /**
         * `just now`, `5m ago`, `2h ago`, `3d ago`, `2mo ago`, `1y ago`.
         * AgentKit's `TaskRow.ago`, transcribed, so the three boards say the
         * same words; Android's `DateUtils` says "2 hours ago". Floors, so a
         * card is never called older than it is; negative is "just now".
         */
        fun ago(elapsedMs: Long): String {
            val ms = maxOf(0L, elapsedMs)
            val minute = 60_000L
            val hour = 60 * minute
            return when {
                ms < minute -> "just now"
                ms < hour -> "${ms / minute}m ago"
                ms < DAY_MS -> "${ms / hour}h ago"
                ms < 30 * DAY_MS -> "${ms / DAY_MS}d ago"
                ms < 365 * DAY_MS -> "${ms / (30 * DAY_MS)}mo ago"
                else -> "${ms / (365 * DAY_MS)}y ago"
            }
        }
    }
}

/** A row whose status word this build has no column for. Shown, never dropped. */
data class UnreadableTaskRow(val id: String, val key: String, val title: String, val status: String)

/** One status's worth of rows, in the order the runner listed them. */
data class TaskBoardColumn(val status: TaskStatus, val rows: List<TaskRow>)

/** A repository's board. */
data class TaskBoard(
    val columns: List<TaskBoardColumn>,
    val unreadable: List<UnreadableTaskRow> = emptyList(),
) {
    val rows: List<TaskRow> get() = columns.flatMap { it.rows }

    /** Anything on it at all, unreadable rows included. */
    val isEmpty: Boolean get() = rows.isEmpty() && unreadable.isEmpty()

    /** Tasks waiting on the person reading: the Needs Decision count. */
    val waitingOnYou: Int get() = columns.firstOrNull { it.status == TaskStatus.NEEDS_DECISION }?.rows?.size ?: 0

    /** The statuses a phone lists: those with a task in them, in [TaskStatus.ORDER]. */
    val listed: List<TaskBoardColumn> get() = columns.filter { it.rows.isNotEmpty() }

    /** Tasks (not panes) at least one live agent is on. */
    fun tasksWithLiveAgents(panes: List<Terminal>): Int = rows.count { it.livePanes(panes).isNotEmpty() }

    fun row(id: String): TaskRow? = rows.firstOrNull { it.id == id }

    companion object {
        val EMPTY = TaskBoard(emptyList())

        /** The title-bar sentence, or null at zero. The verb agrees with the noun. */
        fun waitingSentence(count: Int): String? = when {
            count <= 0 -> null
            count == 1 -> "1 task is waiting on you"
            else -> "$count tasks are waiting on you"
        }

        private val json = Json { ignoreUnknownKeys = true; isLenient = true }

        /** Decode `task.list`'s answer. Throws only when it is not that shape at all. */
        fun decode(text: String): TaskBoard = decode(json.parseToJsonElement(text).jsonObject)

        /**
         * Decode `{"tasks": [...]}`. A row missing a field it cannot be drawn
         * without is skipped, not fatal: one unreadable row must not cost the
         * other forty.
         */
        fun decode(body: JsonObject): TaskBoard {
            val tasks = body["tasks"] as? JsonArray ?: throw IllegalArgumentException("not a board")
            val rows = mutableMapOf<TaskStatus, MutableList<TaskRow>>()
            val unreadable = mutableListOf<UnreadableTaskRow>()
            for (element in tasks) {
                val t = element as? JsonObject ?: continue
                fun text(name: String) = t[name]?.jsonPrimitive?.contentOrNull
                val id = text("id") ?: continue
                val key = text("key") ?: continue
                val title = text("title") ?: continue
                val word = text("status") ?: continue
                val status = TaskStatus.parse(word)
                if (status == null) {
                    unreadable += UnreadableTaskRow(id, key, title, word)
                    continue
                }
                val acceptance = (t["acceptance"] as? JsonArray).orEmpty().mapNotNull { line ->
                    val o = line as? JsonObject ?: return@mapNotNull null
                    TaskAcceptanceLine(
                        id = o["id"]?.jsonPrimitive?.contentOrNull ?: return@mapNotNull null,
                        text = o["text"]?.jsonPrimitive?.contentOrNull ?: "",
                        met = o["met"]?.jsonPrimitive?.booleanOrNull ?: false,
                    )
                }
                rows.getOrPut(status) { mutableListOf() } += TaskRow(
                    id = id,
                    key = key,
                    title = title,
                    status = status,
                    statusSince = t["status_since"]?.jsonPrimitive?.longOrNull ?: 0L,
                    intent = text("intent") ?: "",
                    labels = (t["labels"] as? JsonArray).orEmpty()
                        .mapNotNull { it.jsonPrimitive.contentOrNull },
                    acceptance = acceptance,
                    workspaceId = text("workspace_id"),
                    // Absent, `null`, or an older runner's zero: not said.
                    // No time line rather than "Added 56y ago".
                    createdAt = t["created_at"]?.jsonPrimitive?.longOrNull?.takeIf { it > 0 },
                    updatedAt = t["updated_at"]?.jsonPrimitive?.longOrNull?.takeIf { it > 0 },
                )
            }
            return TaskBoard(
                columns = TaskStatus.ORDER.map { TaskBoardColumn(it, rows[it].orEmpty()) },
                unreadable = unreadable,
            )
        }
    }
}

/** What a card says about the agent on it. */
sealed interface TaskAgentPresence {
    /** Say nothing. */
    data object Unsaid : TaskAgentPresence

    /** In progress, the runner records panes' tasks, and none is working it. */
    data object NoAgent : TaskAgentPresence

    /** This many panes are working it. Never zero. */
    data class Agents(val count: Int) : TaskAgentPresence

    /**
     * The control's words: "Agent" for one, "N agents", "No agent", or null.
     * Android's sentence case, where the iPhone and the Mac title-case the
     * same words.
     */
    val title: String?
        get() = when (this) {
            Unsaid -> null
            NoAgent -> "No agent"
            is Agents -> if (count == 1) "Agent" else "$count agents"
        }
}

/** Which pane is working which card. The same rules as AgentKit's `TaskAgentLink`. */
object TaskAgentLink {
    /** The CLI's `working_on` states. `unknown` is a runner blinking, not a pane gone. */
    val LIVE_STATES = setOf("running", "starting", "unknown")

    /** The names a plain shell reports itself by. */
    private val SHELLS = setOf("sh", "zsh", "bash", "fish", "dash", "ksh", "-zsh")

    /** Not a shell and not a changes pane. */
    fun runsAgent(preset: String, isChangesPane: Boolean): Boolean {
        if (isChangesPane) return false
        // The first non-empty piece, as Swift's `split(separator:)` gives it:
        // `":x"` names `x` on the iPhone and must here too.
        val name = preset.split(':').firstOrNull { it.isNotEmpty() } ?: ""
        if (name.isEmpty()) return false
        return name != "shell" && name.lowercase() !in SHELLS
    }

    fun runsAgent(terminal: Terminal): Boolean = runsAgent(terminal.preset, terminal.isChangesPane)

    /** Whether [pane] is working the task with id [taskId], whatever that task's status. */
    fun isWorking(pane: Terminal, taskId: String): Boolean {
        val id = pane.taskId
        if (id.isNullOrEmpty() || id != taskId) return false
        return runsAgent(pane) && pane.state in LIVE_STATES
    }

    /**
     * Whether a board may say anything about agents on one runner: it is
     * connected right now, and it records which pane works which task. Anything
     * else is "can't say", which is different from "none".
     */
    fun speaksOfAgents(connected: Boolean, build: DaemonBuild?): Boolean =
        connected && build?.can("terminal_task") == true

    /** Menu items for several panes, told apart by short id where the titles collide. */
    fun menuTitles(titles: List<String>, shorts: List<String>): List<String> {
        val counts = titles.groupingBy { it }.eachCount()
        return titles.mapIndexed { i, title ->
            if ((counts[title] ?: 0) > 1 && i < shorts.size) "$title (${shorts[i]})" else title
        }
    }

    /** What a board says when its Agent button has nowhere to land. */
    const val PANE_HAS_CLOSED = "That agent’s pane has closed."
}

/** One repository's Board row on the front door. */
data class BoardRow(
    val hostId: String,
    val repository: String,
    val name: String,
    val decisions: Int,
    val agents: Int,
) {
    /**
     * What a screen reader says after the row's name: the counts in words, or
     * null. Joined with ", " as the iPhone joins them (`RunnerBoardRow.spoken`).
     */
    val spoken: String?
        get() {
            val parts = listOfNotNull(decisionsSentence(decisions), agentsSentence(agents))
            return if (parts.isEmpty()) null else parts.joinToString(", ")
        }

    companion object {
        fun decisionsSentence(count: Int): String? = when {
            count <= 0 -> null
            count == 1 -> "1 task needs a decision"
            else -> "$count tasks need a decision"
        }

        fun agentsSentence(count: Int): String? = when {
            count <= 0 -> null
            count == 1 -> "An agent is on 1 task"
            else -> "Agents are on $count tasks"
        }
    }
}

object RunnerBoards {
    /**
     * One runner's Board rows, in the order the runner lists its repositories.
     *
     * - A runner that does not advertise `tasks` has no board, and gets no rows;
     *   nor does one no link has asked yet ([build] null).
     * - A repository gets a row only once its board has been read and has
     *   something on it. An empty board is most repositories on most runners.
     * - [BoardRow.agents] is counted only where [TaskAgentLink.speaksOfAgents]
     *   holds; anywhere else it is 0, which draws nothing — "can't say".
     */
    fun rows(
        hostId: String,
        repositories: List<Repository>,
        boards: Map<String, TaskBoard>,
        panes: List<Terminal>,
        build: DaemonBuild?,
        connected: Boolean,
    ): List<BoardRow> {
        if (build?.can("tasks") != true) return emptyList()
        val speaks = TaskAgentLink.speaksOfAgents(connected, build)
        return repositories.mapNotNull { repository ->
            val board = boards[repository.id] ?: return@mapNotNull null
            if (board.isEmpty) return@mapNotNull null
            BoardRow(
                hostId = hostId,
                repository = repository.id,
                name = repository.displayName.ifEmpty { repository.short },
                decisions = board.waitingOnYou,
                agents = if (speaks) board.tasksWithLiveAgents(panes) else 0,
            )
        }
    }
}

/**
 * Whether a link still owes its boards a read. A new link clears it; a sweep
 * that read the repositories sets it. When the runner's build lands on a link
 * that still owes one — its first `host` read failed and a later poll installed
 * it — the boards are read then, rather than never on that link.
 */
class BoardSweep {
    var sweptOnThisLink: Boolean = false
        private set

    fun linkCameUp() {
        sweptOnThisLink = false
    }

    fun swept() {
        sweptOnThisLink = true
    }

    val owedWhenBuildLands: Boolean get() = !sweptOnThisLink
}

/**
 * Where a board's Agent button lands: the pane's workspace on its runner, if the
 * runner's fleet still has that pane. Null is "the pane has closed".
 */
fun landingWorkspace(terminalId: String, workspaces: List<Workspace>): String? =
    workspaces.firstOrNull { workspace -> workspace.terminals.any { it.id == terminalId } }?.id
