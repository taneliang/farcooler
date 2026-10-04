package com.farcooler.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

/**
 * What's unread on a board (ov-104, ov-113): what finished, moved to the person
 * or was filed, and the notes written, that this person hasn't seen, an item
 * staying until its ticket is opened ([BoardReads]). AgentKit's `BoardSummary`,
 * rule for rule, and `BoardSummaryTest` holds the same cases as its tests.
 *
 * Built from what the board already holds: a task's `statusSince` says when it
 * finished or moved to Needs Decision or In Review (the status it is in now,
 * entered then), and `createdAt` says when it was filed. Notes live in a task's
 * record, which `task.list` doesn't carry; [noteCandidates] names the tasks
 * whose record is worth reading, and [make] takes what was read.
 */
data class BoardSummary(
    val finished: List<Item> = emptyList(),
    val moved: List<Item> = emptyList(),
    val created: List<Item> = emptyList(),
    val activity: List<Activity> = emptyList(),
) {
    /**
     * One line of the summary, and the task it opens. [id] is the ticket's and
     * what happened to it ("<task>/done"), so a list keyed by it animates an
     * item in, out and along.
     */
    data class Item(
        val id: String,
        val taskId: String,
        val key: String,
        val title: String,
        /** What happened, when the title alone doesn't say: a status word. */
        val detail: String? = null,
        val atMs: Long,
    ) {
        /** "Done 2h ago", "Needs Decision 5m ago", "Added 3h ago". */
        fun whenSaid(nowMs: Long): String =
            "${detail ?: if (id.endsWith("/done")) "Done" else "Added"} ${TaskRow.ago(nowMs - atMs)}"
    }

    /** One ticket's notes: its newest, whole, and how many older ones it has in the summary too. */
    data class Activity(
        val taskId: String,
        val key: String,
        val title: String,
        val noteId: String,
        val kind: TaskNoteKind,
        val text: String,
        val atMs: Long,
        val more: Int,
    ) {
        val id: String get() = "$taskId/activity"

        val moreLine: String? get() = if (more > 0) "+$more more" else null

        /** "12m ago · +2 more". */
        fun foot(nowMs: Long): String = listOfNotNull(TaskRow.ago(nowMs - atMs), moreLine).joinToString(" · ")
    }

    val isEmpty: Boolean get() = finished.isEmpty() && moved.isEmpty() && created.isEmpty() && activity.isEmpty()

    val count: Int get() = finished.size + moved.size + created.size + activity.size

    /** How many tasks it lists: one under Finished and Activity both counts once. */
    val taskCount: Int
        get() = ((finished + moved + created).map { it.taskId } + activity.map { it.taskId }).toSet().size

    companion object {
        /** The most lines a group draws before "and N more". */
        const val GROUP_LIMIT = 5

        /** The most tickets whose notes a phone reads for Unread: each is a call. */
        const val NOTE_LIMIT = 10

        const val NOTHING = "You’re all caught up."

        fun <T> capped(items: List<T>, limit: Int = GROUP_LIMIT): Pair<List<T>, Int> =
            items.take(limit) to maxOf(0, items.size - limit)

        fun listed(kind: TaskNoteKind): Boolean = !kind.isMachineWritten

        /**
         * The tasks whose record is worth reading for notes: unread by their
         * last word, newest first, at most [limit].
         */
        fun noteCandidates(rows: List<TaskRow>, reads: BoardReads, limit: Int = NOTE_LIMIT): List<TaskRow> =
            rows.filter { it.status != TaskStatus.CANCELLED && reads.isUnread(it.id, it.lastMovedMs) }
                .sortedByDescending { it.lastMovedMs }
                .take(limit)

        fun make(rows: List<TaskRow>, notes: Map<String, List<TaskNoteRow>> = emptyMap(), reads: BoardReads): BoardSummary {
            val finished = mutableListOf<Item>()
            val moved = mutableListOf<Item>()
            val created = mutableListOf<Item>()
            for (row in rows) {
                fun item(what: String, detail: String?, atMs: Long) =
                    Item("${row.id}/$what", row.id, row.key, row.title, detail, atMs)
                val changed = reads.isUnread(row.id, row.statusSince)
                when {
                    row.status == TaskStatus.DONE && changed -> finished += item("done", null, row.statusSince)
                    (row.status == TaskStatus.NEEDS_DECISION || row.status == TaskStatus.IN_REVIEW) && changed ->
                        moved += item(row.status.wire, row.status.title, row.statusSince)
                    row.status == TaskStatus.CANCELLED -> Unit
                    else -> row.createdAt?.let { filed ->
                        if (reads.isUnread(row.id, filed)) created += item("created", null, filed)
                    }
                }
            }
            val activity = mutableListOf<Activity>()
            for (row in rows) {
                if (row.status == TaskStatus.CANCELLED) continue
                val written = (notes[row.id].orEmpty())
                    .filter { listed(it.kind) && reads.isUnread(row.id, it.atMs) }
                    .sortedByDescending { it.atMs }
                val newest = written.firstOrNull() ?: continue
                activity += Activity(row.id, row.key, row.title, newest.id, newest.kind, oneRun(newest.body), newest.atMs, written.size - 1)
            }
            fun newest(items: List<Item>) = items.sortedByDescending { it.atMs }
            return BoardSummary(newest(finished), newest(moved), newest(created), activity.sortedByDescending { it.atMs })
        }

        /** A note's words on one line. */
        fun oneRun(body: String): String = body.split(Regex("\\s+")).filter { it.isNotEmpty() }.joinToString(" ")

        /** What Mark All as Read says it will do. On a runner that keeps the state it clears every device. */
        fun markAllReadMessage(tasks: Int, everywhere: Boolean): String {
            val what = if (tasks == 1) "1 task will be marked as read" else "$tasks tasks will be marked as read"
            return if (everywhere) "$what on all your devices." else "$what."
        }
    }
}

