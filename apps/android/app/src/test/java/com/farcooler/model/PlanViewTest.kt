package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import java.io.File

/**
 * The Plan's view model on the phone (ov-1016): that the golden answer the core gave
 * (`test/fixtures/plan-view-material.json`, also the Rust test's) decodes into every slot the renderer draws, that the
 * request the app builds is the request the golden was made from, and what the glance band and strip say over the core's
 * view. The decode and the request run without the native core; the glance tests ask the host JNI build
 * (`cargo build -p farcooler-jni`, `-Pfarcooler.nativeLibDir=<repo>/target/debug`).
 */
class PlanViewTest {
    private val golden = Json.parseToJsonElement(repositoryFile("test/fixtures/plan-view-material.json")).jsonObject

    @Before @After fun reset() = PlanViews.reset()

    @Test
    fun `the engine landing queue uses the shared golden including long titles and failures`() {
        val plan = Plan.decode(repositoryFile("test/fixtures/plan-landing-queue.json"))
        val view = PlanViews.of(plan)!!
        val groups = view.landingGroups!!
        assertEquals(4, groups.size)
        assertTrue(groups[0].lines.any { it.contains("complete long ticket title") })
        assertTrue(groups[2].lines.contains("Split after a gate failed"))
        assertTrue(groups[2].lines.contains("Failed job: cargo test --workspace"))
        assertTrue(groups[3].lines.contains("error: expected a passing gate"))
    }

    @Test
    fun `the golden answer decodes into every slot the phone draws`() {
        val view = PlanViews.decode(golden.getValue("answer").toString())!!
        assertEquals(
            listOf(
                PlanViewKind.NeedsYou, PlanViewKind.Landing, PlanViewKind.InReview, PlanViewKind.Building,
                PlanViewKind.Waiting, PlanViewKind.Agents, PlanViewKind.Themes, PlanViewKind.LandedToday,
            ),
            view.sections.map { it.kind },
        )
        assertEquals("Reviewed tickets land on main together, after one round of checks.", view.section(PlanViewKind.Landing)?.help)
        assertTrue(view.landings.isEmpty())
        val fixing = view.inReview.first { it.key == "ov-312" }
        assertEquals(PlanViewMark.Fixing, fixing.mark)
        assertEquals("Fixing · round 2", fixing.status)
        val stale = view.inReview.first { it.key == "ov-482" }
        assertEquals(listOf(PlanViewNoteKind.Warning), stale.notes.map { it.kind })
        assertTrue(stale.notes.single().attention && stale.markAttention)
        val running = view.building.first { it.key == "ov-310" }
        assertEquals("Builder · sonnet", running.agent?.who)
        assertEquals("Prepare workspace · Approval required", running.notes.single { it.kind == PlanViewNoteKind.Run }.text)
        assertEquals(listOf(PlanViewWaitingKind.NotRanked, PlanViewWaitingKind.OnYou), view.waiting.map { it.kind })
        assertEquals(5, view.waitingCount)
        assertTrue(view.agents.header.startsWith("3 agents, including the orchestrator"))
        assertEquals("Orchestrator · opus", view.agents.rows.first().who)
        assertEquals("Mac look and feel", view.themes.finished.single().name)
        assertEquals("Finished", view.themes.finishedTitle)
        assertEquals("0 of 5 done", view.themes.tiles.first().progress)
    }

    @Test
    fun `a reply that is not a view decodes to none`() {
        assertNull(PlanViews.decode(null))
        assertNull(PlanViews.decode("""{"schema":1,"view":null,"error":"invalid"}"""))
        assertNull(PlanViews.decode("not json"))
    }

    @Test
    fun `the request is the plan as the runner sent it, the board's context in Material's style`() {
        val request = golden.getValue("request").jsonObject
        val plan = Plan.decode(request.getValue("plan").toString())
        val statuses = mapOf("t-ov-300" to TaskStatus.NEEDS_DECISION)
        val sent = Json.parseToJsonElement(PlanViews.request(plan, 2, statuses, emptyMap())!!).jsonObject
        assertEquals(request.getValue("plan"), sent.getValue("plan"))
        assertEquals("material", sent.getValue("context").jsonObject.getValue("style").jsonPrimitive.content)
        assertEquals(request.getValue("context"), sent.getValue("context").jsonObject.let { JsonObject(it - "agents_supported" - "activity" - "utc_offset") })
        assertNull("a plan built by hand has no wire to send", PlanViews.request(Plan(), 0, emptyMap(), emptyMap()))
    }

