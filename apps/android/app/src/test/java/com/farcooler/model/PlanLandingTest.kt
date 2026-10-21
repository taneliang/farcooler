package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Landing (ov-522): the trains' section is Landing, never Shipping; a train is "Train N" with its title under it; a lane
 * aboard a train says nothing unless it waits on the owner or has landed; an open train waiting for the train ahead says
 * so; the ladder is three plain steps; and the Next line never says "Nothing queued" while lanes wait. The twin of AgentKit's `PlanLandingTests`, over the same shared cases.
 */
class PlanLandingTest {
    private val cases = Json.parseToJsonElement(repositoryFile("test/fixtures/plan-sections-cases.json")).jsonObject["cases"]!!.jsonArray

    private fun plan(name: String): Plan =
        Plan.decode(cases.first { it.jsonObject["name"]!!.jsonPrimitive.content == name }.jsonObject["plan"]!!.jsonObject)

    private fun sections(name: String): PlanSections {
        val case = cases.first { it.jsonObject["name"]!!.jsonPrimitive.content == name }.jsonObject
        val statuses = case["statuses"]!!.jsonObject.mapNotNull { (k, v) -> TaskStatus.parse(v.jsonPrimitive.content)?.let { k to it } }.toMap()
        return PlanSections.of(plan(name), statuses = statuses)
    }

    @Test
    fun `no section is called Shipping, and the trains' section is Landing, above In review`() {
        assertEquals("Landing", PlanSections.Kind.LANDING.title)
        assertEquals("Nothing is landing right now.", PlanSections.Kind.LANDING.emptyWords)
        assertTrue(PlanSections.Kind.entries.all { !it.title.lowercase().contains("shipping") })
        val kinds = PlanSections.Kind.entries
        assertEquals(PlanSections.Kind.IN_REVIEW, kinds[kinds.indexOf(PlanSections.Kind.LANDING) + 1])
        assertEquals(listOf(PlanSections.Kind.LANDING, PlanSections.Kind.IN_REVIEW), sections("train-lane-notes").shown.take(2))
    }

    @Test
    fun `a lane in landing with no train is In review, and on a train it is in the train`() {
        val s = sections("train-lane-notes")
        assertTrue(s.inReview.map { it.lane.name }.contains("loose"))
        assertFalse(s.trains.flatMap { it.lanes }.map { it.name }.contains("loose"))
        assertTrue(s.trains.flatMap { it.lanes }.map { it.name }.contains("aboard"))
        assertFalse(s.inReview.map { it.lane.name }.contains("aboard"))
    }

    @Test
    fun `an open train waiting for the train ahead says so in place of its first step`() {
        val open = sections("train-lane-notes").trains.first { it.train.name == "integ-6" }
        assertEquals(TrainPhase.WAITING_FOR_PREVIOUS_TRAIN, open.phase)
        assertEquals("Waiting for the earlier landing \u00b7 step 1 of 3", PlanLadder.compact(open.phase))
        assertEquals("Waiting for the earlier landing", PlanLadder.label(0, PlanLadder.step(open.phase), open.phase))
        assertEquals("waiting for the earlier landing", open.phase.sentence)
        assertTrue(open.phase.explanation.endsWith("."))
        assertFalse(open.phase.needsAttention)
        assertTrue("an open train is behind one that is being checked", TrainPhase.WAITING_FOR_PREVIOUS_TRAIN.progress < TrainPhase.INTEGRATING.progress)
        assertEquals(TrainPhase.WAITING_FOR_PREVIOUS_TRAIN, TrainPhase.parse("waiting_for_previous_train"))
    }

