package com.farcooler.ui

import androidx.compose.runtime.AbstractApplier
import androidx.compose.runtime.BroadcastFrameClock
import androidx.compose.runtime.Composition
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Recomposer
import androidx.compose.runtime.snapshots.Snapshot
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test

/**
 * `rememberMinuteClock` in a real composition, under a lifecycle this test
 * drives. `MinuteClockTest` covers the clock; this covers the call site that
 * starts it on ON_START and stops it on ON_STOP, which nothing else would
 * notice going missing.
 *
 * Plain JVM, no Robolectric: Robolectric 4.17 runs SDK 36 and 37 only on
 * Java 21, this app's `minSdk` is 37, and the build and CI run Java 17. None
 * of that is needed anyway. `rememberMinuteClock` draws nothing, so a
 * composition with a no-op applier and a hand-cranked frame clock runs it
 * whole; `LifecycleStartEffect` and `lifecycle.coroutineScope` are the real
 * ones, against a real `LifecycleRegistry`. Main is the coroutine test
 * dispatcher, so the clock's `delay` runs in virtual time, and the wall clock
 * handed in is that same virtual time.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class MinuteClockLifecycleTest {
    /** 30 s into a minute, so the first boundary is 30 s away and not 60. */
    private val base = 1_757_170_830_000L

    private val main = StandardTestDispatcher()

    @Before
    fun mainIsTheTestScheduler() = Dispatchers.setMain(main)

    @After
    fun mainIsMainAgain() = Dispatchers.resetMain()

    private class Owner : LifecycleOwner {
        override val lifecycle = LifecycleRegistry.createUnsafe(this)
    }

    private class NoNodes : AbstractApplier<Unit>(Unit) {
        override fun insertTopDown(index: Int, instance: Unit) {}
        override fun insertBottomUp(index: Int, instance: Unit) {}
        override fun remove(index: Int, count: Int) {}
        override fun move(from: Int, to: Int, count: Int) {}
        override fun onClear() {}
    }

    /** One `rememberMinuteClock`, composed, and what it last returned. */
    private inner class Card(private val scope: TestScope) {
        val owner = Owner().also { it.lifecycle.currentState = Lifecycle.State.CREATED }
        val reads = mutableListOf<Long>()
        var shown = 0L
        private val frames = BroadcastFrameClock()
        private val recomposer = Recomposer(scope.backgroundScope.coroutineContext + frames)
        private val composition = Composition(NoNodes(), recomposer)

        fun wallNow() = base + scope.testScheduler.currentTime

        init {
            scope.backgroundScope.launch(frames) { recomposer.runRecomposeAndApplyChanges() }
            scope.runCurrent()
            composition.setContent {
                CompositionLocalProvider(LocalLifecycleOwner provides owner) {
                    shown = rememberMinuteClock { wallNow().also { reads += it } }
                }
            }
            settle()
        }

        /** Hand the clock's writes to the recomposer and draw a frame. */
        fun settle() {
            scope.runCurrent()
            Snapshot.sendApplyNotifications()
            scope.runCurrent()
            frames.sendFrame(scope.testScheduler.currentTime * 1_000_000)
            scope.runCurrent()
        }

        fun moveTo(state: Lifecycle.State) {
            owner.lifecycle.currentState = state
            settle()
        }

        fun pass(ms: Long) {
            scope.advanceTimeBy(ms)
            settle()
        }

        /**
         * DESTROYED as well as disposed. That cancels `lifecycle.coroutineScope`,
         * so a clock left running (a call site that never stops it) cannot
         * outlive its test: `runTest` would run its minute loop through virtual
         * time until the heap gave out, and report that instead of the assert.
         */
        fun dispose() {
            owner.lifecycle.currentState = Lifecycle.State.DESTROYED
            composition.dispose()
            recomposer.cancel()
        }
    }

    private fun composed(body: TestScope.(Card) -> Unit) = runTest(main) {
        val card = Card(this)
        try {
            body(card)
        } finally {
            card.dispose()
        }
    }

    /** Started, it reads the clock at once and again on each minute boundary. */
    @Test
    fun startedTheClockMovesOnEachMinute() = composed { card ->
        card.pass(10_000)
        assertEquals("nothing read before ON_START", base, card.shown)

        card.moveTo(Lifecycle.State.STARTED)
        assertEquals("read on ON_START", base + 10_000, card.shown)

        card.pass(19_999)
        assertEquals("nothing before the boundary", base + 10_000, card.shown)

        card.pass(1)
        assertEquals("the minute turned", base + 30_000, card.shown)

        card.pass(60_000)
        assertEquals("and the next", base + 90_000, card.shown)
    }

    /** Stopped, it reads nothing and the time stands still, however long. */
    @Test
    fun stoppedTheClockStandsStill() = composed { card ->
        card.moveTo(Lifecycle.State.STARTED)
        card.pass(30_000)
        assertEquals(base + 30_000, card.shown)

        card.moveTo(Lifecycle.State.CREATED)
        val readsWhenStopped = card.reads.size
        card.pass(5 * 60 * 60_000L)
        assertEquals("nothing read in the background", readsWhenStopped, card.reads.size)
        assertEquals("nothing shown moves", base + 30_000, card.shown)
    }

    /** Started again, it has the new time at once, not at the next minute. */
    @Test
    fun startedAgainItReadsTheClockAfresh() = composed { card ->
        card.moveTo(Lifecycle.State.STARTED)
        card.moveTo(Lifecycle.State.CREATED)
        card.pass(5 * 60 * 60_000L + 7_000)

        card.moveTo(Lifecycle.State.STARTED)
        assertEquals("read on ON_START", base + 5 * 60 * 60_000L + 7_000, card.shown)

        card.pass(22_999)
        assertEquals(base + 5 * 60 * 60_000L + 7_000, card.shown)
        card.pass(1)
        assertEquals("ticking again", base + 5 * 60 * 60_000L + 30_000, card.shown)
    }
}
