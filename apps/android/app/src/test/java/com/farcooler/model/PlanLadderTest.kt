package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * A landing's ladder (ov-503, ov-522): which of three steps each phase stands on, held to the Mac's `PlanLadder` (the same table
 * as `PlanLandingPhase.swift`), what the phone says compactly, and the facts line a card carries.
 */
class PlanLadderTest {
    @Test
    fun `every phase stands on the step the Mac's ladder puts it on`() {
        val expected = mapOf(
            LandingPhase.WAITING_FOR_TICKETS to 0, LandingPhase.WAITING_FOR_PREVIOUS_LANDING to 0, LandingPhase.WAITING_FOR_MAIN to 0, LandingPhase.UNKNOWN to 0,
            LandingPhase.INTEGRATING to 0, LandingPhase.STALLED to 0,
            LandingPhase.GATING_LOCALLY to 0,
            LandingPhase.WAITING_FOR_CI to 1, LandingPhase.CI_FAILED to 1,
            LandingPhase.READY_TO_LAND to 2,
            LandingPhase.LANDED to 3, LandingPhase.DROPPED to 3,
        )
        assertEquals(LandingPhase.entries.toSet(), expected.keys)
        for ((phase, step) in expected) assertEquals(phase.name, step, PlanLadder.step(phase))
        assertEquals(listOf("Checking here", "Checking on CI", "Landed"), PlanLadder.steps)
    }

    @Test
    fun `the phone says the phase and its step, and only the phase once the ladder is climbed`() {
        assertEquals("Checking on CI · step 2 of 3", PlanLadder.compact(LandingPhase.WAITING_FOR_CI))
        assertEquals("CI failed · step 2 of 3", PlanLadder.compact(LandingPhase.CI_FAILED))
        assertEquals("Ready to land · step 3 of 3", PlanLadder.compact(LandingPhase.READY_TO_LAND))
        assertEquals("Waiting for tickets · step 1 of 3", PlanLadder.compact(LandingPhase.WAITING_FOR_TICKETS))
        assertEquals("Waiting for main's checks to pass · step 1 of 3", PlanLadder.compact(LandingPhase.WAITING_FOR_MAIN))
        assertEquals("Landed", PlanLadder.compact(LandingPhase.LANDED))
    }

    private fun card(sha: String?, lander: PlanAgent? = null, since: Long = 0) = PlanSections.LandingCard(
        PlanLanding("t", 9, "main", sha, LandingState.PUSHED, 0, tickets = emptyList(), ciSubject = "sha:$sha"),
        LandingPhase.WAITING_FOR_CI, since, emptyList(), lander,
    )

    private fun lander(spend: PlanSpend, working: Boolean = true) = PlanAgent(
        id = "i1", role = AgentRole.LANDER, harness = "claude", model = "claude-haiku-4", title = "", terminal = null, subagentId = "i1",
        tickets = emptyList(), working = working, doingNow = "", stopAskedAt = null, startedAt = 0, lastAt = 0, spend = spend,
    )

    @Test
    fun `the facts line reads the commit, CI, how long and who lands, in that order`() {
        val read = PlanCiRead("sha:abc", status = CiStatus.RUNNING, jobs = listOf(PlanCiJob("a", "passed"), PlanCiJob("b", "running")), fetchedAt = 1_000)
        val agent = lander(PlanSpend(inputTokens = 1_400_000))
        assertEquals(
            "3f78754f · Running · 1 of 2 jobs done · 12 min · Lander · Haiku · 1.4M",
            LandingWords.facts(card("3f78754f0c2e", agent, since = 1_000), read, now = 1_000 + 12 * 60_000L),
        )
        assertEquals("3f78754f · CI not read yet", LandingWords.facts(card("3f78754f0c2e"), null, now = 0))
        assertEquals("", LandingWords.facts(card(null), null, now = 0))
        assertEquals("Lander · Haiku · done", LandingWords.landerFacts(lander(PlanSpend(), working = false)))
    }
}
