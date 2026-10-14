package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * A train's ladder (ov-503, ov-522): which of three steps each phase stands on, held to the Mac's `PlanLadder` (the same table
 * as `PlanTrainPhase.swift`), what the phone says compactly, and the facts line a card carries.
 */
class PlanLadderTest {
    @Test
    fun `every phase stands on the step the Mac's ladder puts it on`() {
        val expected = mapOf(
            TrainPhase.WAITING_FOR_LANES to 0, TrainPhase.WAITING_FOR_PREVIOUS_TRAIN to 0, TrainPhase.UNKNOWN to 0,
            TrainPhase.INTEGRATING to 0, TrainPhase.STALLED to 0,
            TrainPhase.GATING_LOCALLY to 0,
            TrainPhase.WAITING_FOR_CI to 1, TrainPhase.CI_FAILED to 1,
            TrainPhase.READY_TO_LAND to 2,
            TrainPhase.LANDED to 3, TrainPhase.DROPPED to 3,
        )
        assertEquals(TrainPhase.entries.toSet(), expected.keys)
        for ((phase, step) in expected) assertEquals(phase.name, step, PlanLadder.step(phase))
        assertEquals(listOf("Checking here", "Checking on CI", "Landed"), PlanLadder.steps)
    }

    @Test
    fun `the phone says the phase and its step, and only the phase once the ladder is climbed`() {
        assertEquals("Checking on CI · step 2 of 3", PlanLadder.compact(TrainPhase.WAITING_FOR_CI))
        assertEquals("CI failed · step 2 of 3", PlanLadder.compact(TrainPhase.CI_FAILED))
        assertEquals("Ready to land · step 3 of 3", PlanLadder.compact(TrainPhase.READY_TO_LAND))
        assertEquals("Waiting for lanes · step 1 of 3", PlanLadder.compact(TrainPhase.WAITING_FOR_LANES))
        assertEquals("Landed", PlanLadder.compact(TrainPhase.LANDED))
    }

    private fun card(sha: String?, agent: PlanTrainAgent? = null, since: Long = 0) = PlanSections.TrainCard(
        PlanTrain("t", "integ-9", "main", sha, TrainState.PUSHED, 0, emptyList(), "sha:$sha", agent = agent),
        TrainPhase.WAITING_FOR_CI, since, emptyList(), null,
    )

    @Test
    fun `the facts line reads the commit, CI, how long and who integrates, in that order`() {
        val read = PlanCiRead("sha:abc", status = CiStatus.RUNNING, jobs = listOf(PlanCiJob("a", "passed"), PlanCiJob("b", "running")), fetchedAt = 1_000)
        val agent = PlanTrainAgent("claude", "i1", "claude-opus-4", 0, null, PlanSpend(inputTokens = 1_400_000))
        assertEquals(
            "3f78754f · Running · 1 of 2 jobs done · 12 min · Integrator · Opus · 1.4M",
            TrainWords.facts(card("3f78754f0c2e", agent, since = 1_000), read, now = 1_000 + 12 * 60_000L),
        )
        assertEquals("3f78754f · CI not read yet", TrainWords.facts(card("3f78754f0c2e"), null, now = 0))
        assertEquals("", TrainWords.facts(card(null), null, now = 0))
        assertEquals("Integrator · Opus · done", TrainWords.integrator(PlanTrainAgent("claude", "i1", "claude-opus-4", 0, 5, PlanSpend())))
    }
}
