package com.farcooler.model

import com.farcooler.core.NativeClient
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.put
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlin.math.roundToLong

/** Calls the shared task board view through the client-core presenter bridge. */
internal object TaskBoardPresenter {
    val json = Json { ignoreUnknownKeys = true }

    @Serializable
    data class Settings(
        val day: Double,
        @SerialName("done_floor") val doneFloor: Int,
        @SerialName("section_limit") val sectionLimit: Int,
        @SerialName("live_states") val liveStates: Set<String>,
    )

    @Serializable
    data class Status(
        val wire: String,
        val title: String,
        val finished: Boolean,
        @SerialName("holds_blocks") val holdsBlocks: Boolean,
        val action: String?,
    )

    @Serializable
    data class Clock(
        @SerialName("last_moved") val lastMoved: Double,
        @SerialName("stopped_for") val stoppedFor: Double,
        @SerialName("stale_note") val staleNote: String?,
        @SerialName("time_note") val timeNote: String?,
    )

    @Serializable
    data class Board(val columns: List<Column>, val unreadable: List<Int>) {
        @Serializable
        data class Column(val status: String, val indices: List<Int>)
    }

    @Serializable
    data class Acceptance(val met: Int, val total: Int)

    @Serializable
    data class Presence(val kind: String, val count: Int = 0)

    @Serializable
    data class Cut(val indices: List<Int>, val hidden: Int, val history: Int?)

    inline fun <reified T> ask(op: String, fields: Map<String, JsonElement> = emptyMap()): T =
        json.decodeFromJsonElement(answer(op, fields))

    fun answer(op: String, fields: Map<String, JsonElement>): JsonElement {
        val request = buildJsonObject {
            put("schema", 1)
            put("task_board", buildJsonObject {
                put("op", op)
                put("android", true)
                fields.forEach { (key, value) -> put(key, value) }
            })
        }
        val answer = json.parseToJsonElement(checkNotNull(NativeClient.nativePlanView(request.toString()))).jsonObject
        check(answer["error"]?.jsonPrimitive?.contentOrNull == null)
        return requireNotNull(answer["task_board"])
    }

    val settings: Settings by lazy { ask("settings") }
    val statuses: List<Status> by lazy { ask("statuses") }
    val byWire: Map<String, Status> by lazy { statuses.associateBy { it.wire } }
}

enum class TaskStatus(val wire: String) {
    NEEDS_DECISION("needs_decision"),
    BACKLOG("backlog"),
    TODO("todo"),
    IN_PROGRESS("in_progress"),
    IN_REVIEW("in_review"),
    DONE("done"),
    CANCELLED("cancelled");

    val title: String get() = TaskBoardPresenter.byWire.getValue(wire).title
    val isFinished: Boolean get() = TaskBoardPresenter.byWire.getValue(wire).finished

    companion object {
        val ORDER: List<TaskStatus> by lazy { TaskBoardPresenter.statuses.map { requireNotNull(parse(it.wire)) } }

        fun parse(word: String): TaskStatus? = entries.firstOrNull { it.wire == word }
    }
}

/** One checkable thing, and whether it holds. */
data class TaskAcceptanceLine(val id: String, val text: String, val met: Boolean)

/** How much of a card's acceptance holds. */
data class TaskAcceptanceProgress(val met: Int, val total: Int) {
    /** Every line holds. Drawn in the accent color: "ready to look at". */
    val isComplete: Boolean get() = total > 0 && met == total

