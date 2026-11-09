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
 * What of Landing (ov-522, ov-872) is still Kotlin's: the sidebar's Landing group, the Next line that never says "Nothing
 * queued" while tickets wait, and the facts line's spans. The landings' words, order and ladder are the core's (ov-1016),
 * pinned by `crates/client/src/plan_view/tests.rs`. The tree and the line read the core's view over the host JNI build.
 */
class PlanLandingTest {
    private val cases = Json.parseToJsonElement(repositoryFile("test/fixtures/plan-sections-cases.json")).jsonObject["cases"]!!.jsonArray

    private fun plan(name: String): Plan =
        Plan.decode(cases.first { it.jsonObject["name"]!!.jsonPrimitive.content == name }.jsonObject["plan"]!!.jsonObject)

    private fun view(name: String): PlanView {
        val case = cases.first { it.jsonObject["name"]!!.jsonPrimitive.content == name }.jsonObject
        val statuses = case["statuses"]!!.jsonObject.mapNotNull { (k, v) -> TaskStatus.parse(v.jsonPrimitive.content)?.let { k to it } }.toMap()
        return PlanViews.of(plan(name), statuses = statuses) ?: throw AssertionError("$name: the core gave no view")
    }

    @Test
    fun `the sidebar's Landing group holds the landings as Landing N with their titles, and their tickets inside`() {
        val tree = TicketTree.of(plan("landing-ticket-notes"), view("landing-ticket-notes"))
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
    fun `the Next line counts the tickets that wait while none is ranked, and says nothing queued only when none waits`() {
        val waiting = view("next-empty")
        assertTrue(waiting.waiting(PlanViewWaitingKind.NEXT).isEmpty())
        assertEquals(3, waiting.waitingCount)
        val said = PlanGlanceSentence.of(waiting)
        assertEquals("3 waiting, none ranked", said.nextLine)
        assertTrue(said.text.endsWith("Next: 3 waiting, none ranked."))
        assertEquals(PlanGlanceSentence.NOTHING_QUEUED, PlanGlanceSentence.of(view("nothing")).nextLine)
        assertFalse(PlanGlanceSentence.of(view("next-ranked")).nextLine.contains("none ranked"))
    }

    @Test
    fun `the sidebar lists the agents the plan names on a ticket, and none when it names none`() {
        val plan = plan("landing-ticket-notes")
        fun agents(p: Plan) = TicketTree.of(p, PlanViews.of(p)).groups.flatMap { it.tickets }.first { it.key == "ov-11" }.agents.map { it.who }
        assertEquals(listOf("Builder \u00b7 sonnet"), agents(plan))
        assertEquals("a plan that names no agents lists none on a ticket", emptyList<String>(), agents(plan.copy(agents = emptyList())))
    }

    @Test
    fun `a facts line keeps each span whole, so it breaks between spans only`() {
        val line = "3f78754f \u00b7 Running \u00b7 2 of 5 jobs done \u00b7 1 h \u00b7 Lander \u00b7 opus \u00b7 1.4M"
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
