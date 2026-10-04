package com.farcooler.capture

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.ui.Modifier
import com.farcooler.model.Plan
import com.farcooler.model.PlanPage
import com.farcooler.model.PlanReadState
import com.farcooler.model.PlanRecord
import com.farcooler.model.TaskStatus
import com.farcooler.ui.PlanPageBody
import com.farcooler.ui.PlanSwitch
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
 * The Plan view and its pages (ov-274), drawn from a real board's plan:
 * `test/fixtures/plan-seeded.json` is the CLI's own output for the board
 * `.claude/agent/reports/ov-273/seed.sh` seeds. The composables are the app's
 * own (`planItems`, `PlanPageBody`), with nothing built for the picture.
 * Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class PlanCaptureTest {
    private val seeded = Json.parseToJsonElement(repositoryFile("test/fixtures/plan-seeded.json")).jsonObject
    private val plan = Plan.decode(seeded["plan"]!!.jsonObject)
    private fun record(id: String) = PlanRecord.decode(seeded["records"]!!.jsonObject[id]!!.jsonObject)

    private val statuses = mapOf<String, TaskStatus>()

    @Test fun overview() = Capture.both("plan-overview") {
        LazyColumn(Modifier.fillMaxSize()) {
            item { PlanSwitch(showsPlan = true, onChange = {}) }
            planItems(PlanReadState.Loaded(plan), statuses, onOpen = {}, onRetry = {})
        }
    }

    @Test fun themes() = Capture.both("plan-themes") {
        LazyColumn(Modifier.fillMaxSize()) {
            planItems(PlanReadState.Loaded(plan.copy(order = emptyList(), lanes = emptyList())), statuses, onOpen = {}, onRetry = {})
        }
    }

    @Test fun themePage() = Capture.both("plan-theme-page") {
        val theme = plan.shownThemes.first()
        PlanPageBody(plan, PlanPage.Theme(theme.id), record(theme.id), rows = emptyMap(), onOpenTask = {}, onOpenPage = {})
    }

    @Test fun lanePage() = Capture.both("plan-lane-page") {
        val lane = plan.lanes.first { it.name == "mac-vis" }
        PlanPageBody(plan, PlanPage.Lane(lane.id), record(lane.id), rows = emptyMap(), onOpenTask = {}, onOpenPage = {})
    }

    @Test fun unavailable() = Capture.both("plan-unavailable") {
        LazyColumn(Modifier.fillMaxSize()) {
            item { PlanSwitch(showsPlan = true, onChange = {}) }
            planItems(PlanReadState.Unavailable, statuses, onOpen = {}, onRetry = {})
        }
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
