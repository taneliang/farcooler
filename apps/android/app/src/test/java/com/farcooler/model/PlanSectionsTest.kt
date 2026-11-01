package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * What the cases file can't say in one line (ov-503): that an older answer still reads, that the phase falls back to
 * `state`, that the strip carries the band, and what `test/fixtures/plan.json` (the runner's own bytes) makes of the
 * new fields. AgentKit's `PlanSectionsTests` holds the Mac and the iPhone to the same.
 */
class PlanSectionsTest {
    private fun landing(state: String, extra: String = "", tickets: String = "[]", number: Int = 9) =
        """{"id": "t$number", "short": "t", "number": $number, "title": "", "summary": "", "route": "push", "base": "main", "pushed_sha": null,
           "state": "$state", "state_since": 5, $extra "actor": "m", "created_at": 1, "landed_at": null, "tickets": $tickets, "ci_subject": ""}"""

    private fun plan(landings: String, tickets: String = "[]") =
        Plan.decode("""{"now_ms": 1, "themes": [], "lines": $tickets, "cards": [], "landings": [$landings]}""")

    private fun ticket(key: String, stage: String, extra: String = "") =
        """{"task": "t-$key", "key": "$key", "title": "", "status": "In Progress", "stage": "$stage", "reason": "", "worktree_path": "", "branch": "",
           "stage_since": 1, "stale": false, "fix_rounds": 0, "work": [], "spend": {}$extra}"""

    @Test
    fun `an answer with no phase or agents reads, and a landing falls back to its state`() {
        val plan = plan(landing("pushed"))
        val landing = plan.landings.single()
        assertNull(landing.phase)
        assertNull(landing.phaseSince)
        assertEquals(LandingPhase.WAITING_FOR_CI, landing.phaseIn(plan))
        assertEquals(5L, landing.phaseSinceIn())
        assertTrue(plan.agents.isEmpty())
        assertEquals(listOf(LandingPhase.WAITING_FOR_CI), PlanSections.of(plan).landings.map { it.phase })
    }

    @Test
    fun `a phase this build has no word for reads the state, and the runner's phase wins when it has one`() {
        fun withPhase(phase: String) = plan(landing("red", """"phase": $phase, "phase_since": 9,"""))
        val unknown = withPhase("\"from_the_future\"")
        assertEquals(LandingPhase.UNKNOWN, unknown.landings[0].phase)
        assertEquals(LandingPhase.CI_FAILED, unknown.landings[0].phaseIn(unknown))
        assertEquals(5L, unknown.landings[0].phaseSinceIn())
        val stalled = withPhase("\"stalled\"")
        assertEquals(LandingPhase.STALLED, stalled.landings[0].phaseIn(stalled))
        assertEquals(9L, stalled.landings[0].phaseSinceIn())
        val none = withPhase("null")
        assertNull(none.landings[0].phase)
        assertEquals(LandingPhase.CI_FAILED, none.landings[0].phaseIn(none))
    }

    @Test
    fun `an open landing waits for tickets while one isn't ready, and integrates once all are`() {
        fun withTicket(stage: String) = plan(
            landing("open", tickets = """[{"task": "t-a", "key": "a"}, {"task": "t-b", "key": "b"}]"""),
            "[${ticket("a", "landing")}, ${ticket("b", stage)}]",
        )
        val waiting = withTicket("building")
        assertEquals(LandingPhase.WAITING_FOR_TICKETS, waiting.landings[0].phaseIn(waiting))
        val ready = withTicket("landed")
        assertEquals(LandingPhase.INTEGRATING, ready.landings[0].phaseIn(ready))
    }

    @Test
    fun `the strip carries the band, and its old parts are unchanged`() {
        val plan = Plan.decode(
            """{"now_ms": 1, "themes": [], "cards": [], "lines": [
              ${ticket("mac-vis", "building")}, ${ticket("p", "waiting", ""","plan_rank": 1""").replace("\"title\": \"\"", "\"title\": \"plan-phones\"")},
              ${ticket("v", "waiting", ""","plan_rank": 2""").replace("\"title\": \"\"", "\"title\": \"vis-lint\"")}]}""",
        )
        val strip = PlanStrip.of(plan, needsYou = 0, orchestrator = null)
        assertEquals("1 building", strip.band?.now)
        assertEquals("plan-phones, then vis-lint", strip.band?.next)
        assertEquals("mac-vis Building · next: plan-phones", strip.text)
    }

    @Test
    fun `the runner's own bytes carry the phase on a red landing`() {
        val plan = Plan.decode(repositoryFile("test/fixtures/plan.json"))
        val landing = plan.landings.first()
        assertEquals(LandingPhase.CI_FAILED, landing.phase)
        assertEquals(LandingPhase.CI_FAILED, landing.phaseIn(plan))
        assertTrue(landing.phaseSince != null)
        assertEquals(LandingPhase.CI_FAILED, PlanSections.of(plan).landings.first().phase)
    }

    @Test
    fun `the runner's own bytes carry the agents that exist, the orchestrator first, and what each works for`() {
        val plan = Plan.decode(repositoryFile("test/fixtures/plan.json"))
        assertEquals(listOf(AgentRole.ORCHESTRATOR, AgentRole.REVIEWER, AgentRole.BUILDER), plan.agents.map { it.role })
        val fleet = PlanSections.of(plan).fleet
        // Working agents come first, then by spend.
        assertEquals(listOf("Orchestrator · Opus", "Reviewer · Sonnet", "Builder · gpt-5.6"), fleet.agents.map { it.who })
        assertEquals(listOf("Plan", "ov-1", "ov-4"), fleet.agents.map { it.worksFor })
        assertEquals("Review ov-1", fleet.agents[1].title)
        assertEquals(listOf(true, true, false), fleet.agents.map { it.working })
        assertEquals(3, PlanSections.of(plan).count(PlanSections.Kind.AGENTS))
        assertEquals(2_300_000L, fleet.agents.sumOf { it.spend.totalTokens })
        // The total is the runner's: the day's spend of every agent, those no longer listed too.
        assertEquals("the header counts agents that are gone", 3_000_000L, fleet.totalTokens)
        assertEquals(20_000_000L, fleet.totalCostMicros)
        assertEquals("Stopping", fleet.agents[2].state(plan.nowMs))
    }

    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}.")
    }


    private fun queued(key: String, rank: Int, title: String = "") =
        ticket(key, "waiting", ""","plan_rank": $rank""").replace("\"title\": \"\"", "\"title\": \"$title\"")

    private fun ranked(vararg tickets: String) =
        Plan.decode("""{"now_ms": 1, "themes": [], "lines": [${tickets.joinToString(",")}], "cards": [], "landings": []}""")

    @Test
    fun `a ticket waiting on a decision is never next, in the strip or its band`() {
        val plan = ranked(queued("a", 1), queued("b", 2))
        val blind = PlanStrip.of(plan, needsYou = 0, orchestrator = null)
        assertEquals("a, then b", blind.band?.next)
        val strip = PlanStrip.of(plan, needsYou = 0, orchestrator = null, statuses = mapOf("t-a" to TaskStatus.NEEDS_DECISION))
        assertEquals("b", strip.next)
        assertEquals("b", strip.band?.next)
    }

    @Test
    fun `the band's next names titles only, once each, cut at a word`() {
        val plan = ranked(
            queued("a", 1, "Reps: each person's asks reach the orchestrator, then skill-init writes the manager skill"),
            queued("b", 2, "Onboarding: skill-init explains what the manager skill is for, in the owner's words"),
        )
        val next = PlanStrip.of(plan, needsYou = 0, orchestrator = null).band?.next
        assertEquals("Reps: each person's asks reach the orchestrator, then skill-init\u2026, then Onboarding: skill-init explains what the manager skill is for, in the\u2026", next)
    }

    @Test
    fun `the Now line calls a landing by its number and phase, never its title (ov-514)`() {
        val title = "Landing 89 \u00b7 The agents table and the themes grid on the phones"
        fun at(number: Int, title: String) =
            landing("pushed", """"title": "$title", "phase": "waiting_for_ci", "phase_since": 5,""", number = number)
                .replace("\"title\": \"\",", "")
        val plan = plan(at(89, title) + "," + at(90, "Another long title"))
        assertEquals("the runner's \"Landing 89 \u00b7\" is not said twice", "The agents table and the themes grid on the phones", plan.landings[0].subtitle)
        assertEquals("Landing 89", plan.landings[0].label)
        val now = PlanGlanceSentence.of(PlanSections.of(plan)).now!!
        assertEquals("Landing 89 checking on CI \u00b7 1 more landing", now)
        assertTrue(now, !now.contains("agents table"))
        // Short enough for 320 dp and for a watch's line.
        assertTrue(now, now.length <= 44)
    }
}
