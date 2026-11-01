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
 * Landing (ov-522, ov-872): the landings' section is Landing, never Shipping; a landing is "Landing N" with its title under it; a
 * ticket aboard a landing says nothing unless it waits on the owner or has landed; an open landing waiting for the one ahead
 * says so; the ladder is three plain steps; and the Next line never says "Nothing queued" while tickets wait. The twin of
 * AgentKit's `PlanLandingTests`, over the same shared cases.
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
    fun `no section is called Shipping, and the landings' section is Landing, above In review`() {
        assertEquals("Landing", PlanSections.Kind.LANDING.title)
        assertEquals("Nothing is landing right now.", PlanSections.Kind.LANDING.emptyWords)
        assertTrue(PlanSections.Kind.entries.all { !it.title.lowercase().contains("shipping") })
        val kinds = PlanSections.Kind.entries
        assertEquals(PlanSections.Kind.IN_REVIEW, kinds[kinds.indexOf(PlanSections.Kind.LANDING) + 1])
        assertEquals(listOf(PlanSections.Kind.LANDING, PlanSections.Kind.IN_REVIEW), sections("landing-ticket-notes").shown.take(2))
    }

    @Test
    fun `a ticket in landing with no live landing is In review, and on a landing it is in the landing`() {
        val s = sections("landing-ticket-notes")
        assertTrue(s.inReview.map { it.ticket.key }.contains("ov-11"))
        assertFalse(s.landings.flatMap { it.tickets }.map { it.key }.contains("ov-11"))
        assertTrue(s.landings.flatMap { it.tickets }.map { it.key }.contains("ov-15"))
        assertFalse(s.inReview.map { it.ticket.key }.contains("ov-15"))
        assertTrue("a ticket being fixed is in review, not building", s.inReview.map { it.ticket.key }.containsAll(listOf("ov-13", "ov-14")))
    }

    @Test
    fun `an open landing waiting for the one ahead says so in place of its first step`() {
        val open = sections("landing-ticket-notes").landings.first { it.landing.number == 6 }
        assertEquals(LandingPhase.WAITING_FOR_PREVIOUS_LANDING, open.phase)
        assertEquals("Waiting for the earlier landing \u00b7 step 1 of 3", PlanLadder.compact(open.phase))
        assertEquals("Waiting for the earlier landing", PlanLadder.label(0, PlanLadder.step(open.phase), open.phase))
        assertEquals("waiting for the earlier landing", open.phase.sentence)
        assertTrue(open.phase.explanation.endsWith("."))
        assertFalse(open.phase.needsAttention)
        assertTrue("an open landing is behind one that is being checked", LandingPhase.WAITING_FOR_PREVIOUS_LANDING.progress < LandingPhase.INTEGRATING.progress)
        assertEquals(LandingPhase.WAITING_FOR_PREVIOUS_LANDING, LandingPhase.parse("waiting_for_previous_landing"))
    }

    @Test
    fun `the collecting landing with nothing ahead but main's own checks says so, on the first step, behind a landing being checked`() {
        val plan = Plan.decode(
            """{"now_ms": 1, "themes": [], "lines": [], "cards": [], "landings": [
              {"id": "t", "short": "t", "number": 7, "base": "main", "pushed_sha": null, "state": "open", "state_since": 5,
               "phase": "waiting_for_main", "phase_since": 9, "collecting": true, "actor": "m", "created_at": 1, "landed_at": null, "tickets": [], "ci_subject": ""}]}""",
        )
        val card = PlanSections.of(plan).landings.single()
        assertEquals(LandingPhase.WAITING_FOR_MAIN, card.phase)
        assertTrue(card.landing.collecting)
        assertEquals("Waiting for main's checks to pass", card.phase.word)
        assertEquals("waiting for main's checks to pass", card.phase.sentence)
        assertEquals("Waiting for main's checks to pass \u00b7 step 1 of 3", PlanLadder.compact(card.phase))
        assertEquals("Waiting for main's checks to pass", PlanLadder.label(0, PlanLadder.step(card.phase), card.phase))
        assertEquals(card.phase.explanation, PlanLadder.explanation(0, 0, card.phase))
        assertTrue(card.phase.explanation.endsWith(".") && !card.phase.needsAttention)
        assertEquals(LandingPhase.WAITING_FOR_PREVIOUS_LANDING.progress, LandingPhase.WAITING_FOR_MAIN.progress)
        assertTrue(LandingPhase.WAITING_FOR_MAIN.progress < LandingPhase.INTEGRATING.progress)
        assertEquals(LandingPhase.WAITING_FOR_MAIN, LandingPhase.parse("waiting_for_main"))
    }

    @Test
    fun `a ticket aboard a landing says nothing, except to wait on the owner or once landed, and none says it holds the landing`() {
        val tickets = sections("landing-ticket-notes").landings.first { it.landing.number == 5 }.tickets
        fun note(key: String) = tickets.first { it.key == key }.note
        assertNull("'Ready' said nothing: ready for what?", note("ov-15"))
        assertNull("a ticket being fixed aboard is simply aboard", note("ov-16"))
        assertEquals("Landed \u00b7 3f78754f", note("ov-17"))
        assertEquals("Needs you", note("ov-18"))
        assertTrue(tickets.first { it.key == "ov-18" }.noteNeedsAttention)
        assertFalse(tickets.first { it.key == "ov-15" }.noteNeedsAttention)
        assertTrue(tickets.mapNotNull { it.note }.none { it.contains("holding") })
    }

    @Test
    fun `the sidebar's Landing group holds the landings as Landing N with their titles, and their tickets inside`() {
        val tree = TicketTree.of(plan("landing-ticket-notes"), sections("landing-ticket-notes"), java.util.Locale.US)
        assertEquals(listOf("Landing", "In review"), tree.groups.map { it.title })
        val landing = tree.groups.first()
        assertTrue(landing.tickets.isEmpty())
        assertEquals(listOf("Landing 5", "Landing 6"), landing.landings.map { it.label })
        val first = landing.landings[0]
        assertEquals("People on a shared runner; sign in on the web", first.subtitle)
        assertEquals(listOf("ov-15", "ov-16", "ov-17", "ov-18"), first.tickets.map { it.key })
        assertEquals("", first.tickets.first { it.key == "ov-15" }.state)
        assertEquals("ov-14", tree.cardOf(plan("landing-ticket-notes").tickets.first { it.key == "ov-14" }.task)?.key)
        assertTrue("no landing-named group sits beside Building or Waiting", tree.groups.drop(1).all { it.landings.isEmpty() })
    }

    @Test
    fun `the ladder is three plain steps and none is a word the runner has no state for`() {
        assertEquals(listOf("Checking here", "Checking on CI", "Landed"), PlanLadder.steps)
        val said = (PlanLadder.steps + LandingPhase.entries.map { it.word } + LandingPhase.entries.map { it.sentence }).joinToString(" ").lowercase()
        for (old in listOf("picked", "integrat", "gated", "gating")) assertFalse(old, said.contains(old))
        assertEquals("Checking here \u00b7 step 1 of 3", PlanLadder.compact(LandingPhase.INTEGRATING))
        assertEquals("combining and gating are one step", "Checking here \u00b7 step 1 of 3", PlanLadder.compact(LandingPhase.GATING_LOCALLY))
        assertEquals("Checking on CI \u00b7 step 2 of 3", PlanLadder.compact(LandingPhase.WAITING_FOR_CI))
        assertEquals("Waiting for tickets \u00b7 step 1 of 3", PlanLadder.compact(LandingPhase.WAITING_FOR_TICKETS))
        assertEquals("CI failed \u00b7 step 2 of 3", PlanLadder.compact(LandingPhase.CI_FAILED))
        assertEquals("Stalled \u00b7 step 1 of 3", PlanLadder.compact(LandingPhase.STALLED))
        assertEquals("Ready to land \u00b7 step 3 of 3", PlanLadder.compact(LandingPhase.READY_TO_LAND))
        assertEquals("Landed", PlanLadder.compact(LandingPhase.LANDED))
    }

    @Test
    fun `every step and every phase explains itself in one line, and the step it stands on says the phase's`() {
        assertEquals(PlanLadder.steps.size, PlanLadder.explanations.size)
        for (text in PlanLadder.explanations + LandingPhase.entries.map { it.explanation }) {
            assertTrue(text, text.isNotEmpty() && !text.contains("\n") && text.endsWith("."))
        }
        assertEquals(LandingPhase.CI_FAILED.explanation, PlanLadder.explanation(1, 1, LandingPhase.CI_FAILED))
        assertEquals(PlanLadder.explanations[0], PlanLadder.explanation(0, 1, LandingPhase.CI_FAILED))
        assertEquals("The full test run, on the runner.", PlanLadder.explanations[0])
        assertEquals("Reviewed tickets land on main together, after one round of checks.", LANDING_EXPLANATION)
    }

    @Test
    fun `the Next line counts the tickets that wait while none is ranked, and says nothing queued only when none waits`() {
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
    fun `the sidebar lists only the agents that exist on a ticket`() {
        val plan = plan("landing-ticket-notes")
        fun agents(p: Plan) = TicketTree.of(p, PlanSections.of(p), java.util.Locale.US).groups.flatMap { it.tickets }.first { it.key == "ov-11" }.agents.map { it.who }
        assertEquals(listOf("Builder \u00b7 Sonnet"), agents(plan))
        assertEquals("a runner that lists no agents lists none on a ticket", emptyList<String>(), agents(plan.copy(agents = emptyList())))
    }

    @Test
    fun `no landing word says holding, this Mac or Waits for you, and the explanations tell one story`() {
        val said = (LandingPhase.entries.flatMap { listOf(it.word, it.sentence, it.explanation) } + PlanLadder.explanations + listOf(LANDING_EXPLANATION)).joinToString(" ")
        for (old in listOf("Holding", "holding", "this Mac", "Waits for you", "lane", "train")) assertFalse(old, said.contains(old))
        assertEquals("Pushed, with CI running its checks.", PlanLadder.explanations[1])
        assertEquals("Every check passed, so it's ready to land.", LandingPhase.READY_TO_LAND.explanation)
    }

    @Test
    fun `a facts line keeps each span whole, so it breaks between spans only`() {
        val line = "3f78754f \u00b7 Running \u00b7 2 of 5 jobs done \u00b7 1 h \u00b7 Lander \u00b7 Opus \u00b7 1.4M"
        val kept = LandingWords.unbroken(line)
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