    @Test
    fun `the collecting train with nothing ahead but main's own checks says so, on the first step, behind a train being checked`() {
        val plan = Plan.decode(
            """{"now_ms": 1, "themes": [], "lanes": [], "order": [], "cards": [], "trains": [
              {"id": "t", "short": "t", "name": "integ-7", "base": "main", "pushed_sha": null, "state": "integrating", "state_since": 5,
               "phase": "waiting_for_main", "phase_since": 9, "actor": "m", "created_at": 1, "landed_at": null, "lanes": [], "ci_subject": ""}]}""",
        )
        val card = PlanSections.of(plan).trains.single()
        assertEquals(TrainPhase.WAITING_FOR_MAIN, card.phase)
        assertEquals("Waiting for main's checks to pass", card.phase.word)
        assertEquals("waiting for main's checks to pass", card.phase.sentence)
        assertEquals("Waiting for main's checks to pass \u00b7 step 1 of 3", PlanLadder.compact(card.phase))
        assertEquals("Waiting for main's checks to pass", PlanLadder.label(0, PlanLadder.step(card.phase), card.phase))
        assertEquals(card.phase.explanation, PlanLadder.explanation(0, 0, card.phase))
        assertTrue(card.phase.explanation.endsWith(".") && !card.phase.needsAttention)
        assertEquals(TrainPhase.WAITING_FOR_PREVIOUS_TRAIN.progress, TrainPhase.WAITING_FOR_MAIN.progress)
        assertTrue(TrainPhase.WAITING_FOR_MAIN.progress < TrainPhase.INTEGRATING.progress)
        assertEquals(TrainPhase.WAITING_FOR_MAIN, TrainPhase.parse("waiting_for_main"))
    }

    @Test
    fun `a lane aboard a train says nothing, except to wait on the owner or once landed, and none says it holds the train`() {
        val lanes = sections("train-lane-notes").trains.first().lanes
        fun note(name: String) = lanes.first { it.name == name }.note
        assertNull("'Ready' said nothing: ready for what?", note("aboard"))
        assertNull("a lane fixing aboard is simply aboard", note("holder"))
        assertEquals("Landed · 3f78754f", note("gone"))
        assertEquals("Needs you", note("blocked"))
        assertTrue(lanes.first { it.name == "blocked" }.noteNeedsAttention)
        assertFalse(lanes.first { it.name == "aboard" }.noteNeedsAttention)
        assertTrue(lanes.mapNotNull { it.note }.none { it.contains("holding") })
    }

    @Test
    fun `the sidebar's Landing group holds the trains as Train N with their titles, and their lanes inside`() {
        val tree = LaneTree.of(plan("train-lane-notes"), sections("train-lane-notes"), java.util.Locale.US)
        assertEquals(listOf("Landing", "In review"), tree.groups.map { it.title })
        val landing = tree.groups.first()
        assertTrue(landing.lanes.isEmpty())
        assertEquals(listOf("Train 5", "Train 6"), landing.trains.map { it.label })
        val train = landing.trains[0]
        assertEquals("Train 5", train.label)
        assertEquals("People on a shared runner; sign in on the web", train.subtitle)
        assertEquals(listOf("aboard", "holder", "gone", "blocked"), train.lanes.map { it.name })
        assertEquals("", train.lanes.first { it.name == "aboard" }.state)
        assertEquals(listOf("ov-16"), tree.cardsOf("l-holder").map { it.key })
        assertTrue("no train-named group sits beside Building or Waiting", tree.groups.drop(1).all { it.trains.isEmpty() })
    }

    @Test
    fun `the ladder is three plain steps and none is a word the runner has no state for`() {
        assertEquals(listOf("Checking here", "Checking on CI", "Landed"), PlanLadder.steps)
        val said = (PlanLadder.steps + TrainPhase.entries.map { it.word } + TrainPhase.entries.map { it.sentence }).joinToString(" ").lowercase()
        for (old in listOf("picked", "integrat", "gated", "gating")) assertFalse(old, said.contains(old))
        assertEquals("Checking here · step 1 of 3", PlanLadder.compact(TrainPhase.INTEGRATING))
        assertEquals("combining and gating are one step", "Checking here · step 1 of 3", PlanLadder.compact(TrainPhase.GATING_LOCALLY))
        assertEquals("Checking on CI · step 2 of 3", PlanLadder.compact(TrainPhase.WAITING_FOR_CI))
        assertEquals("Waiting for lanes · step 1 of 3", PlanLadder.compact(TrainPhase.WAITING_FOR_LANES))
        assertEquals("CI failed · step 2 of 3", PlanLadder.compact(TrainPhase.CI_FAILED))
        assertEquals("Stalled · step 1 of 3", PlanLadder.compact(TrainPhase.STALLED))
        assertEquals("Ready to land · step 3 of 3", PlanLadder.compact(TrainPhase.READY_TO_LAND))
        assertEquals("Landed", PlanLadder.compact(TrainPhase.LANDED))
    }