    val sentence: String
        get() = TaskBoardPresenter.ask("acceptance_words", mapOf("met" to JsonPrimitive(met), "total" to JsonPrimitive(total)))
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
    val worktreeId: String? = null,
    /** Unix milliseconds the task was filed, or null from a runner too old to say. */
    val createdAt: Long? = null,
    /**
     * Unix milliseconds anything on the card last changed — a move, a note, an
     * edit — or null from a runner too old to say. Equal to [createdAt] on a
     * card nothing has happened to.
     */
    val updatedAt: Long? = null,
    /**
     * The board this task is on: its workspace's id, sent as `workspace`, or
     * null from a runner without `workstreams`.
     */
    val workspaceId: String? = null,
    /**
     * What it is waiting for before it starts (ov-212), or null: nothing said,
     * a runner without `task_waits`, or a word this build cannot read. Read
     * through [startLine], which knows which statuses it means something in.
     */
    val wait: TaskWait? = null,
    /**
     * The subagents recorded working it (ov-213): every open one, then the most
     * recently closed. Empty from a runner without `task_workers`.
     */
    val workers: List<TaskWorker> = emptyList(),
    /**
     * The keys of the blockers still open, from the list's `waiting_on`
     * (ov-212): a blocker that finished stops counting on its own. Only for
     * Backlog, To Do and In Progress, the statuses a block holds back
     * ([holdsBlocks]).
     */
    val blockedBy: List<String> = emptyList(),
) {
    private fun clock(nowMs: Long): TaskBoardPresenter.Clock =
        TaskBoardPresenter.ask("clock", buildJsonObject {
            put("status", status.wire)
            put("since", statusSince / 1000.0)
            put("now", nowMs / 1000.0)
            createdAt?.let { put("created", it / 1000.0) }
            updatedAt?.let { put("updated", it / 1000.0) }
        })

    val lastMovedMs: Long get() = (clock(statusSince).lastMoved * 1000).roundToLong()
    fun stoppedForMs(nowMs: Long): Long = (clock(nowMs).stoppedFor * 1000).roundToLong()
    fun isStale(nowMs: Long): Boolean = stalenessNote(nowMs) != null
    fun stalenessNote(nowMs: Long): String? = clock(nowMs).staleNote
    fun timeNote(nowMs: Long): String? = clock(nowMs).timeNote

    val callToAction: String? get() = TaskBoardPresenter.byWire.getValue(status.wire).action

    val acceptanceProgress: TaskAcceptanceProgress?
        get() {
            val progress: TaskBoardPresenter.Acceptance? = TaskBoardPresenter.ask(
                "acceptance", mapOf("lines" to JsonArray(acceptance.map { JsonPrimitive(it.met) })),
            )
            return progress?.let { TaskAcceptanceProgress(it.met, it.total) }
        }

    /** The panes working this task, in the order handed in. */
    fun livePanes(panes: List<Terminal>): List<Terminal> =
        panes.filter { TaskAgentLink.isWorking(it, id) }

    /**
     * What the card says about its agent. `runnerRecordsTasks` false is "can't
     * say": no control and no remark, never a guess.
     */
    fun agentPresence(livePanes: Int, runnerRecordsTasks: Boolean): TaskAgentPresence {
        val subagents = subagentPresence() as? TaskAgentPresence.Subagents
        val ranked = wait as? TaskWait.InLine
        val view: TaskBoardPresenter.Presence = TaskBoardPresenter.ask("agent_presence", buildJsonObject {
            put("live", livePanes)
            put("recorded", runnerRecordsTasks)
            put("status", status.wire)
            put("ranked", ranked?.line == TaskLine.BUILD && ranked.position > 0)
            subagents?.let {
                put("subagents", it.count)
                put("state", it.state.name.lowercase())
            }
        })
        return when (view.kind) {
            "agents" -> TaskAgentPresence.Agents(view.count)
            "subagents" -> checkNotNull(subagents)
            "no_agent" -> TaskAgentPresence.NoAgent
            else -> TaskAgentPresence.Unsaid
        }
    }

    companion object {
        val DAY_MS: Long get() = (TaskBoardPresenter.settings.day * 1000).toLong()
        val STALE_AFTER_MS: Long get() = DAY_MS

        /**
         * How long until the wall clock next crosses a minute, so a ticking
         * card redraws as the minute turns rather than up to a minute late.
         * Always in (0, 60 s]: exactly on a boundary waits a whole minute.
         */
        fun untilNextMinuteMs(nowMs: Long): Long = 60_000L - Math.floorMod(nowMs, 60_000L)

        fun ago(elapsedMs: Long): String =
            TaskBoardPresenter.ask("age", mapOf("seconds" to JsonPrimitive(elapsedMs / 1000.0)))
    }
}

/** A row whose status word this build has no column for. Shown, never dropped. */
data class UnreadableTaskRow(val id: String, val key: String, val title: String, val status: String)

