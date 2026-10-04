package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

// Read state kept on the runner (ov-113): the wire shape a runner with
// `board_reads` sends, and the rules for merging it with what this phone has.
// AgentKit's `BoardReadsSync.swift`, rule for rule.
//
// Every value only rises, so merging is order-free and a repeat is harmless.
// Times are the runner's clock; this phone never adds its own `now` to a shared
// mark, which a clock running ahead would turn into news hidden everywhere.

/**
 * One board's read state as a runner sends it: the `reads` of `task.list`'s
 * answer, `workspace.mark_read`'s answer and the `reads` event.
 */
data class WireBoardReads(val workspaceId: String, val floorMs: Long, val opened: Map<String, Long>) {
    /** The state as the rule here keeps it. */
    val reads: BoardReads get() = BoardReads(floorMs, opened).pruned()

    companion object {
        private val json = Json { ignoreUnknownKeys = true; isLenient = true }

        /**
         * The `reads` of a `task.list` answer, or null from a runner that sends
         * none, or for a board that named no workspace. A malformed one is null
         * too: it costs the read state, never the board.
         */
        fun ofBoard(text: String): WireBoardReads? = runCatching {
            ofState(json.parseToJsonElement(text).jsonObject["reads"] as? JsonObject ?: return null)
        }.getOrNull()

        /** A bare state: `workspace.mark_read`'s answer. */
        fun ofState(text: String): WireBoardReads? = runCatching { ofState(json.parseToJsonElement(text).jsonObject) }.getOrNull()

        /** A state as an object, which is also the `reads` event's line. */
        fun ofState(body: JsonObject): WireBoardReads? {
            val workspace = body["workspace_id"]?.jsonPrimitive?.contentOrNull ?: return null
            val floor = body["floor_ms"]?.jsonPrimitive?.longOrNull ?: return null
            val marks = (body["opened"] as? JsonArray).orEmpty().mapNotNull { element ->
                val o = element as? JsonObject ?: return@mapNotNull null
                val id = o["task_id"]?.jsonPrimitive?.contentOrNull ?: return@mapNotNull null
                val at = o["opened_ms"]?.jsonPrimitive?.longOrNull ?: return@mapNotNull null
                id to at
            }
            return WireBoardReads(workspace, floor, marks.groupBy({ it.first }, { it.second }).mapValues { it.value.max() })
        }
    }
}

/**
 * Both devices' words together: each mark and the floor, the later of the
 * two, then without what the floor has passed.
 */
fun BoardReads.merged(other: BoardReads): BoardReads = BoardReads(
    maxOf(floorMs, other.floorMs),
    (opened.keys + other.opened.keys).associateWith { maxOf(opened[it] ?: Long.MIN_VALUE, other.opened[it] ?: Long.MIN_VALUE) },
).pruned()

/**
 * [row] was opened, as far as the runner has told us: through its last word on
 * it ([TaskRow.lastMovedMs]) and the newest note read ([latestMs]). No device
 * clock, so a phone running ahead can't hide what the runner writes next, and a
 * mark already held is never lowered.
 */
fun BoardReads.openSeenThrough(row: TaskRow, latestMs: Long?): BoardReads =
    copy(opened = opened + (row.id to maxOf(opened[row.id] ?: Long.MIN_VALUE, row.lastMovedMs, latestMs ?: Long.MIN_VALUE)))

/**
 * Mark All as Read through what was shown: the rows' last words and the notes
 * read. Raises the floor, never lowers it, and keeps no marks the floor has
 * passed.
 */
fun BoardReads.markAllReadSeenThrough(rows: List<TaskRow>, latestMs: Long?): BoardReads {
    val floor = maxOf(floorMs, rows.maxOfOrNull { it.lastMovedMs } ?: Long.MIN_VALUE, latestMs ?: Long.MIN_VALUE)
    return BoardReads(floor, opened.filterValues { it > floor })
}

/** Mark All as Read on this phone's own clock: what an older runner's phone does. */
fun BoardReads.markAllReadOnDeviceClock(rows: List<TaskRow>, nowMs: Long): BoardReads =
    markAllReadSeenThrough(rows, nowMs)

/** What to tell a runner: `workspace.mark_read`'s floor and opened marks. */
data class ReadsRaise(val floorMs: Long? = null, val opened: Map<String, Long> = emptyMap()) {
    val isEmpty: Boolean get() = floorMs == null && opened.isEmpty()

    /** Both, each value the later. */
    fun merging(other: ReadsRaise): ReadsRaise = ReadsRaise(
        listOfNotNull(floorMs, other.floorMs).maxOrNull(),
        (opened.keys + other.opened.keys).associateWith { maxOf(opened[it] ?: Long.MIN_VALUE, other.opened[it] ?: Long.MIN_VALUE) },
    )

    /** [reads] with this raised into it. */
    fun applied(to: BoardReads): BoardReads = to.merged(BoardReads(floorMs ?: Long.MIN_VALUE, opened))

    /** What is left of this once [sent] has gone: nothing the runner was told at or above its value. */
    fun without(sent: ReadsRaise): ReadsRaise = ReadsRaise(
        floorMs?.takeUnless { f -> sent.floorMs?.let { f <= it } == true },
        opened.filter { (id, at) -> at > (sent.opened[id] ?: Long.MIN_VALUE) },
    )

    /** `workspace.mark_read`'s arguments for [workspace]'s board, as the client core takes them. */
    fun arguments(workspace: String): JsonObject = buildJsonObject {
        put("workspace", workspace)
        floorMs?.let { put("floor_ms", it) }
        put(
            "opened",
            buildJsonArray {
                for ((id, at) in opened.entries.sortedBy { it.key }) {
                    add(buildJsonObject { put("task_id", id); put("opened_ms", at) })
                }
            },
        )
    }
}
