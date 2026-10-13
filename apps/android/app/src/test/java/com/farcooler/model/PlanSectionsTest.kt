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
    fun `an answer with no phase, held_by or agents reads, and a train falls back to its state`() {
        val plan = plan(train("pushed"))
        val train = plan.trains.single()
        assertNull(train.phase)
        assertNull(train.phaseSince)
        assertTrue(train.heldBy.isEmpty())
        assertEquals(TrainPhase.WAITING_FOR_CI, train.phaseIn(plan))
        assertEquals(5L, train.phaseSinceIn())
        assertTrue(plan.agents.isEmpty())
        assertEquals(listOf(TrainPhase.WAITING_FOR_CI), PlanSections.of(plan).shipping.map { it.phase })
    }

    @Test
    fun `a phase this build has no word for reads the state, and the runner's phase wins when it has one`() {
        fun withPhase(phase: String) = plan(train("red", """"phase": $phase, "phase_since": 9, "held_by": [],"""))
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
        assertTrue(train.phaseSince != null && train.heldBy.isEmpty())
        assertEquals(TrainPhase.CI_FAILED, PlanSections.of(plan).shipping.first().phase)
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
}