/** One status's worth of rows, in the order the runner listed them. */
data class TaskBoardColumn(val status: TaskStatus, val rows: List<TaskRow>) {
    /** How many tasks are in this status: what a section's header shows, 0 included. */
    val count: Int get() = rows.size
}

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

    /**
     * Every status in [TaskStatus.ORDER], Needs Decision first, each with its
     * rows and count, the empty ones included (ov-55, owner decision 3). An
     * empty status is a collapsed header reading "Backlog 0", not a gap.
     * Built over the order rather than [columns], so a board with some
     * columns or none ([EMPTY]) still has all seven. AgentKit's
     * `TaskBoardModel.sections`.
     */
    val sections: List<TaskBoardColumn>
        get() = TaskStatus.ORDER.map { status ->
            TaskBoardColumn(status, columns.firstOrNull { it.status == status }?.rows.orEmpty())
        }

    /** Tasks (not panes) at least one live agent is on. */
    fun tasksWithLiveAgents(panes: List<Terminal>): Int = rows.count { it.livePanes(panes).isNotEmpty() }

    fun row(id: String): TaskRow? = rows.firstOrNull { it.id == id }

    companion object {
        val EMPTY = TaskBoard(emptyList())

        fun waitingSentence(count: Int): String? =
            TaskBoardPresenter.ask("waiting", mapOf("count" to JsonPrimitive(count)))

        private val json = Json { ignoreUnknownKeys = true; isLenient = true }

        /** Decode `task.list`'s answer. Throws only when it is not that shape at all. */
        fun decode(text: String): TaskBoard = decode(json.parseToJsonElement(text).jsonObject)

        /**
         * Decode `{"tasks": [...]}`. A row missing a field it cannot be drawn
         * without is skipped, not fatal: one unreadable row must not cost the
         * other forty.
         */
        fun decode(body: JsonObject): TaskBoard {
            val tasks = body["tasks"] as? JsonArray ?: throw IllegalArgumentException("task.list.tasks")
            val payloads = tasks.map { element ->
                (element as? JsonObject)?.takeIf {
                    listOf("id", "key", "title", "status").all { field -> it[field]?.jsonPrimitive?.contentOrNull != null }
                }
            }
            val statuses = JsonArray(payloads.map { row ->
                row?.get("status")?.jsonPrimitive?.contentOrNull?.let(::JsonPrimitive) ?: JsonNull
            })
            val view: TaskBoardPresenter.Board = TaskBoardPresenter.ask("columns", mapOf("statuses" to statuses))
            val columns = view.columns.map { column ->
                val status = requireNotNull(TaskStatus.parse(column.status))
                TaskBoardColumn(status, column.indices.map { decodeRow(requireNotNull(payloads[it]), status) })
            }
            val unreadable = view.unreadable.map { index ->
                val row = requireNotNull(payloads[index])
                UnreadableTaskRow(row.getValue("id").jsonPrimitive.content, row.getValue("key").jsonPrimitive.content,
                    row.getValue("title").jsonPrimitive.content, row.getValue("status").jsonPrimitive.content)
            }
            return TaskBoard(columns, unreadable)
        }

        private fun decodeRow(t: JsonObject, status: TaskStatus): TaskRow {
            fun text(name: String) = t[name]?.jsonPrimitive?.contentOrNull
            val acceptance = (t["acceptance"] as? JsonArray).orEmpty().mapNotNull { line ->
                val o = line as? JsonObject ?: return@mapNotNull null
                TaskAcceptanceLine(
                    id = o["id"]?.jsonPrimitive?.contentOrNull ?: return@mapNotNull null,
                    text = o["text"]?.jsonPrimitive?.contentOrNull ?: "",
                    met = o["met"]?.jsonPrimitive?.booleanOrNull ?: false,
                )
            }
            return TaskRow(
                id = requireNotNull(text("id")),
                key = requireNotNull(text("key")),
                title = requireNotNull(text("title")),
                status = status,
                statusSince = t["status_since"]?.jsonPrimitive?.longOrNull ?: 0L,
                intent = text("intent") ?: "",
                labels = (t["labels"] as? JsonArray).orEmpty()
                    .mapNotNull { it.jsonPrimitive.contentOrNull },
                acceptance = acceptance,
                worktreeId = text("worktree_id"),
                // Absent, `null`, or an older runner's zero: not said.
                // No time line rather than "Added 56y ago".
                createdAt = t["created_at"]?.jsonPrimitive?.longOrNull?.takeIf { it > 0 },
                updatedAt = t["updated_at"]?.jsonPrimitive?.longOrNull?.takeIf { it > 0 },
                workspaceId = text("workspace"),
                wait = StartWire.wait(t["wait"]),
                workers = StartWire.workers(t["workers"]),
                blockedBy = if (holdsBlocks(status)) StartWire.waitingOn(t["waiting_on"]) else emptyList(),
            )
        }
    }
}

/** What a card says about the agent on it. */
sealed interface TaskAgentPresence {
    /** Say nothing. */
    data object Unsaid : TaskAgentPresence

    /**
     * In progress, the runner records panes' tasks, and none is working it:
     * no pane, no subagent and no ranked build wait (ov-213).
     */
    data object NoAgent : TaskAgentPresence

    /**
     * This many panes are working it. Never zero. With subagents as well the
     * count is both, and the control still goes to the panes.
     */
    data class Agents(val count: Int) : TaskAgentPresence

