package com.farcooler.net

import com.farcooler.model.Fleet
import com.farcooler.model.RunnerLink
import com.farcooler.model.liveSummary
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The drawer's footer follows a runner's link as it moves.
 *
 * It read each runner's phase once, while composing, and nothing recomposed
 * it when a runner dropped: the repository republishes lists equal to the last,
 * and a `StateFlow` swallows those. So "3 live · 1 runner" stood beside rows
 * that had already gone dashed, for the whole reconnect.
 */
class RunnerCountsTest {
    /**
     * Mutation: `runnerCounts` returning `flowOf` of each runner's `.value`
     * read once, which is the old footer. Red: still 3 live.
     */
    @Test
    fun `the footer hears a runner drop`() = runBlocking {
        val phase = MutableStateFlow<Connection.Phase>(Connection.Phase.Connected)
        val link = MutableStateFlow(RunnerLink.ANSWERING)
        val fleet = MutableStateFlow(Fleet(runtimeHealthy = true, livePanes = 3))
        val counts = runnerCounts(listOf(Triple(phase, link, fleet)))

        assertEquals("3 live · 1 runner", liveSummary(counts.first()))

        phase.value = Connection.Phase.Reconnecting(attempt = 1)
        link.value = phase.value.link
        val dropped = counts.first()
        assertEquals(RunnerLink.AWAY, dropped.single().link)
        assertEquals("Not connected", liveSummary(dropped))

        phase.value = Connection.Phase.Connected
        link.value = RunnerLink.ANSWERING
        fleet.value = Fleet(runtimeHealthy = true, livePanes = 5)
        assertEquals("5 live · 1 runner", liveSummary(counts.first()))
    }

    @Test
    fun `no runners is no counts`() = runBlocking {
        assertEquals("No runners", liveSummary(runnerCounts(emptyList()).first()))
    }
}
