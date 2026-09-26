package com.farcooler.net

import com.farcooler.model.TaskBoard
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.launch
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
    }

    /**
     * A burst of notices while a read is crossing the network is one read
     * running and ONE after it — not one per notice, and not none.
     */
    @Test
    fun aBurstOfNoticesIsOneReadRunningAndOneAfter() = runTest {
        val reads = Reads()
        val boards = BoardReads(canRead = { true }, read = reads::read)

        val first = launch { boards.readOne("r") }
        runCurrent()
        assertEquals(1, reads.started)
        repeat(3) { launch { boards.readOne("r") } }
        runCurrent()
        assertEquals("folded into the read under way", 1, reads.started)

        reads.gates.removeFirst().complete(TaskBoard.EMPTY)
        runCurrent()
        assertEquals("one more after it, for what arrived meanwhile", 2, reads.started)
        reads.gates.removeFirst().complete(TaskBoard.EMPTY)
        first.join()
        assertEquals(2, reads.started)
    }

    /** A sweep reads one board at a time, in the runner's order. */
    @Test
    fun aSweepReadsOneBoardAtATime() = runTest {
        val reads = Reads()
        val boards = BoardReads(canRead = { true }, read = reads::read)

        val sweep = launch { boards.sweep(listOf("a", "b", "c")) }
        runCurrent()
        while (reads.gates.isNotEmpty()) {
            reads.gates.removeFirst().complete(TaskBoard.EMPTY)
            runCurrent()
        }
        sweep.join()
        assertEquals(listOf("a", "b", "c"), reads.order)
        assertEquals("never more than one board read on the wire", 1, reads.mostAtOnce)
        assertTrue(boards.ledger.sweptOnThisLink)
    }

    /** A failed read keeps the board last read and says it is behind. */
    @Test
    fun aFailedReadKeepsTheLastGoodBoard() = runTest {
        val reads = Reads()
        val boards = BoardReads(canRead = { true }, read = reads::read)

        val good = launch { boards.readOne("r") }
        runCurrent()
        reads.gates.removeFirst().complete(TaskBoard.EMPTY)
        good.join()

        val bad = launch { boards.readOne("r") }
        runCurrent()
        reads.gates.removeFirst().complete(null)
        bad.join()

        assertEquals(TaskBoard.EMPTY, boards.boards.value["r"])
        assertTrue("r" in boards.unread.value)
    }

    /**
     * A sweep that cannot read — the build is not in yet — reads nothing and
     * does not count as this link's sweep, so the build landing reads them.
     */
    @Test
    fun aSweepBeforeTheBuildLeavesTheLinkOwingOne() = runTest {
        val reads = Reads()
        var buildIn = false
        val boards = BoardReads(canRead = { buildIn }, read = reads::read)
        boards.ledger.linkCameUp()

        boards.sweep(listOf("a", "b"))
        assertEquals(0, reads.started)
        assertTrue(boards.ledger.owedWhenBuildLands)

        buildIn = true
        val sweep = launch { boards.sweep(listOf("a")) }
        runCurrent()
        reads.gates.removeFirst().complete(TaskBoard.EMPTY)
        sweep.join()
        assertFalse(boards.ledger.owedWhenBuildLands)
    }
}