    /**
     * This many subagents are working it, from inside another agent's session
     * (ov-213), and no pane is. Never zero. The control goes to the
     * orchestrator's pane, where they live.
     */
    data class Subagents(val count: Int, val state: SubagentState) : TaskAgentPresence

    /**
     * The control's words: "Agent" for one, "N agents", "No agent", or null.
     * Android's sentence case, where the iPhone and the Mac title-case the
     * same words.
     */
    val title: String?
        get() {
            val (kind, count) = when (this) {
                Unsaid -> return null
                NoAgent -> "no_agent" to 0
                is Agents -> "agents" to count
                is Subagents -> "subagents" to count
            }
            return TaskBoardPresenter.ask("agent_title", mapOf("kind" to JsonPrimitive(kind), "count" to JsonPrimitive(count)))
        }
}

/** Which pane is working which card. The same rules as AgentKit's `TaskAgentLink`. */
object TaskAgentLink {
    /** The CLI's `working_on` states. `unknown` is a runner blinking, not a pane gone. */
    val LIVE_STATES: Set<String> get() = TaskBoardPresenter.settings.liveStates

    fun runsAgent(preset: String, isChangesPane: Boolean): Boolean =
        TaskBoardPresenter.ask("runs_agent", mapOf("preset" to JsonPrimitive(preset), "changes" to JsonPrimitive(isChangesPane)))

    fun runsAgent(terminal: Terminal): Boolean = runsAgent(terminal.preset, terminal.isClientDrawn)

    /** Whether [pane] is working the task with id [taskId], whatever that task's status. */
    fun isWorking(pane: Terminal, taskId: String): Boolean {
        val id = pane.taskId
        if (id.isNullOrEmpty() || id != taskId) return false
        return runsAgent(pane) && pane.state in LIVE_STATES
    }

    /**
     * Whether a board may say anything about agents on one runner: it is
     * answering right now — connected, and its fleet read on this link, since
     * the panes an agent count is made from are that fleet's (see
     * [RunnerLink.given]) — and it records which pane works which task.
     * Anything else is "can't say", which is different from "none".
     */
    fun speaksOfAgents(link: RunnerLink, build: DaemonBuild?): Boolean =
        link == RunnerLink.ANSWERING && build?.can(Capability.TERMINAL_TASK) == true

    /** Menu items for several panes, told apart by short id where the titles collide. */
    fun menuTitles(titles: List<String>, shorts: List<String>): List<String> {
        return TaskBoardPresenter.ask("menu_titles", mapOf(
            "titles" to JsonArray(titles.map(::JsonPrimitive)),
            "shorts" to JsonArray(shorts.map(::JsonPrimitive)),
        ))
    }

    /** What a board says when its Agent button has nowhere to land. */
    val PANE_HAS_CLOSED: String by lazy { TaskBoardPresenter.ask("closed_pane") }
}

/**
 * One workspace's Board row on the front door.
 *
 * A repository has a board per workspace now. On a runner without
 * `workstreams` the one workspace is implicit — its id the repository's — and
 * the row is the repository's board, named for the repository, as it was.
 */
data class BoardRow(
    val hostId: String,
    val repository: String,
    val name: String,
    val decisions: Int,
    val agents: Int,
    /** The board: which workspace, and how to read it ([WorkspaceSummary.boardWorkspace]). */
    val workspace: WorkspaceSummary = WorkspaceSummary.implicit(repository),
    /**
     * The repository's display name, for a real workspace's row: "Main board"
     * alone would not say which repository's Main. Null for an implicit one,
     * whose [name] already is the repository's.
     */
    val repositoryName: String? = null,
) {
    /** What the boards are keyed by: the workspace's id. */
    val key: String get() = workspace.id

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
        fun decisionsSentence(count: Int): String? =
            TaskBoardPresenter.ask("decisions", mapOf("count" to JsonPrimitive(count)))

        fun agentsSentence(count: Int): String? =
            TaskBoardPresenter.ask("agents", mapOf("count" to JsonPrimitive(count)))
    }
}

object RunnerBoards {
    @Serializable
    private data class RepositoryGroup(val repository: String, val indices: List<Int>)

