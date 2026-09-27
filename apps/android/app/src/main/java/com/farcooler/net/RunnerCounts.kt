package com.farcooler.net

import com.farcooler.model.Fleet
import com.farcooler.model.RunnerCount
import com.farcooler.model.RunnerLink
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.flowOf

/** One runner's link and last fleet, as the drawer's footer counts them. See [Connection.link]. */
fun runnerCount(link: RunnerLink, fleet: Fleet): RunnerCount =
    RunnerCount(link, fleet.livePanes, fleet.runtimeHealthy)

/**
 * Each runner's link and last count, moving whenever any runner's phase or
 * fleet moves.
 *
 * **A flow over each runner's own flows, not a read of their values.** The
 * footer read `phase.value` while composing, which Compose does not observe:
 * the screen recomposes on `entries` and `active`, and a runner dropping its
 * link changes neither (the repository republishes lists equal to the last,
 * which a `StateFlow` swallows). So the footer went on saying "3 live" in the
 * calm color through the whole reconnect, beside rows that had already gone
 * dashed. Collected, it moves the moment the phase does.
 */
fun runnerCounts(
    runners: List<Pair<StateFlow<RunnerLink>, StateFlow<Fleet>>>,
): Flow<List<RunnerCount>> {
    if (runners.isEmpty()) return flowOf(emptyList())
    return combine(runners.map { (link, fleet) -> combine(link, fleet, ::runnerCount) }) {
        it.toList()
    }
}
