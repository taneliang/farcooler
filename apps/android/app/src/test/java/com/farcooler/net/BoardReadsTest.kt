package com.farcooler.net

import com.farcooler.model.TaskBoard
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

        suspend fun read(repository: String): TaskBoard? {
            started += 1
            order += repository
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

        val first = launch { boards.readOne("r") }
        runCurrent()
        assertEquals(1, reads.started)
        repeat(3) { launch { boards.readOne("r") } }
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

        val screen = launch { boards.readOne("r") }
        runCurrent()
        launch { boards.readOne("r") } // the notice, folded in
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

        launch { boards.readOne("r") }
        runCurrent()
        var refreshed = false
        launch {
            boards.readOne("r")
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

        val sweep = launch { boards.sweep(listOf("a", "b", "c")) }
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

        val good = launch { boards.readOne("r") }
        runCurrent()
        reads.answer()
        good.join()

        val bad = launch { boards.readOne("r") }
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

        boards.sweep(listOf("a", "b"))
        assertEquals(0, reads.started)
        assertEquals("a refused sweep is not stamped", 0L, boards.lastSweepAt)
        assertTrue(boards.ledger.owedWhenBuildLands)

        buildIn = true
        val landed = launch { boards.buildLanded(listOf("a")) }
        runCurrent()
        reads.answer()
        landed.join()
        assertEquals(1, reads.started)
        assertFalse(boards.ledger.owedWhenBuildLands)

        boards.buildLanded(listOf("a"))
        assertEquals("already swept on this link", 1, reads.started)
    }
}