    fun boards(repositories: List<String>, workspaces: List<WorkspaceSummary>?): List<WorkspaceSummary> {
        val all = workspaces.orEmpty()
        val groups: List<RepositoryGroup> = TaskBoardPresenter.ask("repository_groups", mapOf(
            "repositories" to JsonArray(repositories.map(::JsonPrimitive)),
            "workspace_repositories" to JsonArray(all.map { JsonPrimitive(it.repository) }),
        ))
        return groups.flatMap { group ->
            WorkspaceGrouping.group(
                repository = group.repository,
                workspaces = group.indices.map { all[it] },
                worktrees = emptyList(),
                orchestrators = emptyMap(),
            ).workspaces.map { it.workspace }
        }
    }

    fun touched(notice: BoardNotice, boards: List<WorkspaceSummary>): List<WorkspaceSummary> {
        val found = boards.filter { notice.touches(it) }
        val missing: List<String> = TaskBoardPresenter.ask("missing_boards", mapOf(
            "found" to JsonArray(found.map { JsonPrimitive(it.id) }),
            "workspace" to JsonPrimitive(notice.workspace),
            "from" to JsonPrimitive(notice.fromWorkspace),
        ))
        return found + missing.map { WorkspaceSummary(id = it, repository = notice.repository) }
    }

    /**
     * One runner's Board rows, in the order [boards] lists them.
     *
     * - A runner that does not advertise `tasks` has no board, and gets no rows;
     *   nor does one no link has asked yet ([build] null).
     * - A workspace gets a row whenever it exists, its board read or not and
     *   empty or not (ov-56): the row is the way onto the board, and a
     *   workspace made a moment ago has nothing on it yet. The board screen
     *   draws the empty and unread states.
     * - An implicit board — a repository's, on a runner without workspaces —
     *   gets a row the same way, empty or unread (ov-55, spec §5 and §8). It
     *   used to need something on it; but the workspace view treats it as a
     *   workspace, and an empty one with no row could never be opened.
     * - [BoardRow.agents] is counted only where [TaskAgentLink.speaksOfAgents]
     *   holds; anywhere else it is 0, which draws nothing — "can't say".
     * - A workspace's row is called by its name; an implicit one's by its
     *   repository's, from [repositories], as the row always was.
     *
     * [models] are the last good reads, by workspace id.
     */
    fun rows(
        hostId: String,
        boards: List<WorkspaceSummary>,
        repositories: List<Repository>,
        models: Map<String, TaskBoard>,
        panes: List<Terminal>,
        build: DaemonBuild?,
        link: RunnerLink,
    ): List<BoardRow> {
        if (build?.can(Capability.TASKS) != true) return emptyList()
        val speaks = TaskAgentLink.speaksOfAgents(link, build)
        val names = repositories.associate { it.id to it.displayName.ifEmpty { it.short } }
        return boards.map { workspace ->
            val board = models[workspace.id]
            val repository = workspace.repository ?: workspace.id
            val repositoryName = names[repository]
            BoardRow(
                hostId = hostId,
                repository = repository,
                name = if (workspace.isImplicit) repositoryName ?: workspace.name else workspace.name,
                decisions = board?.waitingOnYou ?: 0,
                agents = if (speaks && board != null) board.tasksWithLiveAgents(panes) else 0,
                workspace = workspace,
                repositoryName = if (workspace.isImplicit) null else repositoryName,
            )
        }
    }

    fun decisions(workspace: WorkspaceSummary, items: List<NeedsYouItem>): Int =
        TaskBoardPresenter.ask("decision_count", buildJsonObject {
            put("workspace", workspace.id)
            put("implicit", workspace.isImplicit)
            put("items", buildJsonArray {
                items.forEach { item ->
                    add(buildJsonObject {
                        put("decision", item.kindValue.isDecision || item.alsoValues.any { it.isDecision })
                        put("workspace", item.workspaceId)
                        put("repository", item.repositoryId)
                    })
                }
            })
        })

    fun waiting(columnCount: Int, decisions: Int, listRead: Boolean, listServed: Boolean): Int =
        TaskBoardPresenter.ask("waiting_count", buildJsonObject {
            put("column", columnCount)
            put("decisions", decisions)
            put("read", listRead)
            put("served", listServed)
        })

    /**
     * The Board rows of a runner without workspaces: one per repository, each
     * its repository's implicit board. [rows] over [WorkspaceSummary.implicit].
     */
    fun rows(
        hostId: String,
        repositories: List<Repository>,
        boards: Map<String, TaskBoard>,
        panes: List<Terminal>,
        build: DaemonBuild?,
        link: RunnerLink,
    ): List<BoardRow> = rows(
        hostId = hostId,
        boards = repositories.map { WorkspaceSummary.implicit(it.id) },
        repositories = repositories,
        models = boards,
        panes = panes,
        build = build,
        link = link,
    )
}

