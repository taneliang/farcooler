package com.farcooler.net

import com.farcooler.core.TerminalTransport
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/**
 * The host calls a person makes by tapping a row or a pane control, with the
 * failure of each said aloud (ov-180).
 *
 * These five used to run inside `attempt {}` and drop the result: a restart
 * the runner refused, a hide that didn't take and a reorder that snapped back
 * all looked exactly like success. They live here rather than in [Connection]
 * because that object owns a JNI core and cannot be built in a unit test; this
 * one takes any [TerminalTransport].
 */
class HostActions(private val core: TerminalTransport, val notices: ActionNotices) {
    suspend fun act(action: Connection.Action, terminalId: String) {
        val (method, context) = when (action) {
            Connection.Action.RESTART -> "terminal.restart" to "Couldn’t restart this terminal."
            Connection.Action.STOP -> "terminal.stop" to "Couldn’t stop this terminal."
            Connection.Action.DISMISS_LOST ->
                "terminal.dismiss_lost" to "Couldn’t dismiss this terminal."
        }
        notices.run(context) { core.call(method, Connection.args("terminal" to terminalId)) }
    }

    suspend fun setHidden(worktreeId: String, hidden: Boolean) {
        val method = if (hidden) "worktree.hide" else "worktree.unhide"
        val context =
            if (hidden) "Couldn’t hide this worktree." else "Couldn’t unhide this worktree."
        notices.run(context) { core.call(method, Connection.args("worktree" to worktreeId)) }
    }

    suspend fun reorder(ordered: List<String>) {
        val payload = JsonObject(
            mapOf("worktrees" to JsonArray(ordered.map { JsonPrimitive(it) }))
        )
        notices.run("Couldn’t save the new order.") { core.call("worktree.reorder", payload) }
    }

    suspend fun setPaneMode(terminalId: String, mode: String) {
        val context = "Couldn’t switch this pane to its " +
            (if (mode == "terminal") "terminal." else "chat.")
        notices.run(context) {
            core.call(
                "terminal.set_pane_mode",
                Connection.args("terminal" to terminalId, "paneMode" to mode),
            )
        }
    }
}