    @Test
    fun `the view is the core's answer, asked once for the same plan, and none when the core says invalid`() {
        val plan = Plan.decode(golden.getValue("request").jsonObject.getValue("plan").toString())
        var asked = 0
        PlanViews.bridge = { asked++; golden.getValue("answer").toString() }
        val first = PlanViews.of(plan, 2)
        assertNotNull(first)
        assertEquals(8, first!!.sections.size)
        assertSame(first, PlanViews.of(plan, 2))
        assertEquals(1, asked)
        PlanViews.of(plan, 3)
        assertEquals("a new Needs You count is a new question", 2, asked)
        PlanViews.reset()
        PlanViews.bridge = { """{"schema":1,"view":null,"error":"invalid"}""" }
        assertNull(PlanViews.of(plan, 2))
    }

    // The glance band and the strip, over the core's view (the host JNI build).

    private fun ticket(key: String, stage: String, extra: String = "", title: String = "") =
        """{"task": "t-$key", "key": "$key", "title": "$title", "status": "In Progress", "stage": "$stage", "reason": "", "worktree_path": "", "branch": "",
           "stage_since": 1, "stale": false, "fix_rounds": 0, "work": [], "spend": {}$extra}"""

    private fun ranked(vararg tickets: String) =
        Plan.decode("""{"now_ms": 1, "themes": [], "lines": [${tickets.joinToString(",")}], "cards": [], "landings": []}""")

    private fun queued(key: String, rank: Int, title: String = "") = ticket(key, "waiting", ""","plan_rank": $rank""", title)

    @Test
    fun `the strip carries the band, and its old parts are unchanged`() {
        val plan = ranked(ticket("mac-vis", "building"), queued("p", 1, "plan-phones"), queued("v", 2, "vis-lint"))
        val strip = PlanStrip.of(plan, needsYou = 0, orchestrator = null)
        assertEquals("1 building", strip.band?.now)
        assertEquals("plan-phones, then vis-lint", strip.band?.next)
        assertEquals("mac-vis Building · next: plan-phones", strip.text)
    }

    @Test
    fun `a ticket waiting on a decision is never next, in the strip or its band`() {
        val plan = ranked(queued("a", 1), queued("b", 2))
        assertEquals("a, then b", PlanStrip.of(plan, needsYou = 0, orchestrator = null).band?.next)
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
        assertEquals("Reps: each person's asks reach the orchestrator, then skill-init…, then Onboarding: skill-init explains what the manager skill is for, in the…", next)
    }

    @Test
    fun `historical landings do not appear in the Now line`() {
        val plan = Plan.decode(
            """{"now_ms": 1, "themes": [], "lines": [], "cards": [], "landings": [{"id":"t89","short":"t","number":89,"title":"Old landing","summary":"","route":"push","base":"main","pushed_sha":null,"state":"pushed","state_since":5,"phase":"waiting_for_ci","phase_since":5,"actor":"m","created_at":1,"landed_at":null,"tickets":[],"ci_subject":""}]}""",
        )
        val view = PlanViews.of(plan)!!
        assertTrue(view.landings.isEmpty())
        assertFalse(PlanGlanceSentence.of(view).now.orEmpty().contains("Landing 89"))
    }

    @Test
    fun `agents_spend decodes, and a runner without it leaves null`() {
        val base = """{"now_ms":1,"themes":[],"lines":[],"cards":[]"""
        assertEquals(7L, Plan.decode(base + ""","agents_spend":{"input_tokens":7,"cost_micros":9}}""").agentsSpend?.totalTokens)
        assertNull(Plan.decode("$base}").agentsSpend)
        assertTrue(Plan.decode("$base}").agents.isEmpty())
    }

    private fun assertSame(expected: Any?, actual: Any?) = org.junit.Assert.assertSame(expected, actual)

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
