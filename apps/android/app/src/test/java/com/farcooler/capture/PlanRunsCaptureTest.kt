package com.farcooler.capture

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.ui.Modifier
import com.farcooler.model.Plan
import com.farcooler.model.PlanReadState
import com.farcooler.model.TaskStatus
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
 * The Plan's cards with the engine's run on three tickets (ov-1040), at the narrowest width and with long titles:
 * `plan-runs.json` carries each run in `plan.get` itself; `plan-runs-before.json` is the same plan without them.
 * Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w320dp-h3600dp-xxhdpi")
class PlanRunsCaptureTest {
    private fun plan(file: String) =
        Plan.decode(Json.parseToJsonElement(repositoryFile("test/fixtures/$file")).jsonObject["plan"]!!.jsonObject)

    private fun screen(file: String) = Capture.both(file.removeSuffix(".json") + "-320") {
        LazyColumn(Modifier.fillMaxSize()) {
            item { PlanSwitch(showsPlan = true, onChange = {}) }
            planItems(PlanReadState.Loaded(plan(file)), mapOf<String, TaskStatus>(), onOpen = {}, onRetry = {})
        }
    }

    @Test fun withRuns() = screen("plan-runs.json")

    @Test fun withoutRuns() = screen("plan-runs-before.json")

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
