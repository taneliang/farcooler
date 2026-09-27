package com.farcooler.ui

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The card clock on the coroutine test scheduler: the wall clock here is the
 * scheduler's virtual time, so a test can let hours pass in no time and see
 * exactly which readings the clock took.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class MinuteClockTest {
    /** 30 s into a minute, so the first boundary is 30 s away and not 60. */
    private val base = 1_757_170_830_000L

    private class Readings(private val scope: TestScope, private val base: Long) {
        val reads = mutableListOf<Long>()
        val ticks = mutableListOf<Long>()
        val clock = MinuteClock(
            now = { (base + scope.testScheduler.currentTime).also { reads += it } },
            onTick = { ticks += it },
        )
    }

    /** It reads at once, then on each wall-clock minute, with nothing else. */
    @Test
    fun theClockTicksOnEachMinuteBoundary() = runTest {
        val r = Readings(this, base)
        r.clock.start(backgroundScope)
        runCurrent()
        assertEquals(listOf(base), r.ticks)

        advanceTimeBy(29_999)
        runCurrent()
        assertEquals("nothing before the boundary", listOf(base), r.ticks)

        advanceTimeBy(1)
        runCurrent()
        assertEquals(listOf(base, base + 30_000), r.ticks)

        advanceTimeBy(120_000)
        runCurrent()
        assertEquals(
            listOf(base, base + 30_000, base + 90_000, base + 150_000),
            r.ticks,
        )
        r.clock.stop()
    }

    /**
     * Stopped, it reads nothing, however long the app is away. Started again,
     * it has the new time before `start` returns — before any dispatcher runs,
     * which is what puts it on the first frame back rather than a frame later.
     */
    @Test
    fun startedAgainItReadsTheClockAtOnceAndStoppedItReadsNothing() = runTest {
        val r = Readings(this, base)
        r.clock.start(backgroundScope)
        assertEquals("the first reading is taken inside start", listOf(base), r.ticks)

        r.clock.stop()
        val readsWhenStopped = r.reads.size
        advanceTimeBy(5 * 60 * 60_000L)
        runCurrent()
        assertEquals("nothing ticks in the background", readsWhenStopped, r.reads.size)

        r.clock.start(backgroundScope)
        assertEquals(
            "back after five hours, and the first reading is now",
            base + 5 * 60 * 60_000L,
            r.ticks.last(),
        )
        r.clock.stop()
    }
}