    @Test
    fun `every step and every phase explains itself in one line, and the step it stands on says the phase's`() {
        assertEquals(PlanLadder.steps.size, PlanLadder.explanations.size)
        for (text in PlanLadder.explanations + TrainPhase.entries.map { it.explanation }) {
            assertTrue(text, text.isNotEmpty() && !text.contains("\n") && text.endsWith("."))
        }
        assertEquals(TrainPhase.CI_FAILED.explanation, PlanLadder.explanation(1, 1, TrainPhase.CI_FAILED))
        assertEquals(PlanLadder.explanations[0], PlanLadder.explanation(0, 1, TrainPhase.CI_FAILED))
        assertEquals("The full test run, on the runner.", PlanLadder.explanations[0])
        assertEquals("Reviewed lanes land on main together, after one round of checks.", TRAIN_EXPLANATION)
    }

    @Test
    fun `the Next line counts the lanes that wait while none is ranked, and says nothing queued only when none waits`() {
        val waiting = sections("next-empty")
        assertTrue(waiting.waiting.next.isEmpty())
        assertEquals(3, waiting.waiting.count)
        val said = PlanGlanceSentence.of(waiting)
        assertEquals("3 waiting, none ranked", said.nextLine)
        assertTrue(said.text.endsWith("Next: 3 waiting, none ranked."))
        assertEquals(PlanGlanceSentence.NOTHING_QUEUED, PlanGlanceSentence.of(sections("nothing")).nextLine)
        assertFalse(PlanGlanceSentence.of(sections("next-ranked")).nextLine.contains("none ranked"))
    }

    @Test
    fun `the sidebar lists no agent that has finished, a runner that lists agents lists the ones that exist`() {
        val plan = plan("train-lane-notes")
        fun agents(p: Plan) = LaneTree.of(p, PlanSections.of(p), java.util.Locale.US).groups.flatMap { it.lanes }.first { it.name == "loose" }.agents.map { it.who }
        assertTrue(plan.agentsSpend != null)
        assertEquals("a finished agent stayed in the sidebar", emptyList<String>(), agents(plan))
        assertEquals("a runner that totals the day lists the ones that exist", emptyList<String>(), agents(plan.copy(agents = emptyList())))
        assertEquals(listOf("Builder · Sonnet"), agents(plan.copy(agents = emptyList(), agentsSpend = null)))
    }

    @Test
    fun `no train word says holding, this Mac or Waits for you, and the explanations tell one story`() {
        val card = sections("old-runner-held-by").trains.first()
        assertTrue("a runner before Train 119 still sends held_by; nothing is drawn from it", card.lanes.all { it.note == null || it.note == "Needs you" })
        val said = (TrainPhase.entries.flatMap { listOf(it.word, it.sentence, it.explanation) } + PlanLadder.explanations + listOf(TRAIN_EXPLANATION)).joinToString(" ")
        for (old in listOf("Holding", "holding", "this Mac", "Waits for you")) assertFalse(old, said.contains(old))
        assertEquals("Pushed, with CI running its checks.", PlanLadder.explanations[1])
        assertEquals("Every check passed, so it's ready to land.", TrainPhase.READY_TO_LAND.explanation)
    }

    @Test
    fun `a facts line keeps each span whole, so it breaks between spans only`() {
        val line = "3f78754f \u00b7 Running \u00b7 2 of 5 jobs done \u00b7 1 h \u00b7 Integrator \u00b7 Opus \u00b7 1.4M"
        val kept = TrainWords.unbroken(line)
        assertTrue(kept.contains("1\u00a0h") && kept.contains("2\u00a0of\u00a05\u00a0jobs\u00a0done"))
        assertEquals(line, kept.replace('\u00a0', ' '))
        assertEquals(7, kept.split(" \u00b7 ").size)
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
