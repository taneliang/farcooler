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
    private fun train(state: String, extra: String = "", lanes: String = "[]") =
        """{"id": "t", "short": "t", "name": "integ-9", "base": "main", "pushed_sha": null, "state": "$state", "state_since": 5,
           $extra "actor": "m", "created_at": 1, "landed_at": null, "lanes": $lanes, "ci_subject": ""}"""

    private fun plan(trains: String, lanes: String = "[]") =
        Plan.decode("""{"now_ms": 1, "themes": [], "lanes": $lanes, "order": [], "cards": [], "trains": [$trains]}""")

    private fun lane(name: String, state: String) =
        """{"id": "l-$name", "name": "$name", "state": "$state", "reason": "", "worktree_path": "", "branch": "", "harness": "", "model": "",
           "state_since": 1, "stale": false, "fix_rounds": 0, "cards": [], "agents": [], "spend": {}}"""

    @Test
    fun `an answer with no phase or agents reads, and a train falls back to its state`() {
        val plan = plan(train("pushed"))
        val train = plan.trains.single()
        assertNull(train.phase)
        assertNull(train.phaseSince)
        assertEquals(TrainPhase.WAITING_FOR_CI, train.phaseIn(plan))
        assertEquals(5L, train.phaseSinceIn())
        assertTrue(plan.agents.isEmpty())
        assertEquals(listOf(TrainPhase.WAITING_FOR_CI), PlanSections.of(plan).trains.map { it.phase })
    }

    @Test
    fun `a runner before Train 119 still sends held_by, and no line is drawn from it`() {
        val plan = plan(train("integrating", """"phase": "waiting_for_lanes", "phase_since": 9, "held_by": [{"lane": "l-slow", "name": "slow"}],"""))
        val card = PlanSections.of(plan).trains.single()
        assertEquals(TrainPhase.WAITING_FOR_LANES, card.phase)
        assertEquals(TrainPhase.WAITING_FOR_LANES.word, card.phase.word)
        assertTrue("nothing is said of a hold", listOf(card.train.label, card.train.subtitle, card.phase.word, card.phase.explanation).none { it.orEmpty().contains("slow") })
    }

    @Test
    fun `a phase this build has no word for reads the state, and the runner's phase wins when it has one`() {
        fun withPhase(phase: String) = plan(train("red", """"phase": $phase, "phase_since": 9,"""))
        val unknown = withPhase("\"from_the_future\"")
        assertEquals(TrainPhase.UNKNOWN, unknown.trains[0].phase)
        assertEquals(TrainPhase.CI_FAILED, unknown.trains[0].phaseIn(unknown))
        assertEquals(5L, unknown.trains[0].phaseSinceIn())
        val stalled = withPhase("\"stalled\"")
        assertEquals(TrainPhase.STALLED, stalled.trains[0].phaseIn(stalled))
        assertEquals(9L, stalled.trains[0].phaseSinceIn())
        val none = withPhase("null")
        assertNull(none.trains[0].phase)
        assertEquals(TrainPhase.CI_FAILED, none.trains[0].phaseIn(none))
    }

    @Test
    fun `an integrating train waits for lanes while one isn't ready, and integrates once all are`() {
        fun withLane(state: String) = plan(
            train("integrating", lanes = """[{"lane": "l-a", "name": "a"}, {"lane": "l-b", "name": "b"}]"""),
            "[${lane("a", "landing")}, ${lane("b", state)}]",
        )
        val waiting = withLane("fixing")
        assertEquals(TrainPhase.WAITING_FOR_LANES, waiting.trains[0].phaseIn(waiting))
        val ready = withLane("landed")
        assertEquals(TrainPhase.INTEGRATING, ready.trains[0].phaseIn(ready))
    }

    @Test
    fun `the strip carries the band, and its old parts are unchanged`() {
        val plan = Plan.decode(
            """{"now_ms": 1, "themes": [], "order": ["l-p", "l-v"], "cards": [], "lanes": [
              ${lane("mac-vis", "building")}, ${lane("p", "queued").replace("\"p\"", "\"plan-phones\"")},
              ${lane("v", "queued").replace("\"v\"", "\"vis-lint\"")}]}""",
        )
        val strip = PlanStrip.of(plan, needsYou = 0, orchestrator = null)
        assertEquals("1 building", strip.band?.now)
        assertEquals("plan-phones, then vis-lint", strip.band?.next)
        assertEquals("mac-vis Building · next: plan-phones", strip.text)
    }

    @Test
    fun `the runner's own bytes carry the phase on a red train`() {
        val plan = Plan.decode(repositoryFile("test/fixtures/plan.json"))
        val train = plan.trains.single()
        assertEquals(TrainPhase.CI_FAILED, train.phase)
        assertEquals(TrainPhase.CI_FAILED, train.phaseIn(plan))
        assertTrue(train.phaseSince != null)
        assertEquals(TrainPhase.CI_FAILED, PlanSections.of(plan).trains.first().phase)
    }

    @Test
    fun `the runner's own bytes carry the agents that exist, the orchestrator first, a subagent nobody recorded unjoined`() {
        val plan = Plan.decode(repositoryFile("test/fixtures/plan.json"))
        assertEquals(listOf(PlanFleetRole.ORCHESTRATOR, PlanFleetRole.REVIEW, PlanFleetRole.SUBAGENT), plan.agents.map { it.role })
        val fleet = PlanSections.of(plan).fleet
        assertEquals(listOf("Orchestrator · Opus", "Reviewer · Sonnet", "Subagent · Sonnet"), fleet.agents.map { it.who })
        assertEquals(listOf("Plan", "mac-ux", PlanWords.NOT_ON_A_LANE), fleet.agents.map { it.worksFor })
        assertEquals("Design ov-503", fleet.agents[2].label)
        assertTrue(fleet.agents.all { it.working })
        assertEquals(3, PlanSections.of(plan).count(PlanSections.Kind.AGENTS))
        assertEquals(2_390_000L, fleet.agents.sumOf { it.spend.totalTokens })
        // The total is the runner's: the day's spend of every agent, those no longer listed too.
        assertEquals("the header counts agents that are gone", 3_000_000L, fleet.totalTokens)
        assertEquals(20_000_000L, fleet.totalCostMicros)
        assertEquals("mac-ux", fleet.agents.first { it.role == PlanFleetRole.REVIEW }.laneName)
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

    private fun queued(name: String, rank: Int, task: String, title: String = "") =
        """{"id": "l-$name", "name": "$name", "title": "$title", "state": "queued", "reason": "", "plan_rank": $rank, "worktree_path": "",
           "branch": "", "harness": "", "model": "", "state_since": 1, "stale": false, "fix_rounds": 0,
           "cards": [{"task": "$task", "key": "$task", "slice": ""}], "agents": [], "spend": {}}"""

    private fun ranked(vararg lanes: String) =
        Plan.decode("""{"now_ms": 1, "themes": [], "lanes": [${lanes.joinToString(",")}], "order": ["l-a", "l-b"], "cards": [], "trains": []}""")

    @Test
    fun `a lane waiting on a decision is never next, in the strip or its band`() {
        val plan = ranked(queued("a", 1, "t1"), queued("b", 2, "t2"))
        val blind = PlanStrip.of(plan, needsYou = 0, orchestrator = null)
        assertEquals("a, then b", blind.band?.next)
        val strip = PlanStrip.of(plan, needsYou = 0, orchestrator = null, statuses = mapOf("t1" to TaskStatus.NEEDS_DECISION))
        assertEquals("b", strip.next)
        assertEquals("b", strip.band?.next)
    }

    @Test
    fun `the band's next names titles only, once each, cut at a word`() {
        val plan = ranked(
            queued("a", 1, "t1", "Reps: each person's asks reach the orchestrator, then skill-init writes the manager skill"),
            queued("b", 2, "t2", "Onboarding: skill-init explains what the manager skill is for, in the owner's words"),
        )
        val next = PlanStrip.of(plan, needsYou = 0, orchestrator = null).band?.next
        assertEquals("Reps: each person's asks reach the orchestrator, then skill-init\u2026, then Onboarding: skill-init explains what the manager skill is for, in the\u2026", next)
    }

    @Test
    fun `the Now line calls a train by its number and phase, never its title (ov-514)`() {
        val title = "Train 89 \u00b7 The agents table and the themes grid on the phones"
        fun at(name: String, title: String) =
            train("pushed", """"title": "$title", "phase": "waiting_for_ci", "phase_since": 5,""")
                .replace("integ-9", name)
        val plan = plan(at("integ-89", title) + "," + at("integ-90", "Another long title"))
        assertEquals("the runner's \"Train 89 \u00b7\" is not said twice", "The agents table and the themes grid on the phones", plan.trains[0].subtitle)
        assertEquals("Train 89", plan.trains[0].label)
        val now = PlanGlanceSentence.of(PlanSections.of(plan)).now!!
        assertEquals("Train 89 checking on CI \u00b7 1 more landing", now)
        assertTrue(now, !now.contains("agents table"))
        // Short enough for 320 dp and for a watch's line.
        assertTrue(now, now.length <= 40)
    }
}