/**
 * Whether a link still owes its boards a read, and which link a sweep is for.
 * The iPhone's `BoardSweep`, and the same rules.
 *
 * A new link clears it; a sweep that read the repositories sets it. When the
 * runner's build lands on a link whose sweep was refused for want of it —
 * its first `host` read failed and a later poll installed it — the boards are
 * read then, rather than never on that link. Only a refused sweep is owed:
 * every link-up reads the build and then sweeps, so sweeping when the build
 * lands as well read every board twice on every connect.
 *
 * And a sweep belongs to its link: [link] moves with each new one, and a
 * sweep no longer [isCurrent] stops rather than read on over the new link.
 */
class BoardSweep {
    var sweptOnThisLink: Boolean = false
        private set
    var refusedOnThisLink: Boolean = false
        private set
    var link: Int = 0
        private set

    fun linkCameUp() {
        sweptOnThisLink = false
        refusedOnThisLink = false
        link += 1
    }

    fun swept() {
        sweptOnThisLink = true
    }

    /** A sweep on this link could not read, and read nothing. */
    fun refused() {
        refusedOnThisLink = true
    }

    val owedWhenBuildLands: Boolean get() = refusedOnThisLink && !sweptOnThisLink

    /** Whether a sweep started on [link] is still this link's. */
    fun isCurrent(link: Int): Boolean = link == this.link
}

/**
 * Where a board's Agent button lands: the pane's worktree on its runner, if the
 * runner's fleet still has that pane. Null is "the pane has closed".
 */
fun landingWorktree(terminalId: String, worktrees: List<Worktree>): String? =
    worktrees.firstOrNull { worktree -> worktree.terminals.any { it.id == terminalId } }?.id

/** Local read marks. The Rust board view compares and projects these values. */
data class BoardReads(val floorMs: Long, val opened: Map<String, Long> = emptyMap()) {
    fun mark(taskId: String): Long = readMark(floorMs, opened[taskId])

    fun isUnread(taskId: String, atMs: Long): Boolean =
        TaskBoardPresenter.ask("unread", mapOf("at" to JsonPrimitive(atMs), "mark" to JsonPrimitive(mark(taskId))))

    fun finishedUnread(row: TaskRow): Boolean = row.status.isFinished && isUnread(row.id, row.statusSince)

    fun open(row: TaskRow, nowMs: Long, latestMs: Long? = null): BoardReads =
        copy(opened = opened + (row.id to readMark(nowMs, row.lastMovedMs, latestMs)))

    fun pruned(): BoardReads = copy(opened = TaskBoardPresenter.ask("prune_marks", mapOf(
        "floor" to JsonPrimitive(floorMs), "opened" to JsonObject(opened.mapValues { JsonPrimitive(it.value) }),
    )))

    companion object {
        fun firstLook(nowMs: Long): BoardReads =
            BoardReads(TaskBoardPresenter.ask("first_look", mapOf("now" to JsonPrimitive(nowMs))))

        private fun readMark(vararg stamps: Long?): Long =
            TaskBoardPresenter.ask("read_mark", mapOf("stamps" to JsonArray(stamps.map(::JsonPrimitive))))
    }
}

/**
 * Where a board's read state is kept: this phone's preferences, until a runner
 * keeps it (ov-113, `BoardReadsKeeper`).
 */
interface BoardReadsStore {
    fun load(host: String, workspace: String, nowMs: Long): BoardReads
    fun save(reads: BoardReads, host: String, workspace: String)

    /**
     * What this phone kept for the board, as opposed to what [load] made up:
     * marks, and a floor only if someone set it. A first look nobody saw as
     * Unread is no floor here. Null when nothing real is kept.
     */
    fun keptReads(host: String, workspace: String): BoardReads?

    /** Whether this phone's state was sent up to a runner that keeps it, once, so it isn't again. */
    fun isUploaded(host: String, workspace: String): Boolean
    fun markUploaded(host: String, workspace: String)

    /** Marks made and not yet acknowledged by the runner, kept so they survive a relaunch. */
    fun loadPending(host: String, workspace: String): ReadsRaise
    fun savePending(pending: ReadsRaise, host: String, workspace: String)

    /**
     * Whether the floor was set on purpose on this phone (a Mark All as Read that was kept), as a phone counts it:
     * false for one a first look or an older build left. A store that can't say answers yes.
     */
    fun floorWasSet(host: String, workspace: String): Boolean = true
    fun markFloorSet(host: String, workspace: String) {}
}

