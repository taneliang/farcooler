package com.farcooler.model

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A phone's board reads (ov-113): the runner's state, marks and Mark All as Read
 * sent through it, the one upload, and the fallback to this phone's own.
 * AgentKit's `BoardReadsKeeperTests`, case for case.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class BoardReadsKeeperTest {
    private val task = "task-a"
    private val host = "runner-1"
    private val ws = "ws-1"

    /** A moment in 2027, past any "now" a test hands in as the past, so a device clock can be told from the runner's. */
    private val moved = 1_800_000_000_000L

    private fun row(id: String = task, movedMs: Long = moved) = TaskRow(
        id = id, key = "k-$id", title = id, status = TaskStatus.DONE, statusSince = movedMs, updatedAt = movedMs,
    )

    /** A runner, as far as these tests ask: keeps state by max, skips the tasks it's told to, and records every send. */
    private class Runner {
        var floor = 0L
        val marks = mutableMapOf<String, Long>()
        val sent = mutableListOf<ReadsRaise>()
        var fails = false
        var skips = setOf<String>()
        var holds: CompletableDeferred<Unit>? = null

        suspend fun answer(raise: ReadsRaise): String? {
            sent += raise
            holds?.await()
            if (fails) return null
            raise.floorMs?.let { floor = maxOf(floor, it) }
            for ((id, at) in raise.opened) if (id !in skips) marks[id] = maxOf(marks[id] ?: 0L, at)
            return state()
        }

        fun state(): String {
            val kept = marks.filter { it.value > floor }.map { """{"task_id":"${it.key}","opened_ms":${it.value}}""" }
            return """{"workspace_id":"ws-1","floor_ms":$floor,"opened":[${kept.joinToString(",")}]}"""
        }

        fun board(): String = """{"tasks":[],"reads":${state()}}"""
    }

    private fun TestScope.keeper(runner: Runner, store: BoardReadsStore, nowMs: Long = System.currentTimeMillis()) =
        BoardReadsKeeper(store, host, ws, nowMs, backgroundScope) { runner.answer(it) }

    @Test
    fun theRunnersStateDecidesWhatIsRead() = runTest {
        val runner = Runner().apply { floor = moved + 1000 }
        val k = keeper(runner, InMemoryBoardReads())
        assertFalse(k.runnerKeepsReads)
        k.adopt(runner.board())
        assertTrue(k.runnerKeepsReads)
        assertFalse("the runner says it's read", k.reads.value.finishedUnread(row()))
        assertEquals(moved + 1000, k.reads.value.floorMs)
    }

    @Test
    fun aMarkIsSentWithTheRunnersTimeAndKeptNowhereElse() = runTest {
        val runner = Runner()
        val store = InMemoryBoardReads()
        val k = keeper(runner, store)
        k.adopt(runner.board())
        k.flush()
        k.open(row(), nowMs = 4_100_000_000_000L)
        assertFalse("read at once, before the runner answers", k.reads.value.finishedUnread(row()))
        k.flush()
        assertEquals("the row's own time, not this phone's clock", moved, runner.sent.last().opened[task])
        assertEquals(moved, runner.marks[task])
        assertTrue("settled once it answered", store.loadPending(host, ws).isEmpty)
        assertNull("this phone keeps nothing", store.load(host, ws, 0L).opened[task])
    }

    @Test
    fun markAllAsReadSendsAFloorThroughWhatWasShown() = runTest {
        val runner = Runner()
        val k = keeper(runner, InMemoryBoardReads())
        k.adopt(runner.board())
        k.markAllRead(listOf(row(), row("b", moved + 500)), latestMs = moved + 900)
        k.flush()
        assertEquals(moved + 900, runner.floor)
        assertEquals(moved + 900, k.reads.value.floorMs)
    }

    @Test
    fun aStateHeardLateOrLowerNeverLowersAMark() = runTest {
        val runner = Runner().apply { floor = moved + 5000 }
        val k = keeper(runner, InMemoryBoardReads())
        k.adopt(runner.board())
        k.heard(WireBoardReads(ws, moved, emptyMap()))
        assertEquals(moved + 5000, k.reads.value.floorMs)
        k.heard(WireBoardReads(ws, moved + 9000, emptyMap()))
        assertEquals("a higher one rises", moved + 9000, k.reads.value.floorMs)
    }

    @Test
    fun aMarkNotYetAnsweredSurvivesAStateHeardWithoutIt() = runTest {
        val runner = Runner().apply { fails = true }
        val k = keeper(runner, InMemoryBoardReads())
        k.adopt(runner.board())
        k.open(row())
        k.heard(WireBoardReads(ws, 1, emptyMap()))
        assertFalse("a state without the mark doesn't undo it", k.reads.value.finishedUnread(row()))
    }

    @Test
    fun openingNeverLowersAMarkHeardFromAnotherDevice() = runTest {
        val runner = Runner().apply { marks[task] = moved + 4000 }
        val k = keeper(runner, InMemoryBoardReads())
        k.adopt(runner.board())
        k.open(row())
        assertEquals(moved + 4000, k.reads.value.opened[task])
    }

    @Test
    fun anUnsentMarkSurvivesARelaunchAndGoesOnTheNextRead() = runTest {
        val runner = Runner().apply { fails = true }
        val store = InMemoryBoardReads()
        val first = keeper(runner, store)
        first.adopt(runner.board())
        first.open(row())
        first.flush()
        assertFalse("no answer keeps it owed", store.loadPending(host, ws).isEmpty)

        runner.fails = false
        val second = keeper(runner, store)
        assertFalse("counted read before the runner is heard from", second.reads.value.finishedUnread(row()))
        second.adopt(runner.board())
        second.flush()
        assertEquals(moved, runner.marks[task])
        assertTrue(store.loadPending(host, ws).isEmpty)
    }

    @Test
    fun anyAnswerSettlesAMarkEvenOneTheRunnerSkipped() = runTest {
        val runner = Runner().apply { skips = setOf(task) }
        val store = InMemoryBoardReads()
        val k = keeper(runner, store)
        k.adopt(runner.board())
        k.flush()
        k.open(row())
        k.flush()
        val sends = runner.sent.size
        k.adopt(runner.board())
        k.flush()
        assertEquals("a skipped mark isn't sent again on every board read", sends, runner.sent.size)
        assertTrue(store.loadPending(host, ws).isEmpty)
    }

    @Test
    fun aMarkMadeWhileASendIsOutGoesAfterIt() = runTest {
        val runner = Runner()
        val k = keeper(runner, InMemoryBoardReads())
        k.adopt(runner.board())
        k.flush()
        val gate = CompletableDeferred<Unit>()
        runner.holds = gate
        k.open(row())
        runCurrent()
        k.adopt(runner.board())  // a board read, while the send is out: it doesn't wait
        k.open(row("b"))
        runCurrent()
        assertEquals("one send at a time, the rest wait their turn", 1, runner.sent.size)
        gate.complete(Unit)
        k.flush()
        assertEquals(moved, runner.marks[task])
        assertEquals("the second mark isn't lost behind the first", moved, runner.marks["b"])
    }

    @Test
    fun thePhonesOwnMarksGoUpOnceAndNeverItsFloor() = runTest {
        val runner = Runner()
        val store = InMemoryBoardReads()
        // State this phone kept from before the runner kept any: a floor it set itself, and one opened ticket.
        store.save(BoardReads(moved - 90_000, mapOf(task to moved + 100)), host, ws)
        val k = keeper(runner, store)
        k.adopt(runner.board())
        k.flush()
        assertEquals("a phone's floor was never seen as Unread by anyone", 0L, runner.floor)
        assertEquals(moved + 100, runner.marks[task])
        assertTrue(store.isUploaded(host, ws))

        val sends = runner.sent.size
        val again = keeper(runner, store)
        again.adopt(runner.board())
        again.flush()
        assertEquals("once per runner", sends, runner.sent.size)
    }

    @Test
    fun aFirstLookThePhoneMadeUpIsNeverUploaded() = runTest {
        val runner = Runner()
        val store = InMemoryBoardReads()
        // Launch one makes up a first look and never hears from a runner that keeps state.
        keeper(runner, store)
        // Launch two does.
        val k = keeper(runner, store)
        k.adopt(runner.board())
        k.flush()
        assertTrue("nothing real was kept, so nothing goes up", runner.sent.isEmpty())
        assertEquals(0L, runner.floor)
    }

    @Test
    fun aFirstLookThePhoneMadeUpDoesNotHideWhatTheRunnerShows() = runTest {
        val runner = Runner()
        val now = moved + 90_000_000
        val k = keeper(runner, InMemoryBoardReads(), now)
        assertEquals("alone, the last day counts unread", now - 86_400_000, k.reads.value.floorMs)
        k.adopt(runner.board())  // the runner's floor is 0: nothing is read
        assertTrue("the runner says it's unread", k.reads.value.finishedUnread(row()))
    }

    @Test
    fun aFloorThePhoneKeptStillStandsOnARunnerThatKeepsState() = runTest {
        val runner = Runner()
        val store = InMemoryBoardReads()
        store.save(BoardReads(moved + 1000), host, ws)
        store.save(BoardReads(moved + 2000), host, ws)  // moved: somebody's doing
        val k = keeper(runner, store)
        k.adopt(runner.board())
        assertEquals(moved + 2000, k.reads.value.floorMs)
    }

    @Test
    fun anOlderRunnerKeepsThePhonesOwnStateOnTheDeviceClock() = runTest {
        val runner = Runner()
        val store = InMemoryBoardReads()
        store.save(BoardReads(1000), host, ws)
        val k = keeper(runner, store)
        k.adopt("""{"tasks":[]}""")
        assertFalse(k.runnerKeepsReads)
        val now = 1_900_000_000_000L
        k.open(row(), nowMs = now)
        k.flush()
        assertTrue(runner.sent.isEmpty())
        assertEquals(now, store.load(host, ws, now).opened[task])
    }

    @Test
    fun aRunnerThatStopsKeepingStateIsKeptOnThePhoneAgain() = runTest {
        val runner = Runner()
        val store = InMemoryBoardReads()
        val k = keeper(runner, store)
        k.adopt(runner.board())
        k.open(row())
        k.adopt("""{"tasks":[]}""")
        assertFalse(k.runnerKeepsReads)
        assertEquals(moved, store.load(host, ws, 0L).opened[task])
    }

    @Test
    fun theArgumentsAreTheCoresAndAMalformedStateIsNothing() {
        val raise = ReadsRaise(5000, mapOf("b" to 7000L, "a" to 6000L))
        assertEquals(
            """{"workspace":"w","floor_ms":5000,"opened":[{"task_id":"a","opened_ms":6000},{"task_id":"b","opened_ms":7000}]}""",
            raise.arguments("w").toString(),
        )
        assertFalse(ReadsRaise(opened = mapOf("a" to 1L)).arguments("w").containsKey("floor_ms"))
        assertNull(WireBoardReads.ofBoard("""{"tasks":[]}"""))
        assertNull(WireBoardReads.ofBoard("""{"tasks":[],"reads":{"floor_ms":"x"}}"""))
        val wire = WireBoardReads.ofBoard("""{"tasks":[],"reads":{"workspace_id":"w","floor_ms":1000,"opened":[{"task_id":"a","opened_ms":5000},{"task_id":"b","opened_ms":500}]}}""")!!
        assertEquals("a mark under the floor says nothing", mapOf("a" to 5000L), wire.reads.opened)
    }
}
