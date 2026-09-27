package com.farcooler.net

import com.farcooler.model.BoardNotice
import com.farcooler.model.RunnerBoards
import com.farcooler.model.TaskBoard
import com.farcooler.model.WorkspaceSummary
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.Test

/**
 * When a board is read, and how many reads that costs — the rules the iPhone's
 * `Connection.readBoard` follows, with the network replaced by a gate the test
 * opens by hand.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class BoardReadsTest {
    /** A read that waits for the test to let it finish, and counts itself. */
    private class Reads {
        var started = 0
        var inFlight = 0
        var mostAtOnce = 0
        val gates = ArrayDeque<CompletableDeferred<TaskBoard?>>()
        val order = mutableListOf<String>()

        suspend fun read(workspace: WorkspaceSummary): TaskBoard? {
            started += 1
            order += workspace.id
            inFlight += 1
            mostAtOnce = maxOf(mostAtOnce, inFlight)
            val gate = CompletableDeferred<TaskBoard?>()
            gates.addLast(gate)
            try {
                return gate.await()
            } finally {
                inFlight -= 1
            }
        }

        /** Let the oldest read answer. */
        fun answer(board: TaskBoard? = TaskBoard.EMPTY) = gates.removeFirst().complete(board)
    }

    /** A repository's one implicit board, which is how a runner without workspaces keys them. */
    private fun ws(repository: String) = WorkspaceSummary.implicit(repository)

    /** The connection's scope, which outlives the screens that ask for reads. */
    private fun TestScope.connection(): CoroutineScope = backgroundScope

    /**
     * A burst of notices while a read is crossing the network is one read
     * running and ONE after it — not one per notice, and not none.
     */
    @Test
    fun aBurstOfNoticesIsOneReadRunningAndOneAfter() = runTest {
        val reads = Reads()
        val boards = BoardReads(connection(), canRead = { true }, read = reads::read)

        val first = launch { boards.readOne(ws("r")) }
        runCurrent()
        assertEquals(1, reads.started)
        repeat(3) { launch { boards.readOne(ws("r")) } }
        runCurrent()
        assertEquals("folded into the read under way", 1, reads.started)

        reads.answer()
        runCurrent()
        assertEquals("one more after it, for what arrived meanwhile", 2, reads.started)
        reads.answer()
        first.join()
        assertEquals(2, reads.started)
    }

    /**
     * **A screen that leaves mid-read does not lose the read a notice folded
     * into it.** The board screen asks, a `task` notice folds in, the person
     * presses Back before the first read answers: the trailing re-read must
     * still happen, or the front door's counts stay where they were until some
     * later notice for that repository.
     */
    @Test
    fun leavingTheBoardMidReadStillReadsWhatANoticeFoldedIn() = runTest {
        val reads = Reads()
        val boards = BoardReads(connection(), canRead = { true }, read = reads::read)

        val screen = launch { boards.readOne(ws("r")) }
        runCurrent()
        launch { boards.readOne(ws("r")) } // the notice, folded in
        runCurrent()
        screen.cancel() // Back
        runCurrent()

        reads.answer()
        runCurrent()
        assertEquals("the re-read the notice was owed", 2, reads.started)
        reads.answer()
        runCurrent()
        assertTrue(boards.boards.value.containsKey("r"))
    }

    /** A caller that folds in waits until the fresh board has landed — pull to refresh's spinner. */
    @Test
    fun aCallerThatFoldsInWaitsForTheFreshBoard() = runTest {
        val reads = Reads()
        val boards = BoardReads(connection(), canRead = { true }, read = reads::read)

        launch { boards.readOne(ws("r")) }
        runCurrent()
        var refreshed = false
        launch {
            boards.readOne(ws("r"))
            refreshed = true
        }
        runCurrent()
        reads.answer()
        runCurrent()
        assertFalse("still waiting on the re-read it asked for", refreshed)
        reads.answer()
        runCurrent()
        assertTrue(refreshed)
    }

    /** A sweep reads one board at a time, in the runner's order, and records when it ran. */
    @Test
    fun aSweepReadsOneBoardAtATime() = runTest {
        val reads = Reads()
        val boards = BoardReads(connection(), canRead = { true }, read = reads::read, clock = { 42L })

        val sweep = launch { boards.sweep(listOf(ws("a"), ws("b"), ws("c"))) }
        runCurrent()
        while (reads.gates.isNotEmpty()) {
            reads.answer()
            runCurrent()
        }
        sweep.join()
        assertEquals(listOf("a", "b", "c"), reads.order)
        assertEquals("never more than one board read on the wire", 1, reads.mostAtOnce)
        assertTrue(boards.ledger.sweptOnThisLink)
        assertEquals(42L, boards.lastSweepAt)
    }

    /** A failed read keeps the board last read and says it is behind. */
    @Test
    fun aFailedReadKeepsTheLastGoodBoard() = runTest {
        val reads = Reads()
        val boards = BoardReads(connection(), canRead = { true }, read = reads::read)

        val good = launch { boards.readOne(ws("r")) }
        runCurrent()
        reads.answer()
        good.join()

        val bad = launch { boards.readOne(ws("r")) }
        runCurrent()
        reads.answer(null)
        bad.join()

        assertEquals(TaskBoard.EMPTY, boards.boards.value["r"])
        assertTrue("r" in boards.unread.value)
    }

    /**
     * A sweep that cannot read — the build is not in yet — reads nothing,
     * records neither a sweep nor a time, and leaves the link owing one, so the
     * build landing reads the boards; once read, a build landing again does
     * not read them twice.
     */
    @Test
    fun aBuildThatLandsLateSweepsOnlyTheLinkThatOwesOne() = runTest {
        val reads = Reads()
        var buildIn = false
        val boards = BoardReads(connection(), canRead = { buildIn }, read = reads::read, clock = { 7L })
        boards.ledger.linkCameUp()

        boards.sweep(listOf(ws("a"), ws("b")))
        assertEquals(0, reads.started)
        assertEquals("a refused sweep is not stamped", 0L, boards.lastSweepAt)
        assertTrue(boards.ledger.owedWhenBuildLands)

        buildIn = true
        val landed = launch { boards.buildLanded(listOf(ws("a"))) }
        runCurrent()
        reads.answer()
        landed.join()
        assertEquals(1, reads.started)
        assertFalse(boards.ledger.owedWhenBuildLands)

        boards.buildLanded(listOf(ws("a")))
        assertEquals("already swept on this link", 1, reads.started)
    }

    /**
     * **A build landing on the ordinary path starts no sweep of its own**
     * (ov-20 R-M8). A link-up reads the build and then sweeps; the build
     * landing first started a sweep too, and every connect read every board
     * twice. Only a sweep already refused is owed.
     *
     * Mutation: `owedWhenBuildLands` back to `!sweptOnThisLink`. Red: 1 read.
     */
    @Test
    fun aBuildLandingBeforeTheSweepReadsNothing() = runTest {
        val reads = Reads()
        val boards = BoardReads(connection(), canRead = { true }, read = reads::read)
        boards.ledger.linkCameUp()

        val landed = launch { boards.buildLanded(listOf(ws("a"))) }
        runCurrent()
        assertEquals("the link-up's own sweep is on its way", 0, reads.started)
        landed.join()
    }

    /**
     * **A sweep stops at the link it started on** (ov-20 R-M8). One still
     * going when its link dropped read on over the new link, beside the new
     * link's own sweep.
     *
     * Mutation: `isCurrent` answering true for any link. Red: b is read.
     */
    @Test
    fun aSweepStopsWhenItsLinkIsGone() = runTest {
        val reads = Reads()
        val boards = BoardReads(connection(), canRead = { true }, read = reads::read)
        boards.ledger.linkCameUp()

        val sweep = launch { boards.sweep(listOf(ws("a"), ws("b"))) }
        runCurrent()
        boards.ledger.linkCameUp()
        reads.answer()
        runCurrent()
        // Asserted before anything waits on the sweep: a sweep that read on
        // would sit on b's unanswered read, and a join here would hang the
        // test into runTest's timeout instead of failing on the reason.
        assertEquals("b was read after its link went", listOf("a"), reads.order)
        while (reads.gates.isNotEmpty()) {
            reads.answer()
            runCurrent()
        }
        sweep.join()
    }

    // ---- boards by workspace ----

    private val main = WorkspaceSummary(id = "w-main", name = "Main", isMain = true, repository = "r1")
    private val billing = WorkspaceSummary(id = "w-billing", name = "Billing", ordinal = 1, repository = "r1")

    /**
     * A workspace's board is read by naming the workspace: `task.list` with the
     * repository AND the workspace. An implicit board — a runner without
     * workspaces — names only the repository, which is its whole board.
     */
    @Test
    fun aBoardIsReadByWorkspace() {
        assertEquals(
            buildJsonObject {
                put("repository", "r1")
                put("workspace", "w-billing")
            },
            BoardReads.request(billing),
        )
        assertEquals(buildJsonObject { put("repository", "r1") }, BoardReads.request(ws("r1")))
    }

    /** Each board lands under its own workspace, and one failing marks only that one. */
    @Test
    fun twoBoardsInOneRepositoryAreKeptApart() = runTest {
        val reads = Reads()
        val boards = BoardReads(connection(), canRead = { true }, read = reads::read)
        val a = launch { boards.readOne(main) }
        runCurrent()
        reads.answer()
        a.join()
        val b = launch { boards.readOne(billing) }
        runCurrent()
        reads.answer(null)
        b.join()
        assertEquals(setOf("w-main"), boards.boards.value.keys)
        assertEquals(setOf("w-billing"), boards.unread.value)
    }

    /**
     * A notice reads the board it names and, on a move, the board it left —
     * not the other workspace's in the same repository.
     */
    @Test
    fun aNoticeReadsOnlyTheBoardsItMoved() = runTest {
        val reads = Reads()
        val boards = BoardReads(connection(), canRead = { true }, read = reads::read)
        val held = listOf(main, billing)

        val own = launch { boards.noticed(BoardNotice("r1", "w-billing"), held) }
        runCurrent()
        reads.answer()
        runCurrent()
        // Asked before waiting: a notice that read Main's board too would be
        // waiting on a second read here, and the order says which.
        assertEquals(listOf("w-billing"), reads.order)
        assertTrue("no other board is being read", reads.gates.isEmpty())
        own.join()

        val move = launch { boards.noticed(BoardNotice("r1", "w-main", fromWorkspace = "w-billing"), held) }
        runCurrent()
        while (reads.gates.isNotEmpty()) {
            reads.answer()
            runCurrent()
        }
        move.join()
        assertEquals(listOf("w-billing", "w-main", "w-billing"), reads.order)
    }

    /**
     * **A board restored from a stack saved before workspaces reads again.**
     * `Route.Board` named the repository, which opens its implicit board; the
     * runner's list is by workspace and never names it. A notice from that
     * repository still reads it, and so does a sweep.
     */
    @Test
    fun aBoardRestoredByRepositoryReadsAgainOnItsRepositorysNews() = runTest {
        val reads = Reads()
        val boards = BoardReads(connection(), canRead = { true }, read = reads::read)
        val held = listOf(main, billing)
        suspend fun drain() {
            runCurrent()
            while (reads.gates.isNotEmpty()) {
                reads.answer()
                runCurrent()
            }
        }

        val opened = launch { boards.readOne(ws("r1")) }
        drain()
        opened.join()

        val news = launch { boards.noticed(BoardNotice("r1", "w-main"), held) }
        drain()
        news.join()
        assertEquals(listOf("r1", "w-main", "r1"), reads.order)

        // Another repository's news leaves it alone.
        val elsewhere = launch { boards.noticed(BoardNotice("r2", "w-other"), held) }
        drain()
        elsewhere.join()
        assertEquals(listOf("r1", "w-main", "r1", "w-other"), reads.order)

        val sweep = launch { boards.sweep(held) }
        drain()
        sweep.join()
        assertEquals(listOf("w-main", "w-billing", "r1"), reads.order.takeLast(3))
    }

    /**
     * **A reconnect reads every workspace's board again**, whatever notices
     * the dropped link lost: the sweep a new link starts is over every board
     * the runner keeps ([RunnerBoards.boards]), so a board open on screen —
     * Billing's here — is among them, and not only the repository's.
     */
    @Test
    fun aReconnectReadsEveryWorkspacesBoard() = runTest {
        val reads = Reads()
        val boards = BoardReads(connection(), canRead = { true }, read = reads::read)
        boards.ledger.linkCameUp()

        val sweep = launch { boards.sweep(RunnerBoards.boards(listOf("r1"), listOf(billing, main))) }
        runCurrent()
        while (reads.gates.isNotEmpty()) {
            reads.answer()
            runCurrent()
        }
        sweep.join()
        assertEquals(listOf("w-main", "w-billing"), reads.order)
        assertEquals(setOf("w-main", "w-billing"), boards.boards.value.keys)
        assertFalse(boards.ledger.owedWhenBuildLands)
    }

    /**
     * **News that arrived while away is owed one sweep on return, and any
     * sweep pays it.** A reconnect's own sweep reads every board, so the
     * return to the foreground after it has nothing left to read. Before, only
     * the return's Connected branch paid it, and a reconnect in between left
     * it standing for a wasted sweep at some later, unrelated return.
     */
    @Test
    fun aSweepPaysForNewsThatArrivedWhileAway() = runTest {
        val reads = Reads()
        val boards = BoardReads(connection(), canRead = { true }, read = reads::read)

        boards.newsWhileAway()
        val reconnect = launch { boards.sweep(listOf(ws("r"))) }
        runCurrent()
        reads.answer()
        runCurrent()
        reconnect.join()
        assertEquals(1, reads.started)

        val back = launch { boards.cameBack(listOf(ws("r"))) }
        runCurrent()
        assertEquals("the reconnect's sweep already read it", 1, reads.started)
        back.join()
    }

    /** A sweep the runner refused reads nothing, so the debt still stands. */
    @Test
    fun aRefusedSweepLeavesTheNewsOwed() = runTest {
        val reads = Reads()
        var connected = false
        val boards = BoardReads(connection(), canRead = { connected }, read = reads::read)

        boards.newsWhileAway()
        boards.sweep(listOf(ws("r")))
        assertEquals(0, reads.started)

        connected = true
        val back = launch { boards.cameBack(listOf(ws("r"))) }
        runCurrent()
        assertEquals("owed from before the refused sweep", 1, reads.started)
        reads.answer()
        back.join()
    }

    /**
     * **A closed connection's reads stop with it.** A read still crossing the
     * network when [BoardReads.close] runs lands nowhere, and a notice folded
     * into it owes no re-read: nobody is reading this connection's boards any
     * more, and its flows must not move after the caller let it go.
     */
    @Test
    fun closingStopsTheReadsUnderWay() = runTest {
        val reads = Reads()
        val boards = BoardReads(connection(), canRead = { true }, read = reads::read)

        launch { boards.readOne(ws("r")) }
        runCurrent()
        launch { boards.readOne(ws("r")) }
        runCurrent()
        assertEquals(1, reads.started)

        boards.close()
        runCurrent()
        reads.gates.forEach { it.complete(TaskBoard.EMPTY) }
        runCurrent()
        assertTrue("a read landed after close", boards.boards.value.isEmpty())
        assertEquals("a folded notice re-read after close", 1, reads.started)

        launch { boards.readOne(ws("r")) }
        runCurrent()
        assertEquals("a read started after close", 1, reads.started)
    }
}