/** A store that lives as long as the process: what a test, or a connection built without a phone, keeps its marks in. */
class InMemoryBoardReads : BoardReadsStore {
    private val states = mutableMapOf<String, BoardReads>()
    private val invented = mutableSetOf<String>()
    private val uploaded = mutableSetOf<String>()
    private val floorSet = mutableSetOf<String>()
    private val pending = mutableMapOf<String, ReadsRaise>()

    private fun key(host: String, workspace: String) = "$host/$workspace"

    override fun load(host: String, workspace: String, nowMs: Long): BoardReads {
        val k = key(host, workspace)
        return states[k] ?: BoardReads.firstLook(nowMs).also { states[k] = it; invented += k }
    }

    override fun save(reads: BoardReads, host: String, workspace: String) {
        val k = key(host, workspace)
        if (states[k]?.floorMs != reads.floorMs) invented -= k
        states[k] = reads.pruned()
    }

    override fun keptReads(host: String, workspace: String): BoardReads? {
        val k = key(host, workspace)
        val state = states[k] ?: return null
        if (k !in invented) return state
        return BoardReads(Long.MIN_VALUE, state.opened).takeIf { it.opened.isNotEmpty() }
    }

    override fun floorWasSet(host: String, workspace: String) = key(host, workspace) in floorSet
    override fun markFloorSet(host: String, workspace: String) { floorSet += key(host, workspace) }
    override fun isUploaded(host: String, workspace: String) = key(host, workspace) in uploaded
    override fun markUploaded(host: String, workspace: String) { uploaded += key(host, workspace) }
    override fun loadPending(host: String, workspace: String) = pending[key(host, workspace)] ?: ReadsRaise()
    override fun savePending(pending: ReadsRaise, host: String, workspace: String) { this.pending[key(host, workspace)] = pending }
}

/**
 * What the Done and Canceled sections show (ov-103), AgentKit's `BoardDone`
 * rule for rule: every task finished and still unread, plus everything
 * finished today, with a floor of the latest three. The rest are on the
 * History page, behind the "All Done" row.
 */
object BoardDone {
    val FLOOR: Int get() = TaskBoardPresenter.settings.doneFloor

    /** The row to the History page: "All Done". Its count is drawn beside it, never in parentheses. */
    fun historyTitle(status: TaskStatus): String =
        TaskBoardPresenter.ask("history_title", mapOf("status" to JsonPrimitive(status.wire)))

    fun newestFirst(rows: List<TaskRow>): List<TaskRow> =
        BoardSectionCut.project(rows, finished = true, filtering = true).rows

    fun shown(rows: List<TaskRow>, reads: BoardReads, nowMs: Long, zone: java.time.ZoneId = java.time.ZoneId.systemDefault()): List<TaskRow> {
        val today = java.time.Instant.ofEpochMilli(nowMs).atZone(zone).toLocalDate()
        return BoardSectionCut.project(rows, finished = true) { row ->
            reads.finishedUnread(row) ||
                java.time.Instant.ofEpochMilli(row.statusSince).atZone(zone).toLocalDate() == today
        }.rows
    }
}

/** How a section is cut for display. */
object BoardSectionCut {
    val LIMIT: Int get() = TaskBoardPresenter.settings.sectionLimit

    fun showMoreTitle(hidden: Int): String =
        TaskBoardPresenter.ask("show_more", mapOf("count" to JsonPrimitive(hidden)))

    data class Cut(val rows: List<TaskRow>, val hidden: Int, val history: Int?)

    internal fun project(
        rows: List<TaskRow>,
        finished: Boolean,
        showingAll: Boolean = false,
        filtering: Boolean = false,
        recent: (TaskRow) -> Boolean = { false },
    ): Cut {
        val view: TaskBoardPresenter.Cut = TaskBoardPresenter.ask("cut", buildJsonObject {
            put("finished", finished)
            put("all", showingAll)
            put("filtering", filtering)
            put("rows", buildJsonArray {
                rows.forEach { row ->
                    add(buildJsonObject {
                        put("id", row.id)
                        put("since", row.statusSince)
                        put("recent", recent(row))
                    })
                }
            })
        })
        return Cut(view.indices.map { rows[it] }, view.hidden, view.history)
    }

    fun cut(
        column: TaskBoardColumn,
        reads: BoardReads,
        nowMs: Long,
        showingAll: Boolean = false,
        zone: java.time.ZoneId = java.time.ZoneId.systemDefault(),
    ): Cut {
        if (!column.status.isFinished) return project(column.rows, finished = false, showingAll = showingAll)
        val today = java.time.Instant.ofEpochMilli(nowMs).atZone(zone).toLocalDate()
        return project(column.rows, finished = true) { row ->
            reads.finishedUnread(row) ||
                java.time.Instant.ofEpochMilli(row.statusSince).atZone(zone).toLocalDate() == today
        }
    }
}

