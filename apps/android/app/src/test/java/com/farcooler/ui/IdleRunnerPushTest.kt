package com.farcooler.ui

import com.farcooler.data.RunnerIds
import com.farcooler.model.Destination
import com.farcooler.model.DestinationPayloads
import com.farcooler.model.DestinationResolver
import com.farcooler.model.DestinationResolver.Arrival
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** A push naming a paired runner the phone isn't connected to (ov-231). */
class IdleRunnerPushTest {
    private val push = mapOf("terminal" to "t-x", "runner" to "RUNNER-B")

    private fun connected() = mapOf(
        "h1" to DestinationWorld.Source(hostId = "h1", runnerId = "runner-a", ready = true, idle = false, fleet = null),
    )

    private fun sources(known: Map<String, String>, connected: Map<String, DestinationWorld.Source> = connected()) =
        DestinationWorld.sources(
            paired = listOf("h1", "h2"), connected = connected, known = known, everyRunner = false, selected = "h1",
        )

    private val stayed = DestinationDriver.Step.Stay(DestinationResolver.Note.RUNNER_UNAVAILABLE, Arrival.NOTIFICATION)

    private fun step(sources: List<DestinationWorld.Source>): DestinationDriver.Step {
        val driver = DestinationDriver { 0L }
        driver.request((DestinationPayloads.from(push::get) ?: error("no destination")), Arrival.NOTIFICATION)
        return driver.step(DestinationWorld.world(sources), moved = false)
    }

    @Test
    fun aPushNamingAnIdleKnownRunnerConnectsIt() {
        val sources = sources(known = mapOf("h2" to "runner-b"))
        val idle = sources.first { it.hostId == "h2" }
        assertEquals("runner-b", idle.runnerId)
        assertTrue(idle.idle)
        assertEquals(DestinationDriver.Step.Connect("h2"), step(sources))
    }

    @Test
    fun aPushNamingARunnerNeverConnectedDialsNothing() {
        val sources = sources(known = emptyMap())
        assertEquals(stayed, step(sources))
        assertTrue(sources.first { it.hostId == "h2" }.idle)
    }

    @Test
    fun aLiveIdBeatsARememberedOne() {
        val live = connected() + ("h2" to DestinationWorld.Source("h2", "runner-b", ready = true, idle = false, fleet = null))
        assertEquals("runner-b", sources(known = mapOf("h2" to "stale"), connected = live).first { it.hostId == "h2" }.runnerId)
    }

    @Test
    fun aConnectingRunnerMatchesByItsRememberedIdAndIsWaitedFor() {
        val dialing = connected() + ("h2" to DestinationWorld.Source("h2", null, ready = false, idle = false, fleet = null))
        val sources = sources(known = mapOf("h2" to "runner-b"), connected = dialing)
        assertEquals("runner-b", sources.first { it.hostId == "h2" }.runnerId)
        assertEquals(DestinationDriver.Step.Wait, step(sources))
    }

    @Test
    fun anEditThatRepointsARunnerOrChangesItsUserForgetsItsId() {
        assertTrue(RunnerIds.survivesEdit(reachChanged = false, userChanged = false))
        assertTrue(!RunnerIds.survivesEdit(reachChanged = true, userChanged = false))
        assertTrue(!RunnerIds.survivesEdit(reachChanged = false, userChanged = true))
        // Forgotten, a push for the old runner no longer finds the edited one.
        assertEquals(stayed, step(sources(known = emptyMap())))
    }

    @Test
    fun idsAreRememberedOverwrittenAndForgotten() {
        var ids = RunnerIds.decode(null)
        assertTrue(ids.isEmpty())
        ids = RunnerIds.remember(ids, "h", "one")
        assertEquals(mapOf("h" to "one"), RunnerIds.remember(ids, "h", null))
        assertEquals(mapOf("h" to "one"), RunnerIds.remember(ids, "h", ""))
        ids = RunnerIds.decode(RunnerIds.encode(RunnerIds.remember(ids, "h", "two")))
        assertEquals(mapOf("h" to "two"), ids)
        assertTrue(RunnerIds.forget(ids, "h").isEmpty())
        assertTrue(RunnerIds.decode("not json").isEmpty())
    }
}
