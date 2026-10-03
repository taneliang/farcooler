package com.farcooler.ui

import android.content.Context
import android.content.SharedPreferences
import com.farcooler.model.BoardReads
import com.farcooler.model.BoardReadsStore

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
        prefs.edit()
            .putLong(floorKey(host, workspace), kept.floorMs)
            .putStringSet(openedKey(host, workspace), kept.opened.map { (id, ms) -> "$id=$ms" }.toSet())
            .apply()
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
    }
}
