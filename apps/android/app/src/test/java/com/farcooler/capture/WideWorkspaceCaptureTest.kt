package com.farcooler.capture

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.Text
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.farcooler.data.Themes
import com.farcooler.model.OneTree
import com.farcooler.model.Plan
import com.farcooler.model.PlanReadState
import com.farcooler.model.PlanStrip
import com.farcooler.model.Role
import com.farcooler.model.Terminal
import com.farcooler.model.TranscriptRow
import com.farcooler.model.WorkspaceSummary
import com.farcooler.ui.AgentRowView
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.PlanStripPill
import com.farcooler.ui.SheetHeader
import com.farcooler.ui.TreeFilterChips
import com.farcooler.ui.TreeNavigation
import com.farcooler.ui.TreeRootList
import com.farcooler.ui.WideContent
import com.farcooler.ui.WideDestination
import com.farcooler.ui.WideWorkspaceFrame
import com.farcooler.ui.WorkspaceLayout
import com.farcooler.ui.WorkspaceTab
import com.farcooler.ui.WorkspaceTabs
import com.farcooler.ui.planItems
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import androidx.compose.ui.geometry.Rect
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The wide workspace (ov-347) on a tablet and the same screen on a phone,
 * drawn from a real board's plan (`test/fixtures/plan-seeded.json`, the CLI's
 * own output) with the app's own views: the tree's rows, the plan's items, the
 * transcript's rows and the wide frame itself. A connection needs the native
 * core, which Robolectric doesn't have, so the host here is the one thing
 * standing in for `WorkspaceScreen`: it makes the same choice
 * ([WorkspaceLayout.current]) and hands the same views to the same frame.
 *
 * Beside the pictures, the tests read what is where: each pane's tag, at 839
 * and 840 dp. Run only under `-Pfarcooler.captures` (JDK 21, as the other
 * captures).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37])
@OptIn(ExperimentalMaterial3Api::class)
class WideWorkspaceCaptureTest {
    private val plan = Plan.decode(Json.parseToJsonElement(repositoryFile("test/fixtures/plan-seeded.json")).jsonObject["plan"]!!.jsonObject)
    private val orchestrator = Terminal(id = "o", state = "running", activity = "working", role = "orchestrator", line = "Dispatching plan-phones to its lane")
    private val strip = PlanStrip.of(plan, 2, orchestrator)
    private val tree = OneTree.build(WorkspaceSummary(id = "ws", name = "Billing"), null, plan, emptyList(), emptyList(), OneTree.Filter.OPEN)
    private val nav = TreeNavigation({}, {}, { _, _ -> }, {}, {})
    private val rows = listOf(
        TranscriptRow(1, TranscriptRow.Kind.Message(Role.USER, "Where are the phone lanes?", null)),
        TranscriptRow(2, TranscriptRow.Kind.Message(Role.AGENT, "Two are in review and one is still building. I'm dispatching plan-phones next.", null)),
    )

    /** What `WorkspaceScreen` does: the frame past 840 dp, the phone's tab and strip below. */
    @Composable
    private fun Host(destination: WideDestination = WideDestination.PLAN) {
        if (WorkspaceLayout.current(implicit = false) == WorkspaceLayout.Kind.WIDE) {
            WideWorkspaceFrame(
                destination = destination,
                onSelect = {},
                onBack = {},
                onNeedsYou = {},
                topBar = { TopAppBar(title = { Text("Billing") }) },
            ) { pane ->
                when (pane) {
                    WideContent.TREE -> Column(Modifier.fillMaxSize()) {
                        TreeFilterChips(OneTree.Filter.OPEN) {}
                        TreeRootList(tree, OneTree.Filter.OPEN, failed = false, nav = nav, menu = null) {}
                    }
                    WideContent.PLAN -> LazyColumn(Modifier.fillMaxSize()) {
                        item { SheetHeader(strip) }
                        planItems(PlanReadState.Loaded(plan), emptyMap(), onOpen = {}, onRetry = {})
                    }
                    WideContent.BOARD -> Text("Board", Modifier.padding(16.dp))
                    WideContent.CHAT -> Chat()
                }
            }
        } else {
            Column(Modifier.fillMaxSize()) {
                WorkspaceTabs(WorkspaceTab.ORCHESTRATOR, implicit = false) {}
                PlanStripPill(strip) {}
                Chat()
            }
        }
    }

    @Composable
    private fun Chat() = Column(Modifier.fillMaxSize().padding(16.dp)) { rows.forEach { AgentRowView(it) } }

    /** The tagged nodes of what [content] draws, with where each is on screen. */
    private fun drawn(content: @Composable () -> Unit): Map<String, Rect> = SemanticsProbe.tagged { FarCoolerTheme { content() } }

    @Test
    @Config(qualifiers = "w1280dp-h800dp-xhdpi")
    fun `a tablet shows the rail, the tree, the plan and the chat`() {
        val tags = drawn { Host() }
        for (tag in listOf("wide-rail", "wide-pane-list", "wide-pane-main", "wide-pane-supporting", "plan-sheet-orchestrator")) {
            assertTrue("$tag is drawn: ${tags.keys}", tag in tags)
        }
        assertFalse("the strip is the phone's", "plan-strip" in tags)
        // Left to right: rail, tree, plan, chat, with the plan taking what the others leave.
        val order = listOf("wide-rail", "wide-pane-list", "wide-pane-main", "wide-pane-supporting").map { tags.getValue(it).left }
        assertEquals(order.sorted(), order)
        assertTrue(tags.getValue("wide-pane-main").width > tags.getValue("wide-pane-list").width)
        assertTrue(tags.getValue("wide-pane-main").width > tags.getValue("wide-pane-supporting").width)
    }

    @Test
    @Config(qualifiers = "w840dp-h700dp-xhdpi")
    fun `an unfolded foldable at 840 dp is wide`() {
        val tags = drawn { Host() }
        assertTrue("wide-pane-supporting" in tags)
        assertTrue("the plan keeps a column of its own", tags.getValue("wide-pane-main").width > 150)
    }

    @Test
    @Config(qualifiers = "w839dp-h700dp-xhdpi")
    fun `839 dp is the phone's layout`() {
        val tags = drawn { Host() }
        assertFalse("wide-workspace" in tags)
        assertTrue("plan-strip" in tags)
    }

    @Test
    @Config(qualifiers = "w411dp-h891dp-xxhdpi")
    fun `a phone shows the tabs and the strip`() {
        val tags = drawn { Host() }
        assertFalse("wide-workspace" in tags)
        assertTrue("workspace-tab-orchestrator" in tags)
        assertTrue("plan-strip" in tags)
    }

    @Test
    @Config(qualifiers = "w1280dp-h800dp-xhdpi")
    fun `the board keeps the tree and the chat`() {
        val tags = drawn { Host(WideDestination.BOARD) }
        assertTrue("wide-pane-list" in tags)
        assertTrue("wide-pane-supporting" in tags)
        assertFalse("the plan's header is the plan place's", "plan-sheet-orchestrator" in tags)
    }

    @Test
    @Config(qualifiers = "w1280dp-h800dp-xhdpi")
    fun tablet() = Capture.both("tablet-workspace") { Host() }

    @Test
    @Config(qualifiers = "w1280dp-h800dp-xhdpi")
    fun tabletBoard() = Capture.both("tablet-workspace-board") { Host(WideDestination.BOARD) }

    @Test
    @Config(qualifiers = "w411dp-h891dp-xxhdpi")
    fun phone() = Capture.both("tablet-workspace-on-phone") { Host() }

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
