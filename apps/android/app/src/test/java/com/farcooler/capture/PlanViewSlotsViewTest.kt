package com.farcooler.capture

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import com.farcooler.model.Plan
import com.farcooler.model.PlanReadState
import com.farcooler.model.PlanViews
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.PlanFolds
import com.farcooler.ui.planItems
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The phone draws the words the core's view gives it and no others (ov-1016): the golden answer
 * (`test/fixtures/plan-view-material.json`, the Rust test's own) stands in for the core, so this needs no native library,
 * and each slot it fills is read back off the screen. Where the app's own Kotlin used to word a ticket, a landing, an agent
 * or a tile, the answer here is made to say something Kotlin could not have, and the screen says it.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w320dp-h4200dp-xhdpi")
class PlanViewSlotsViewTest {
    private val golden = Json.parseToJsonElement(PlanFixtures.repositoryFile("test/fixtures/plan-view-material.json")).jsonObject
    private val plan = Plan.decode(golden.getValue("request").jsonObject.getValue("plan").toString())

    @Before @After fun reset() = PlanViews.reset()

    private fun drawn(answer: String, folds: PlanFolds = PlanFolds.Defaults): Map<String, SemanticsProbe.Drawn> {
        PlanViews.bridge = { answer }
        val content: @Composable () -> Unit = {
            FarCoolerTheme {
                LazyColumn(Modifier.fillMaxSize()) {
                    planItems(PlanReadState.Loaded(plan), emptyMap(), onOpen = {}, onRetry = {}, folds = folds, needsYou = 2, themeColumns = 1)
                }
            }
        }
        return SemanticsProbe.described(content = content)
    }

    private fun golden() = golden.getValue("answer").toString()

    @Test fun `every section the view lists is drawn in its order, with its own title and count`() {
        val tags = drawn(golden())
        val order = listOf("plan-needs-you", "plan-landing", "plan-in-review", "plan-building", "plan-waiting", "plan-agents", "plan-themes")
        for (tag in order) assertTrue("$tag is drawn: ${tags.keys}", tag in tags)
        val tops = order.map { tags.getValue(it).bounds.top }
        assertEquals(tops.sorted(), tops)
        assertTrue(tags.getValue("plan-in-review").words, "In review" in tags.getValue("plan-in-review").words && "3" in tags.getValue("plan-in-review").words)
    }

    @Test fun `a landing says the view's label, ladder, lander and tickets`() {
        val tags = drawn(golden())
        assertEquals("Landing 86", tags.getValue("plan-landing-86-label").words.substringBefore(" |"))
        assertEquals("Waiting for tickets · step 1 of 3", tags.getValue("plan-landing-86-phase").words.substringBefore(" |"))
        assertEquals("Stalled · step 1 of 3", tags.getValue("plan-landing-84-phase").words.substringBefore(" |"))
        assertTrue(tags.getValue("plan-landing-86-words").words, "Lander · opus · 1.4M" in tags.getValue("plan-landing-86-words").words.replace(' ', ' '))
        assertTrue("plan-landing-ticket-ov-487" in tags)
    }

    @Test fun `a card says its second line and its notes as the view wrote them`() {
        val tags = drawn(golden())
        assertTrue(tags.getValue("plan-ticket-ov-312-second").words, "Fixing · round 2 · ov-312" == tags.getValue("plan-ticket-ov-312-second").words)
        assertTrue(tags.getValue("plan-ticket-ov-310-agent").words, "Builder · sonnet · 380K" in tags.getValue("plan-ticket-ov-310-agent").words)
        assertFalse("the agent is said once, on its own line", "Builder" in tags.getValue("plan-ticket-ov-310-second").words)
        assertTrue(tags.getValue("plan-ticket-ov-310-run").words, "Prepare workspace · Approval required" in tags.getValue("plan-ticket-ov-310-run").words)
        assertTrue(tags.getValue("plan-ticket-ov-311-theme").words, "Trustworthy agent view" in tags.getValue("plan-ticket-ov-311-theme").words)
        assertTrue("a warning is a note of the card", "No move in 30 h" in tags.getValue("plan-ticket-ov-482").words)
    }

    @Test fun `agents and themes say the view's words`() {
        val tags = drawn(golden())
        assertTrue(tags.getValue("plan-agents-header").words, tags.getValue("plan-agents-header").words.startsWith("3 agents, including the orchestrator"))
        assertTrue(tags.getValue("plan-agent-pane:orch").words, "Orchestrator · opus" in tags.getValue("plan-agent-pane:orch").words)
        assertTrue(tags.getValue("plan-tile-Trustworthy agent view-progress").words, "0 of 5 done" in tags.getValue("plan-tile-Trustworthy agent view-progress").words)
        assertTrue(tags.getValue("plan-finished-tile").words, "Mac look and feel" in tags.getValue("plan-finished-tile").words)
    }

    @Test fun `what the view says is what is drawn, whatever the app would have said`() {
        val said = golden().replace("Waiting for tickets", "Under way elsewhere").replace("0 of 5 done", "nought of five")
        val tags = drawn(said)
        assertTrue(tags.getValue("plan-landing-86-phase").words, tags.getValue("plan-landing-86-phase").words.startsWith("Under way elsewhere"))
        assertTrue(tags.getValue("plan-tile-Trustworthy agent view-progress").words.startsWith("nought of five"))
    }

    @Test fun `Waiting for the next landing is drawn folded, and opens`() {
        // The golden view with its sections cut to this one and one parked ticket: every field the generated PlanView
        // requires is there, in the camelCase the core writes.
        val answer = golden.getValue("answer").jsonObject
        val base = answer.getValue("view").jsonObject
        val parked = JsonObject(
            base.getValue("inReview").jsonArray.first().jsonObject + mapOf(
                "id" to JsonPrimitive("t-a"), "key" to JsonPrimitive("ov-9"), "heading" to JsonPrimitive("Parked ticket"),
                "stage" to JsonPrimitive("waiting"), "status" to JsonPrimitive("Waiting"), "second" to JsonPrimitive("Waiting · ov-9"),
                "mark" to JsonPrimitive("waiting"), "accessibility" to JsonPrimitive("Parked ticket"),
            ),
        )
        val section = JsonObject(
            mapOf(
                "kind" to JsonPrimitive("on_landed_landing"), "title" to JsonPrimitive("Waiting for the next landing"),
                "count" to JsonPrimitive(1), "folded" to JsonPrimitive(true), "empty" to JsonNull, "help" to JsonNull,
            ),
        )
        val view = JsonObject(
            answer + ("view" to JsonObject(base + mapOf("sections" to JsonArray(listOf(section)), "onLandedLanding" to JsonArray(listOf(parked))))),
        ).toString()
        val closed = drawn(view)
        assertTrue("plan-on-landed-landing" in closed)
        assertFalse("its rows wait behind the fold", "plan-ticket-ov-9" in closed)
        val open = drawn(view, PlanFolds(listOf("on-landed-landing")) {})
        assertTrue("plan-ticket-ov-9" in open)
    }

    @Test fun `a plan the core cannot read says so and draws no sections`() {
        val tags = drawn("""{"schema":1,"view":null,"error":"invalid"}""")
        assertTrue("plan-unreadable" in tags)
        assertFalse("plan-landing" in tags)
    }
}