/** What the notes of a task say, from `task.get`: the part Unread reads. */
enum class TaskNoteKind(val wire: String, val title: String) {
    DECISION("decision", "Decision"),
    FINDING("finding", "Finding"),
    QUESTION("question", "Question"),
    ANSWER("answer", "Answer"),
    PROGRESS("progress", "Progress"),
    COMMENT("comment", "Comment"),
    STATUS_CHANGE("status_change", "Status change"),
    CREATED("created", "Created"),
    WAIT("wait", "Start"),
    WORKER("worker", "Subagent");

    /** Whether a person wrote this or a transaction did. */
    val isMachineWritten: Boolean get() = this == STATUS_CHANGE || this == CREATED || this == WAIT || this == WORKER

    companion object {
        fun parse(word: String): TaskNoteKind? = entries.firstOrNull { it.wire == word }
    }
}

/** One note, as Unread reads it. */
data class TaskNoteRow(val id: String, val kind: TaskNoteKind, val atMs: Long, val body: String)

object TaskNotes {
    private val json = Json { ignoreUnknownKeys = true; isLenient = true }

    /** `task.get`'s notes. One of a kind this build can't read is skipped, not fatal. */
    fun decode(text: String): List<TaskNoteRow> = decode(json.parseToJsonElement(text).jsonObject)

    fun decode(body: JsonObject): List<TaskNoteRow> = (body["notes"] as? JsonArray).orEmpty().mapNotNull { element ->
        val o = element as? JsonObject ?: return@mapNotNull null
        val kind = o["kind"]?.jsonPrimitive?.contentOrNull?.let(TaskNoteKind::parse) ?: return@mapNotNull null
        TaskNoteRow(
            id = o["id"]?.jsonPrimitive?.contentOrNull ?: return@mapNotNull null,
            kind = kind,
            atMs = o["at"]?.jsonPrimitive?.longOrNull ?: return@mapNotNull null,
            body = o["body"]?.jsonPrimitive?.contentOrNull ?: "",
        )
    }
}