/**
 * The History page (ov-103), AgentKit's `BoardHistory`: every task finished
 * in a status, grouped by when it landed, searchable by key and title, and
 * narrowed by the area its title names.
 */
object BoardHistory {
    enum class Period {
        TODAY, YESTERDAY, THIS_WEEK, EARLIER;

        val title: String get() = titles[ordinal]
    }

    private val titles: List<String> by lazy { TaskBoardPresenter.ask("period_titles") }

    data class Group(val period: Period, val rows: List<TaskRow>)

    fun period(
        atMs: Long,
        nowMs: Long,
        zone: java.time.ZoneId = java.time.ZoneId.systemDefault(),
        week: java.time.temporal.WeekFields = java.time.temporal.WeekFields.of(java.util.Locale.getDefault()),
    ): Period {
        val day = java.time.Instant.ofEpochMilli(atMs).atZone(zone).toLocalDate()
        val today = java.time.Instant.ofEpochMilli(nowMs).atZone(zone).toLocalDate()
        if (day == today) return Period.TODAY
        if (day == today.minusDays(1)) return Period.YESTERDAY
        val start = today.with(week.dayOfWeek(), 1)
        return if (!day.isBefore(start) && day.isBefore(start.plusWeeks(1))) Period.THIS_WEEK else Period.EARLIER
    }

    fun groups(
        rows: List<TaskRow>,
        nowMs: Long,
        zone: java.time.ZoneId = java.time.ZoneId.systemDefault(),
        week: java.time.temporal.WeekFields = java.time.temporal.WeekFields.of(java.util.Locale.getDefault()),
    ): List<Group> {
        val sorted = BoardDone.newestFirst(rows)
        return Period.entries.mapNotNull { period ->
            sorted.filter { period(it.statusSince, nowMs, zone, week) == period }
                .takeIf { it.isNotEmpty() }?.let { Group(period, it) }
        }
    }

    /** "Mac" of "Mac: diff viewer…"; null for a title with no area. */
    fun area(title: String): String? {
        val colon = title.indexOf(": ")
        if (colon <= 0) return null
        val area = title.substring(0, colon).trim()
        return area.takeIf { it.isNotEmpty() && it.length <= 16 && it.none { c -> c in ".,;!?()" } }
    }

    /** The areas, most used first, then by name: the chips. */
    fun areas(rows: List<TaskRow>): List<String> =
        rows.mapNotNull { area(it.title) }.groupingBy { it }.eachCount().entries
            .sortedWith(compareByDescending<Map.Entry<String, Int>> { it.value }.thenBy { it.key })
            .map { it.key }

    /** When [row] landed, under its period: the time today and yesterday, weekday and time this week, else the date. */
    fun landed(
        row: TaskRow,
        nowMs: Long,
        zone: java.time.ZoneId = java.time.ZoneId.systemDefault(),
        locale: java.util.Locale = java.util.Locale.getDefault(),
        week: java.time.temporal.WeekFields = java.time.temporal.WeekFields.of(locale),
    ): String {
        val at = java.time.Instant.ofEpochMilli(row.statusSince).atZone(zone)
        // The time in the locale's own style, so a 24-hour phone reads 19:00.
        val time = java.time.format.DateTimeFormatter.ofLocalizedTime(java.time.format.FormatStyle.SHORT).withLocale(locale)
        return when (period(row.statusSince, nowMs, zone, week)) {
            Period.TODAY, Period.YESTERDAY -> time.format(at)
            Period.THIS_WEEK -> java.time.format.DateTimeFormatter.ofPattern("EEE", locale).format(at) + " " + time.format(at)
            Period.EARLIER ->
                java.time.format.DateTimeFormatter.ofPattern(
                    if (at.year == java.time.Instant.ofEpochMilli(nowMs).atZone(zone).year) "MMM d" else listOf("MMM d", "yyyy").joinToString(", "),
                    locale,
                ).format(at)
        }
    }

    /** Rows in [area] (null for any) whose key or title carries every word of [query]. */
    fun filter(rows: List<TaskRow>, query: String, area: String? = null): List<TaskRow> {
        val words = query.split(Regex("\\s+")).filter { it.isNotEmpty() }
        return rows.filter { row ->
            (area == null || area(row.title) == area) &&
                words.all { w -> "${row.key} ${row.title}".contains(w, ignoreCase = true) }
        }
    }
}
