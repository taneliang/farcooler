package com.farcooler.ui

import android.content.Context
import android.content.SharedPreferences
import com.farcooler.model.BoardReads
import com.farcooler.model.BoardReadsStore
import com.farcooler.model.ReadsRaise

/**
 * A board's read state in this phone's preferences (ov-104), keyed by runner
 * and workspace: a floor, and each opened ticket's mark as `<task id>=<ms>`.
 * The seam a runner-synced store replaces later is [BoardReadsStore].
 */
class PrefsBoardReads(private val prefs: SharedPreferences) : BoardReadsStore {
    override fun load(host: String, workspace: String, nowMs: Long): BoardReads {
        val floorKey = floorKey(host, workspace)
        if (!prefs.contains(floorKey)) {
            val first = BoardReads.firstLook(nowMs)
            save(first, host, workspace)
            // Made up here, not read by anyone: never sent to a runner (ov-113).
            prefs.edit().putBoolean(inventedKey(host, workspace), true).apply()
            return first
        }
        val opened = prefs.getStringSet(openedKey(host, workspace), emptySet()).orEmpty().mapNotNull { entry ->
            val at = entry.lastIndexOf('=')
            if (at <= 0) null else entry.substring(at + 1).toLongOrNull()?.let { entry.substring(0, at) to it }
        }.toMap()
        return BoardReads(prefs.getLong(floorKey, nowMs), opened)
    }

    override fun save(reads: BoardReads, host: String, workspace: String) {
        val kept = reads.pruned()
        // A floor that moved is somebody's doing, no longer made up.
        if (prefs.contains(floorKey(host, workspace)) && prefs.getLong(floorKey(host, workspace), 0L) != kept.floorMs) {
            prefs.edit().remove(inventedKey(host, workspace)).apply()
        }
        prefs.edit()
            .putLong(floorKey(host, workspace), kept.floorMs)
            .putStringSet(openedKey(host, workspace), kept.opened.map { (id, ms) -> "$id=$ms" }.toSet())
            .apply()
    }

    override fun keptReads(host: String, workspace: String): BoardReads? {
        if (!prefs.contains(floorKey(host, workspace))) return null
        val state = load(host, workspace, System.currentTimeMillis())
        if (!prefs.getBoolean(inventedKey(host, workspace), false)) return state
        return BoardReads(Long.MIN_VALUE, state.opened).takeIf { it.opened.isNotEmpty() }
    }

    override fun isUploaded(host: String, workspace: String) = prefs.getBoolean(syncedKey(host, workspace), false)

    override fun markUploaded(host: String, workspace: String) {
        prefs.edit().putBoolean(syncedKey(host, workspace), true).apply()
    }

    override fun loadPending(host: String, workspace: String): ReadsRaise {
        val key = pendingKey(host, workspace)
        val opened = prefs.getStringSet("$key.opened", emptySet()).orEmpty().mapNotNull { entry ->
            val at = entry.lastIndexOf('=')
            if (at <= 0) null else entry.substring(at + 1).toLongOrNull()?.let { entry.substring(0, at) to it }
        }.toMap()
        return ReadsRaise(if (prefs.contains("$key.floor")) prefs.getLong("$key.floor", 0L) else null, opened)
    }

    override fun savePending(pending: ReadsRaise, host: String, workspace: String) {
        val key = pendingKey(host, workspace)
        prefs.edit().apply {
            if (pending.floorMs != null) putLong("$key.floor", pending.floorMs) else remove("$key.floor")
            putStringSet("$key.opened", pending.opened.map { (id, ms) -> "$id=$ms" }.toSet())
        }.apply()
    }

    /** Opens [row] on the kept state, and keeps the result. */
    fun open(row: com.farcooler.model.TaskRow, host: String, workspace: String, nowMs: Long = System.currentTimeMillis()): BoardReads {
        val next = load(host, workspace, nowMs).open(row, nowMs)
        save(next, host, workspace)
        return next
    }

    companion object {
        fun of(context: Context): PrefsBoardReads =
            PrefsBoardReads(context.applicationContext.getSharedPreferences("farcooler.boardReads", Context.MODE_PRIVATE))

        fun floorKey(host: String, workspace: String) = "board.read.$host.$workspace.floor"
        fun openedKey(host: String, workspace: String) = "board.read.$host.$workspace.opened"
        private fun inventedKey(host: String, workspace: String) = "board.read.$host.$workspace.floor.invented"
        private fun syncedKey(host: String, workspace: String) = "board.read.$host.$workspace.synced"
        private fun pendingKey(host: String, workspace: String) = "board.read.$host.$workspace.pending"
    }
}
