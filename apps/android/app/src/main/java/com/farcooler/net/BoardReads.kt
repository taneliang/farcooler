package com.farcooler.net

import com.farcooler.model.BoardNotice
import com.farcooler.model.BoardSweep
import com.farcooler.model.RunnerBoards
import com.farcooler.model.TaskBoard
import com.farcooler.model.WorkspaceSummary
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

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
 *   in [scope], under a job [close] cancels when the connection closes, and a
 *   caller only waits for it. A board screen that is left
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
 * - **A board is a workspace's.** Boards are keyed by workspace id, and a
 *   read names the workspace ([request]); on a runner without `workstreams`
 *   the workspace is implicit, its id the repository's, and the read is the
 *   whole repository's, exactly as before workspaces.
 *
 * Not thread-safe, and not meant to be: every caller, and [scope], is on the
 * app's main dispatcher, where `Connection` lives.
 */
class BoardReads(
    /** Where reads run: the connection's own scope, which outlives every screen. */
    private val scope: CoroutineScope,
    /** Whether a read may go out now: connected, and the runner keeps a board. */
    private val canRead: () -> Boolean,
    /** Read one workspace's board, or null when the read failed. */
    private val read: suspend (workspace: WorkspaceSummary) -> TaskBoard?,
    private val clock: () -> Long = System::currentTimeMillis,
) {
    private val _boards = MutableStateFlow<Map<String, TaskBoard>>(emptyMap())

    /** Each board as last read, by workspace id. Kept through a failed read and a reconnect. */
    val boards: StateFlow<Map<String, TaskBoard>> = _boards.asStateFlow()

    private val _unread = MutableStateFlow<Set<String>>(emptySet())

    /** Boards, by workspace id, whose last read failed. */
    val unread: StateFlow<Set<String>> = _unread.asStateFlow()

    /** Whether this link's boards were read. See [BoardSweep]. */
    val ledger = BoardSweep()

    /** When a sweep last actually ran, or 0 for never. */
    var lastSweepAt: Long = 0L
        private set

    /**
     * The parent of every read, a child of [scope]'s job: the app's teardown
     * cancels it, and so does [close], which is this one connection's.
     */
    private val reads = SupervisorJob(scope.coroutineContext[Job])

    /** Whether news arrived while away that no sweep has read yet. See [newsWhileAway]. */
    private var owedOnReturn = false

    private val running = mutableMapOf<String, Job>()
    private val movedAgain = mutableSetOf<String>()

    /**
     * Every implicit board read on this link, by id. Most are among the boards
     * the connection lists; one that is not was opened by a route saved before
     * workspaces (`Route.Board` by repository id), whose board is the whole
     * repository's and which no workspace-keyed list will ever name again.
     * [noticed] and [sweep] read it with the rest, so it does not sit on
     * screen as it was first read.
     */
    private val implicitRead = mutableMapOf<String, WorkspaceSummary>()

    /** [boards], and every implicit board read on this link that they leave out. */
    private fun withImplicit(boards: List<WorkspaceSummary>): List<WorkspaceSummary> {
        val listed = boards.map { it.id }.toSet()
        return boards + implicitRead.values.filter { it.id !in listed }
    }

    /**
     * Read one board, or fold into the read already under way — and wait, in
     * either case, until that read (and any re-read it owes) has landed.
     */
    suspend fun readOne(workspace: WorkspaceSummary) {
        val key = workspace.id
        if (workspace.isImplicit) implicitRead[key] = workspace
        running[key]?.let { inFlight ->
            movedAgain += key
            inFlight.join()
            return
        }
        // Lazy, and recorded before it starts: on an immediate dispatcher the
        // body could otherwise run to the end — `canRead` false, say — before
        // `running` knew about it, and leave a finished job standing there.
        val job = scope.launch(reads, start = CoroutineStart.LAZY) {
            try {
                do {
                    movedAgain -= key
                    if (!canRead()) return@launch
                    val board = read(workspace)
                    if (board != null) {
                        _boards.value = _boards.value + (key to board)
                        _unread.value = _unread.value - key
                    } else {
                        _unread.value = _unread.value + key
                    }
                } while (key in movedAgain)
            } finally {
                running -= key
            }
        }
        running[key] = job
        job.start()
        job.join()
    }

    /**
     * Read every board, one after another: what a link coming up, a reconnect
     * and a `resync` do, over [RunnerBoards.boards] — every workspace's board,
     * so an open one is read again whatever notices a dropped link lost.
     */
    suspend fun sweep(boards: List<WorkspaceSummary>) {
        if (!canRead()) {
            ledger.refused()
            return
        }
        lastSweepAt = clock()
        owedOnReturn = false
        val link = ledger.link
        if (boards.isNotEmpty()) ledger.swept()
        for (board in withImplicit(boards)) {
            // Its link is gone: the new one has its own sweep.
            if (!ledger.isCurrent(link)) return
            readOne(board)
        }
    }

    /**
     * A `task` notice: read the boards it moved, of [boards] — the board the
     * task is on and, on a move, the one it left, never another workspace's.
     * See [RunnerBoards.touched].
     */
    suspend fun noticed(notice: BoardNotice, boards: List<WorkspaceSummary>) {
        for (board in RunnerBoards.touched(notice, withImplicit(boards))) readOne(board)
    }

    /**
     * The runner's build just landed on this link. Sweep if this link's boards
     * were never read — a sweep before the build could not tell the runner
     * keeps a board, and read nothing.
     */
    suspend fun buildLanded(boards: List<WorkspaceSummary>) {
        if (ledger.owedWhenBuildLands) sweep(boards)
    }

    /**
     * A `task` or `resync` notice arrived while the app was away, and was not
     * read. One sweep on return pays for it — or any sweep before then, since
     * a sweep reads every board: a reconnect's, most often. See [cameBack].
     */
    fun newsWhileAway() {
        owedOnReturn = true
    }

    /** The app is back: sweep if news arrived while it was away and no sweep has read it since. */
    suspend fun cameBack(boards: List<WorkspaceSummary>) {
        if (owedOnReturn) sweep(boards)
    }

    /**
     * The connection is closed: stop every read under way, and start none.
     * A read still crossing the network lands nowhere, and a notice folded
     * into it owes nothing, because nobody reads this connection's boards
     * again.
     */
    fun close() {
        reads.cancel()
    }

    companion object {
        /**
         * The `task.list` arguments for [workspace]'s board: its repository,
         * and the workspace itself unless it is implicit — whose board is the
         * whole repository's. The client core drops the workspace for a runner
         * without `workstreams`, and makes one with it refuse rather than widen.
         */
        fun request(workspace: WorkspaceSummary): JsonObject = buildJsonObject {
            put("repository", workspace.repository ?: workspace.id)
            workspace.boardWorkspace?.let { put("workspace", it) }
        }
    }
}
