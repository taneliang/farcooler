package com.farcooler.net

import com.farcooler.model.BoardSweep
import com.farcooler.model.TaskBoard
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * One runner's boards, and the rules for reading them — the iPhone's
 * `Connection.readBoard` / `loadBoards`, with the network taken out so a JVM
 * can prove them (`BoardReadsTest`).
 *
 * - **One read per board at a time, and one more after it if anything moved
 *   meanwhile.** A manager moving six cards is six `task` notices; the core
 *   coalesces the ones still queued, and this folds the ones that arrive while
 *   a read is crossing the network.
 * - **A sweep reads one board at a time.** The client core holds this runner's
 *   session for the whole of each round trip, so calls go over the wire one
 *   after another whatever this side does. Starting several would buy no speed
 *   and only put several board reads ahead of the next keystroke.
 * - **A failed read keeps the last good board** and marks it unread, and is
 *   never taken for a dropped link.
 * - **[sweep] records that this link's boards were read** ([BoardSweep]), so a
 *   build that lands late can tell whether it owes a sweep.
 *
 * Not thread-safe, and not meant to be: every caller is on the app's main
 * dispatcher, where `Connection` lives.
 */
class BoardReads(
    /** Whether a read may go out now: connected, and the runner keeps a board. */
    private val canRead: () -> Boolean,
    /** Read one repository's board, or null when the read failed. */
    private val read: suspend (repository: String) -> TaskBoard?,
) {
    private val _boards = MutableStateFlow<Map<String, TaskBoard>>(emptyMap())

    /** Each repository's board as last read. Kept through a failed read and a reconnect. */
    val boards: StateFlow<Map<String, TaskBoard>> = _boards.asStateFlow()

    private val _unread = MutableStateFlow<Set<String>>(emptySet())

    /** Repositories whose last read failed. */
    val unread: StateFlow<Set<String>> = _unread.asStateFlow()

    /** Whether this link's boards were read. See [BoardSweep]. */
    val ledger = BoardSweep()

    private val reading = mutableSetOf<String>()
    private val movedAgain = mutableSetOf<String>()

    /** Read one board, or fold into the read already under way. */
    suspend fun readOne(repository: String) {
        if (repository in reading) {
            movedAgain += repository
            return
        }
        reading += repository
        try {
            do {
                movedAgain -= repository
                if (!canRead()) return
                val board = read(repository)
                if (board != null) {
                    _boards.value = _boards.value + (repository to board)
                    _unread.value = _unread.value - repository
                } else {
                    _unread.value = _unread.value + repository
                }
            } while (repository in movedAgain)
        } finally {
            reading -= repository
        }
    }

    /** Read every board, one after another. */
    suspend fun sweep(repositories: List<String>) {
        if (!canRead()) return
        if (repositories.isNotEmpty()) ledger.swept()
        for (repository in repositories) readOne(repository)
    }
}
