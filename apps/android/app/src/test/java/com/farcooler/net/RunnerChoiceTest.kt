package com.farcooler.net

import com.farcooler.data.Reach
import com.farcooler.data.Runner
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Which runners the app talks to, and which Settings offers to switch
 * between, with "Connect every runner at once" on and off.
 *
 * Off, the app talks only to the runner picked, and nothing let anyone pick:
 * the one caller of `RunnerStore.select` was `add`, so the runner added last
 * was the only one this app could ever reach again (ov-27).
 */
class RunnerChoiceTest {
    private fun runner(id: String) = Runner(id = id, label = id, reach = Reach.Direct("$id.local", 22), user = "me")

    private val studio = runner("studio")
    private val laptop = runner("laptop")

    @Test
    fun `every runner at once talks to every runner and offers no switch`() {
        assertEquals(listOf(studio, laptop), wantedRunners(listOf(studio, laptop), "laptop", everything = true))
        assertEquals(emptyList<Runner>(), switchableRunners(listOf(studio, laptop), everything = true))
    }

    /**
     * **Off, Settings lists every runner to pick from.** Mutation: the list
     * left empty when off, which is the app before this. Red.
     */
    @Test
    fun `one at a time talks to the one picked and offers the rest`() {
        assertEquals(listOf(laptop), wantedRunners(listOf(studio, laptop), "laptop", everything = false))
        assertEquals(listOf(studio, laptop), switchableRunners(listOf(studio, laptop), everything = false))
    }

    @Test
    fun `one runner is nothing to switch between`() {
        assertEquals(emptyList<Runner>(), switchableRunners(listOf(studio), everything = false))
    }
}
