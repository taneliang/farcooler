package com.farcooler.capture

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.ui.Modifier
import com.farcooler.model.OneTree
import com.farcooler.model.Plan
import com.farcooler.model.PlanReadState
import com.farcooler.model.WorkspaceSummary
import com.farcooler.ui.PlanSwitch
import com.farcooler.ui.RulingsHook
import com.farcooler.ui.TreeNavigation
import com.farcooler.ui.TreeRow
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
 * The plan, its rulings and the tree with long real content (ov-412 sweep):
 * `test/fixtures/plan-rulings-long.json` is the rulings fixture's board with
 * its themes, lanes and rulings lengthened. Run on other screens with
 * `-Pfarcooler.captureQualifiers=w320dp-h640dp-xxhdpi` (narrowest) or
 * `w891dp-h411dp-xxhdpi` (widest, landscape). Run only under
 * `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class PolishLongCaptureTest {
    private val plan = Plan.decode(Json.parseToJsonElement(repositoryFile("test/fixtures/plan-rulings-long.json")).jsonObject["plan"]!!.jsonObject)
    private val tree = OneTree.build(WorkspaceSummary(id = "ws", name = "Billing"), null, plan, emptyList(), emptyList(), OneTree.Filter.OPEN)
    private val nav = TreeNavigation({}, {}, { _, _ -> }, {}, {})

    @Test fun longPlan() = Capture.both("long-plan") {
        LazyColumn(Modifier.fillMaxSize()) {
            item { PlanSwitch(showsPlan = true, onChange = {}) }
            planItems(PlanReadState.Loaded(plan), emptyMap(), onOpen = {}, onRetry = {}, rulings = RulingsHook(copy = {}, canMark = true, canAsk = true, pastOpen = true))
        }
    }

    @Test fun longTree() = Capture.both("long-tree") {
        LazyColumn(Modifier.fillMaxSize()) { items(tree.work + tree.below, key = { it.id }) { TreeRow(it, nav) } }
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
