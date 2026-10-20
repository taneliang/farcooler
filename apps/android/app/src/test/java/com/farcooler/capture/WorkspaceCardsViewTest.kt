package com.farcooler.capture

import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.core.view.drawToBitmap
import androidx.compose.foundation.layout.Column
import com.farcooler.ui.AgentsSheetContent
import androidx.compose.ui.unit.dp
import androidx.compose.ui.platform.testTag
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import com.farcooler.model.Plan
import com.farcooler.model.PlanStrip
import com.farcooler.model.RunningAgent
import com.farcooler.model.RunningAgents
import com.farcooler.model.Terminal
import com.farcooler.ui.AgentsList
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.WorkspaceCards
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import android.view.View
import android.view.ViewGroup

/**
 * The two cards at the top of a workspace's conversation, and the agents view (ov-641), as the app draws them: Agents
 * beside Now/Next at the same height, gone when none run, alone beside a wide layout's plan, stacked at large font
 * sizes, and a row of the agents view opening its agent. Each state is also captured, light and dark, at the narrowest
 * phone, a phone, a phone at 1.6x font, and a tablet. Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class WorkspaceCardsViewTest {
    private val plan = Plan.decode(Json.parseToJsonElement(repositoryFile("test/fixtures/plan-seeded.json")).jsonObject["plan"]!!.jsonObject)
    private val strip = PlanStrip.of(plan, 2, Terminal(id = "o", state = "running", activity = "working", role = "orchestrator"))
    private val quiet = PlanStrip.of(Plan(), 0, Terminal(id = "o", state = "running", activity = "idle", role = "orchestrator"))
    private val busy = AgentsFixtures.running()

    @androidx.compose.runtime.Composable
    private fun Host(strip: PlanStrip?, agents: RunningAgents, showsPlan: Boolean = true) = FarCoolerTheme {
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surface) {
            Column(Modifier.fillMaxSize()) {
                WorkspaceCards(strip, agents, onOpenPlan = {}, onOpenAgents = {}, showsPlan = showsPlan)
                Text("The conversation goes here.", Modifier.fillMaxWidth().padding(16.dp))
            }
        }
    }

    @Test
    fun `Agents sits beside Now and Next at its height, and Now and Next says what it says empty`() {
        val tags = SemanticsProbe.described { Host(strip, busy) }
        val card = tags.getValue("agents-card").bounds
        val plan = tags.getValue("plan-strip").bounds
        assertTrue("beside it: $card $plan", card.left >= plan.right)
        assertEquals("one height: $card $plan", plan.height, card.height, 2f)
        assertTrue("compact: ${card.width}", card.width < plan.width / 3)
        assertEquals("4 agents", tags.getValue("agents-card").words.substringBefore(" |"))
        assertTrue("the band's two lines are drawn", "plan-band-now" in tags && "plan-band-next" in tags)
        val empty = SemanticsProbe.described { Host(quiet, RunningAgents.EMPTY) }
        assertTrue("No lanes moving", empty.getValue("plan-band-now").words.contains("No lanes moving"))
        assertTrue(empty.getValue("plan-band-next").words.contains("Nothing queued"))
    }

    @Test
    fun `with none running the card is gone and Now and Next takes the row`() {
        val withAgents = SemanticsProbe.tagged { Host(strip, busy) }
        val none = SemanticsProbe.tagged { Host(strip, AgentsFixtures.none()) }
        assertFalse("agents-card" in none)
        assertTrue(none.getValue("plan-strip").width > withAgents.getValue("plan-strip").width)
    }

    @Test
    fun `beside a wide layout's plan the card is alone and the row's width`() {
        val tags = SemanticsProbe.tagged { Host(null, busy, showsPlan = false) }
        assertFalse("plan-strip" in tags)
        assertTrue(tags.getValue("agents-card").width > tags.getValue("workspace-cards").width - 2 * 16 * 2.75f - 2)
        assertFalse("workspace-cards" in SemanticsProbe.tagged { Host(null, AgentsFixtures.none(), showsPlan = false) })
    }

    @Test
    @Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi", fontScale = 1.6f)
    fun `at a large font size the two cards stack, Agents under Now and Next, and Next moves to the plan sheet`() {
        val tags = SemanticsProbe.described { Host(strip, busy) }
        assertTrue("Now stays", "plan-band-now" in tags)
        assertFalse("Next is the sheet's: it is said once, there", "plan-band-next" in tags)
        assertTrue("and Now has the room Next had", tags.getValue("plan-band-now").bounds.height > 0f)
        val card = tags.getValue("agents-card")
        val plan = tags.getValue("plan-strip")
        assertTrue("under it: ${card.bounds} ${plan.bounds}", card.bounds.top >= plan.bounds.bottom)
        assertEquals("the row's width", plan.bounds.width, card.bounds.width, 2f)
        assertTrue("says its count in words: ${card.words}", card.words.startsWith("4 agents"))
    }

    @Test
    fun `the agents view lists the orchestrator's subagents under their heading and a row opens its agent`() {
        val chosen = ArrayList<RunningAgent>()
        var click: (() -> Unit)? = null
        val tags = SemanticsProbe.tagged(act = { activity ->
            click = onClickOf(activity, "agent-row-b")
        }) { FarCoolerTheme { Surface { AgentsList(busy, onChoose = { chosen += it }) } } }
        for (tag in listOf("agents-list", "agent-row-b", "agent-row-o-s1", "agent-row-o-s2", "agent-row-o-s3")) {
            assertTrue("$tag in ${tags.keys}", tag in tags)
        }
        assertNotNull(click)
        click!!()
        assertEquals(listOf("Build ov-619"), chosen.map { it.name })
        assertEquals("o", busy.agents.first().terminal)
        val none = SemanticsProbe.tagged { FarCoolerTheme { Surface { AgentsList(RunningAgents.EMPTY, onChoose = {}) } } }
        assertTrue("agents-none" in none && "agents-list" !in none)
    }

    /** What a tap on the node tagged [tag] runs: its semantics' click action, since input injection is gone on SDK 37. */
    private fun onClickOf(activity: ComponentActivity, tag: String): (() -> Unit)? {
        fun roots(view: View): List<RootForTest> =
            if (view is RootForTest) listOf(view) else if (view is ViewGroup) (0 until view.childCount).flatMap { roots(view.getChildAt(it)) } else emptyList()
        fun find(node: SemanticsNode): SemanticsNode? =
            if (node.config.getOrNull(SemanticsProperties.TestTag) == tag) node else node.children.firstNotNullOfOrNull(::find)
        val node = roots(activity.window.decorView).firstOrNull()?.semanticsOwner?.unmergedRootSemanticsNode?.let(::find) ?: return null
        val action = node.config.getOrNull(SemanticsActions.OnClick)?.action ?: return null
        return { action() }
    }

    // Captures.

    @Test
    @Config(sdk = [37], qualifiers = "w320dp-h640dp-xxhdpi")
    fun narrowest() = Capture.both("agents-cards-320") { Host(strip, busy) }

    @Test
    fun phone() = Capture.both("agents-cards-411") { Host(strip, busy) }

    @Test
    fun phoneNone() = Capture.both("agents-cards-411-none") { Host(quiet, RunningAgents.EMPTY) }

    @Test
    @Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi", fontScale = 1.6f)
    fun phoneLargeFont() = Capture.both("agents-cards-411-large-font") { Host(strip, busy) }

    @Test
    @Config(sdk = [37], qualifiers = "w1280dp-h800dp-xhdpi")
    fun tablet() = Capture.both("agents-cards-1280-chat") { Host(null, busy, showsPlan = false) }

    @Test
    fun agentsView() = Capture.both("agents-view-411") { SheetHost(busy) }

    @Test
    @Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi", fontScale = 1.6f)
    fun agentsViewLargeFont() = Capture.both("agents-view-411-large-font") { SheetHost(busy) }

    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}.")
    }

    // A turn that died is drawn red, at the needs-you weight, as the iPhone's and the Mac's are.

    private fun fleet(failed: Boolean) = com.farcooler.model.Fleet(
        runtimeHealthy = true, livePanes = 1,
        worktrees = listOf(
            com.farcooler.model.Worktree(
                id = "lane", workspace = "ws",
                terminals = listOf(Terminal(id = "b", title = "Build", state = "running", activity = "done", role = "agent", workspace = "ws", turnFailed = failed)),
            ),
        ),
    )

    /** How many pixels of [content]'s screen are the scheme's error ink, whatever the platform's dynamic color made of it. */
    private fun errorPixels(content: @androidx.compose.runtime.Composable () -> Unit): Int {
        var count = 0
        var error = 0
        androidx.test.core.app.ActivityScenario.launch(androidx.activity.ComponentActivity::class.java).use { scenario ->
            scenario.onActivity {
                it.setContent {
                    FarCoolerTheme {
                        error = MaterialTheme.colorScheme.error.toArgb()
                        Surface(Modifier.fillMaxSize()) { content() }
                    }
                }
            }
            org.robolectric.shadows.ShadowLooper.idleMainLooper()
            scenario.onActivity { activity ->
                val bitmap = activity.window.decorView.drawToBitmap()
                fun near(a: Int, b: Int) = kotlin.math.abs(a - b) <= 16
                for (x in 0 until bitmap.width) for (y in 0 until bitmap.height) {
                    val c = bitmap.getPixel(x, y)
                    if (near(android.graphics.Color.red(c), android.graphics.Color.red(error)) &&
                        near(android.graphics.Color.green(c), android.graphics.Color.green(error)) &&
                        near(android.graphics.Color.blue(c), android.graphics.Color.blue(error))
                    ) count++
                }
            }
        }
        return count
    }

    @Test
    fun `a failed pane's dot and row are drawn red, and a finished one's are not`() {
        val failed = RunningAgents.workspace("ws", "r", fleet(failed = true))
        val fine = RunningAgents.workspace("ws", "r", fleet(failed = false))
        assertTrue(failed.agents.single().failed)
        assertEquals("a finished turn is not red", 0, errorPixels { WorkspaceCards(null, fine, {}, {}, showsPlan = false) })
        assertTrue("the card's dot is red: ${errorPixels { WorkspaceCards(null, failed, {}, {}, showsPlan = false) }}", errorPixels { WorkspaceCards(null, failed, {}, {}, showsPlan = false) } > 0)
        assertEquals(0, errorPixels { AgentsList(fine, onChoose = {}) })
        assertTrue("the row's mark is red", errorPixels { AgentsList(failed, onChoose = {}) } > 0)
    }

    /** A pane whose turn died, among others at work. */
    private fun withFailed() = RunningAgents.workspace(
        "ws", "r",
        com.farcooler.model.Fleet(
            runtimeHealthy = true, livePanes = 3,
            worktrees = listOf(
                com.farcooler.model.Worktree(
                    id = "lane", workspace = "ws",
                    terminals = listOf(
                        Terminal(id = "x", title = "Fix ov-640", state = "running", activity = "done", role = "agent", workspace = "ws", turnFailed = true, line = "cargo test failed: 3 errors"),
                        Terminal(id = "y", title = "Build ov-619", state = "running", activity = "working", role = "agent", workspace = "ws", line = "Edit RunningAgents.swift", turnStartedAt = (System.currentTimeMillis() - 185_000).toDouble()),
                        Terminal(id = "z", title = "Review ov-616", state = "running", activity = "blocked", role = "agent", workspace = "ws", line = "Allow Bash cargo test?"),
                    ),
                ),
            ),
        ),
    )

    @Test
    fun failedCards() = Capture.both("agents-cards-411-failed") { Host(strip, withFailed()) }

    @Test
    fun failedView() = Capture.both("agents-view-411-failed") { SheetHost(withFailed()) }

    /** Long content: nine agents, more than the sheet has room for, so it scrolls under the title. */
    @Test
    fun longSheet() = Capture.both("agents-view-411-long") { SheetHost(AgentsFixtures.many()) }

    @Test
    @Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi", fontScale = 1.6f)
    fun longSheetLargeFont() = Capture.both("agents-view-411-long-large-font") { SheetHost(AgentsFixtures.many()) }

    /**
     * The sheet as the app opens it, drawn in the screen where its window can't be: over the scrim, from the bottom, with its
     * drag handle, its own 16 dp title and the list under it, as tall as the content or [0.9] of the screen.
     */
    @OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
    @Composable
    private fun SheetHost(agents: RunningAgents) = FarCoolerTheme {
        val screen = androidx.compose.ui.platform.LocalConfiguration.current.screenHeightDp.dp
        Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.scrim.copy(alpha = 0.32f))) {
            Surface(
                Modifier.align(Alignment.BottomCenter).fillMaxWidth().heightIn(max = screen * 0.9f).testTag("agents-sheet"),
                shape = androidx.compose.material3.BottomSheetDefaults.ExpandedShape,
                color = androidx.compose.material3.BottomSheetDefaults.ContainerColor,
            ) {
                Column {
                    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { androidx.compose.material3.BottomSheetDefaults.DragHandle() }
                    AgentsSheetContent(agents, onChoose = {})
                }
            }
        }
    }

    // The agents view's rows: the dot, the name, the time and the chevron on one line; the rule inset to the content edge.

    @Test
    fun `a row's name, time and chevron share one line, and every row is as tall as another`() {
        val dp = 3f // xxhdpi
        val tags = SemanticsProbe.described { FarCoolerTheme { Surface { AgentsList(AgentsFixtures.running(), onChoose = {}) } } }
        for (row in listOf("agent-row-b", "agent-row-o-s1")) {
            val name = tags.getValue("$row-name").bounds
            val go = tags.getValue("$row-go").bounds
            assertEquals("$row: the chevron is on the name's line", name.center.y, go.center.y, 2f)
            tags["$row-time"]?.let { assertEquals("$row: so is the time", name.center.y, it.bounds.center.y, 2f) }
            assertTrue("$row: the time has no gap in it: ${tags["$row-time"]?.words}", tags["$row-time"]?.words?.contains(" ") != true)
        }
        // The rule starts at the content edge (16 dp) and stops short of the screen's.
        val rule = tags.getValue("agent-row-b-rule").bounds
        assertEquals("starts at the content edge", 16f * dp, rule.left, 2f)
        assertTrue("and ends before the screen's edge: ${rule.right}", rule.right <= 411f * dp - 16f * dp + 2f)
        // With a time and without one, the row is as tall.
        val mixed = SemanticsProbe.described { FarCoolerTheme { Surface { AgentsList(withFailed(), onChoose = {}) } } }
        assertTrue("one has a time, one has none", "agent-row-y-time" in mixed && "agent-row-x-time" !in mixed)
        assertEquals("a row's height does not depend on its time", mixed.getValue("agent-row-y").bounds.height, mixed.getValue("agent-row-x").bounds.height, 2f)
    }
}
