package com.farcooler.capture

import androidx.compose.foundation.layout.Column
import androidx.compose.ui.Alignment
import androidx.compose.material3.Surface
import androidx.compose.material3.MaterialTheme
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.HorizontalDivider
import androidx.compose.ui.Modifier
import com.farcooler.model.LaneTree
import com.farcooler.model.OneTree
import com.farcooler.model.Plan
import com.farcooler.model.PlanReadState
import com.farcooler.model.PlanStrip
import com.farcooler.model.Terminal
import com.farcooler.model.ThemeTree
import com.farcooler.model.WorkspaceSummary
import com.farcooler.ui.WorkspaceCards
import com.farcooler.ui.SheetHeader
import com.farcooler.ui.TreeNavigation
import com.farcooler.ui.TreeRow
import com.farcooler.ui.SidebarActions
import com.farcooler.ui.SidebarModels
import com.farcooler.ui.TreeRoot
import com.farcooler.ui.WorkspaceTab
import com.farcooler.ui.WorkspaceTabs
import com.farcooler.ui.planItems
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The plan strip, the sheet it opens and the One tree on Android (ov-300),
 * drawn from a real board's plan (`test/fixtures/plan-seeded.json`, the CLI's
 * own output). The composables are the app's own (`PlanStripPill`,
 * `SheetHeader`, `planItems`, `TreeRow`) and the tree is `OneTree.build`'s.
 * Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class PhoneTreeCaptureTest {
    private val plan = Plan.decode(Json.parseToJsonElement(repositoryFile("test/fixtures/plan-seeded.json")).jsonObject["plan"]!!.jsonObject)
    private val strip = PlanStrip.of(plan, 2, Terminal(id = "o", state = "running", activity = "working", role = "orchestrator", line = "Dispatching plan-phones to its lane"))
    private val tree = OneTree.build(WorkspaceSummary(id = "ws", name = "Billing"), null, plan, emptyList(), emptyList(), OneTree.Filter.OPEN)
    private val nav = TreeNavigation({}, {}, { _, _ -> }, {}, {})
    private val models = SidebarModels(ThemeTree.of(plan), LaneTree.of(plan))
    private val actions = SidebarActions({}, {}, {}, { false })

    @Test fun orchestratorTab() = Capture.both("phone-orchestrator-tab") {
        // In place: the tab row, then the strip over where the pane goes.
        Column(Modifier.fillMaxSize()) {
            WorkspaceTabs(WorkspaceTab.ORCHESTRATOR, implicit = false) {}
            WorkspaceCards(strip, AgentsFixtures.running(), onOpenPlan = {}, onOpenAgents = {})
        }
    }

    @Test fun themesTab() = Capture.both("phone-themes-tab") {
        Column(Modifier.fillMaxSize()) {
            WorkspaceTabs(WorkspaceTab.WORKTREES, implicit = false) {}
            TreeRoot(tree, models, OneTree.Filter.OPEN, onFilter = {}, failed = false, stacked = false, nav = nav, actions = actions, menu = null) {}
        }
    }

    /**
     * The plan peek as the app opens it: the plan's sheet over the scrim, from the bottom, the cards behind it. The card
     * behind says Now and Next, so the sheet does not repeat the plan's band (ov-659), and the card's chevron points down.
     */
    @OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
    private fun peekScreen(sheetHeight: Float = 0.6f, covers: Boolean = false): @androidx.compose.runtime.Composable () -> Unit = {
        val band = com.farcooler.ui.peekBand(
            androidx.compose.ui.platform.LocalDensity.current.fontScale,
            androidx.compose.ui.platform.LocalConfiguration.current.screenWidthDp, covers,
        )
        Box(Modifier.fillMaxSize()) {
            Column(Modifier.fillMaxSize()) {
                WorkspaceTabs(WorkspaceTab.ORCHESTRATOR, implicit = false) {}
                WorkspaceCards(strip, AgentsFixtures.running(), onOpenPlan = {}, onOpenAgents = {}, planOpen = true)
            }
            Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.scrim.copy(alpha = 0.32f)))
            Surface(
                Modifier.align(Alignment.BottomCenter).fillMaxWidth().fillMaxHeight(sheetHeight),
                shape = androidx.compose.material3.BottomSheetDefaults.ExpandedShape,
                color = androidx.compose.material3.BottomSheetDefaults.ContainerColor,
            ) {
                Column {
                    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { androidx.compose.material3.BottomSheetDefaults.DragHandle() }
                    LazyColumn(Modifier.fillMaxSize()) {
                        item { SheetHeader(strip, showNeedsYou = false) }
                        planItems(PlanReadState.Loaded(plan), emptyMap(), onOpen = {}, onRetry = {}, needsYou = strip.needsYou, band = band)
                    }
                }
            }
        }
    }

    @OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
    @Test fun peek() = Capture.both("phone-plan-peek") { peekScreen()() }

    @OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
    @Test @Config(qualifiers = "w320dp-h700dp-xxhdpi")
    fun peekNarrow() = Capture.both("phone-plan-peek-320") { peekScreen()() }

    /** The sheet pulled up over the card: it says Now and Next itself. */
    @OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
    @Test @Config(qualifiers = "w320dp-h700dp-xxhdpi")
    fun peekExpandedNarrow() = Capture.both("phone-plan-peek-expanded-320") { peekScreen(sheetHeight = 0.92f, covers = true)() }

    /** Large type: the card keeps Now on two lines and leaves Next to the sheet. */
    @OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
    @Test @Config(qualifiers = "w411dp-h891dp-xxhdpi", fontScale = 1.6f)
    fun peekLargeFont() = Capture.both("phone-plan-peek-large-font") { peekScreen()() }

    @OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
    @Test @Config(qualifiers = "w600dp-h891dp-xxhdpi")
    fun peekWide() = Capture.both("phone-plan-peek-600") { peekScreen()() }

    @Test fun treeRoot() = Capture.both("phone-tree-root") {
        LazyColumn(Modifier.fillMaxSize()) { items(tree.work + tree.below, key = { it.id }) { TreeRow(it, nav) } }
    }

    @Test fun treeTheme() = Capture.both("phone-tree-theme") {
        LazyColumn(Modifier.fillMaxSize()) { items(tree.work.first().children, key = { it.id }) { TreeRow(it, nav) } }
    }

    @Test fun treeLane() = Capture.both("phone-tree-lane") {
        val lane = tree.all.first { it.kind == OneTree.Kind.LANE && it.title == "mac-vis" }
        LazyColumn(Modifier.fillMaxSize()) { items(lane.children, key = { it.id }) { TreeRow(it, nav) } }
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
