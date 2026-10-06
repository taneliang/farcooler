package com.farcooler.capture

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.HorizontalDivider
import androidx.compose.ui.Modifier
import com.farcooler.model.OneTree
import com.farcooler.model.Plan
import com.farcooler.model.PlanReadState
import com.farcooler.model.PlanStrip
import com.farcooler.model.Terminal
import com.farcooler.model.WorkspaceSummary
import com.farcooler.ui.PlanStripPill
import com.farcooler.ui.SheetHeader
import com.farcooler.ui.TreeNavigation
import com.farcooler.ui.TreeRow
import com.farcooler.ui.TreeFilterChips
import com.farcooler.ui.TreeRootList
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

    @Test fun orchestratorTab() = Capture.both("phone-orchestrator-tab") {
        // In place: the tab row, then the strip over where the pane goes.
        Column(Modifier.fillMaxSize()) {
            WorkspaceTabs(WorkspaceTab.ORCHESTRATOR, implicit = false) {}
            PlanStripPill(strip) {}
        }
    }

    @Test fun themesTab() = Capture.both("phone-themes-tab") {
        Column(Modifier.fillMaxSize()) {
            WorkspaceTabs(WorkspaceTab.WORKTREES, implicit = false) {}
            TreeFilterChips(OneTree.Filter.OPEN) {}
            TreeRootList(tree, OneTree.Filter.OPEN, failed = false, nav = nav, menu = null) {}
        }
    }

    @Test fun peek() = Capture.both("phone-plan-peek") {
        Column(Modifier.fillMaxSize()) {
            PlanStripPill(strip) {}
            HorizontalDivider()
            LazyColumn(Modifier.fillMaxSize()) {
                item { SheetHeader(strip) }
                planItems(PlanReadState.Loaded(plan), emptyMap(), onOpen = {}, onRetry = {})
            }
        }
    }

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
