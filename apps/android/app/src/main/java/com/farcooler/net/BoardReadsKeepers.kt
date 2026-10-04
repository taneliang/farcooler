package com.farcooler.net

import com.farcooler.model.BoardReads
import com.farcooler.model.BoardReadsKeeper
import com.farcooler.model.BoardReadsStore
import com.farcooler.model.ReadsRaise
import com.farcooler.model.TaskRow
import com.farcooler.model.WireBoardReads
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject

/**
 * One runner's read state, a [BoardReadsKeeper] per board (ov-113): built when
 * the board is first read, from what this phone kept for it under this runner's
 * id, with the state each holds published for the screens ([reads]). The rules
 * are the keeper's, and `BoardReadsKeeperTest` holds them.
 *
 * Not thread-safe, like the keepers: every caller is on the main dispatcher.
 */
class BoardReadsKeepers(
    private val store: BoardReadsStore,
    private val host: String,
    private val scope: CoroutineScope,
    /** Tell the runner a raise on a board (`workspace.mark_read`): its answer, or null when it gave none. */
    private val markRead: suspend (workspace: String, raise: ReadsRaise) -> String?,
    /** Whether this phone's grant lets it write the runner's read state (Control). */
    private val mayWrite: () -> Boolean = { true },
    private val clock: () -> Long = System::currentTimeMillis,
) {
    private val keepers = mutableMapOf<String, BoardReadsKeeper>()
    private val _reads = MutableStateFlow<Map<String, BoardReads>>(emptyMap())

    /** What's read on each board read, by workspace id: the runner's state when it keeps it, this phone's when it can't. */
    val reads: StateFlow<Map<String, BoardReads>> = _reads.asStateFlow()

    private fun keeper(workspace: String): BoardReadsKeeper = keepers.getOrPut(workspace) {
        BoardReadsKeeper(store, host, workspace, clock(), scope, mayWrite) { raise -> markRead(workspace, raise) }.also { made ->
            _reads.value = _reads.value + (workspace to made.reads.value)
            scope.launch { made.reads.collect { _reads.value = _reads.value + (workspace to it) } }
        }
    }

    /** A board read's text: its `reads` adopted by the board's keeper. */
    fun adopt(workspace: String, board: String) = keeper(workspace).adopt(board)

    /**
     * Another device read something: the runner's `reads` event, for a board
     * this phone has read. One it hasn't gets the state with its first read.
     */
    fun hear(notice: JsonObject) {
        val wire = WireBoardReads.ofState(notice) ?: return
        keepers.entries.firstOrNull { it.key.equals(wire.workspaceId, ignoreCase = true) }?.value?.heard(wire)
    }

    /** [row] was opened on [workspace]'s board: read, through the newest of its notes read ([latestMs]). */
    fun open(workspace: String, row: TaskRow, latestMs: Long? = null) = keeper(workspace).open(row, latestMs, clock())

    /** Mark All as Read, once the person said yes. */
    fun markAllRead(workspace: String, rows: List<TaskRow>, latestMs: Long?) =
        keeper(workspace).markAllRead(rows, latestMs, clock())

    /** Whether marking something read there reaches every device: the runner keeps this board's state. */
    fun areShared(workspace: String): Boolean = keepers[workspace]?.readsAreShared == true

    /** Send everything owed, and wait. For tests, and for a connection closing. */
    suspend fun flush() = keepers.values.forEach { it.flush() }
}
