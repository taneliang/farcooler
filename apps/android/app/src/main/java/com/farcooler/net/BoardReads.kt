package com.farcooler.net

import com.farcooler.model.BoardSweep
import com.farcooler.model.TaskBoard
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/**
 * One runner's boards, and the rules for reading them — the iPhone's
 * `Connection.readBoard` / `loadBoards`, with the network taken out so a JVM
 * can prove them (`BoardReadsTest`).
 *
 * - **One read per board at a time, and one more after it if anything moved
 *   meanwhile.** A manager moving six cards is six `task` notices; the core
 *   coalesces the ones still queued, and this folds the ones that arrive while
 *   a read is crossing the network.
 * - **A read belongs to the connection, not to whoever asked for it.** It runs
 *   in [scope], and a caller only waits for it. A board screen that is left
 *   mid-read cancels its wait, never the read — so a notice folded into that
 *   read still gets its trailing re-read, instead of the row's counts staying
 *   where they were until some later notice.
 * - **A caller that folds in waits for the read it folded into**, trailing
 *   re-read included, so pull to refresh stops spinning when the fresh board
 *   has actually landed.
 * - **A sweep reads one board at a time.** The client core holds this runner's
 *   session for the whole of each round trip, so calls go over the wire one
 *   after another whatever this side does. Starting several would buy no speed
 *   and only put several board reads ahead of the next keystroke.
 * - **A failed read keeps the last good board** and marks it unread, and is
 *   never taken for a dropped link.
 * - **A sweep that ran is recorded** — [ledger] for the late-build rule, and
 *   [lastSweepAt] for the once-a-minute sweep without an event channel — and a
 *   sweep refused because the runner could not be read yet records neither.
 *
 * Not thread-safe, and not meant to be: every caller, and [scope], is on the
 * app's main dispatcher, where `Connection` lives.
 */
class BoardReads(
    /** Where reads run: the connection's own scope, which outlives every screen. */
    private val scope: CoroutineScope,
    /** Whether a read may go out now: connected, and the runner keeps a board. */
    private val canRead: () -> Boolean,
    /** Read one repository's board, or null when the read failed. */
    private val read: suspend (repository: String) -> TaskBoard?,
    private val clock: () -> Long = System::currentTimeMillis,
) {
    private val _boards = MutableStateFlow<Map<String, TaskBoard>>(emptyMap())

    /** Each repository's board as last read. Kept through a failed read and a reconnect. */
    val boards: StateFlow<Map<String, TaskBoard>> = _boards.asStateFlow()

    private val _unread = MutableStateFlow<Set<String>>(emptySet())

    /** Repositories whose last read failed. */
    val unread: StateFlow<Set<String>> = _unread.asStateFlow()

    /** Whether this link's boards were read. See [BoardSweep]. */
    val ledger = BoardSweep()

    /** When a sweep last actually ran, or 0 for never. */
    var lastSweepAt: Long = 0L
        private set

    private val running = mutableMapOf<String, Job>()
    private val movedAgain = mutableSetOf<String>()

    /**
     * Read one board, or fold into the read already under way — and wait, in
     * either case, until that read (and any re-read it owes) has landed.
     */
    suspend fun readOne(repository: String) {
        running[repository]?.let { inFlight ->
            movedAgain += repository
            inFlight.join()
            return
        }
        // Lazy, and recorded before it starts: on an immediate dispatcher the
        // body could otherwise run to the end — `canRead` false, say — before
        // `running` knew about it, and leave a finished job standing there.
        val job = scope.launch(start = CoroutineStart.LAZY) {
            try {
                do {
                    movedAgain -= repository
                    if (!canRead()) return@launch
                    val board = read(repository)
                    if (board != null) {
                        _boards.value = _boards.value + (repository to board)
                        _unread.value = _unread.value - repository
                    } else {
                        _unread.value = _unread.value + repository
                    }
                } while (repository in movedAgain)
            } finally {
                running -= repository
            }
        }
        running[repository] = job
        job.start()
        job.join()
    }

    /** Read every board, one after another. */
    suspend fun sweep(repositories: List<String>) {
        if (!canRead()) return
        lastSweepAt = clock()
        if (repositories.isNotEmpty()) ledger.swept()
        for (repository in repositories) readOne(repository)
    }

    /**
     * The runner's build just landed on this link. Sweep if this link's boards
     * were never read — a sweep before the build could not tell the runner
     * keeps a board, and read nothing.
     */
    suspend fun buildLanded(repositories: List<String>) {
        if (ledger.owedWhenBuildLands) sweep(repositories)
    }
}
