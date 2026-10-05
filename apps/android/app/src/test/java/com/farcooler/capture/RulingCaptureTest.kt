package com.farcooler.capture

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.ui.Modifier
import com.farcooler.model.Plan
import com.farcooler.model.PlanReadState
import com.farcooler.ui.PlanSwitch
import com.farcooler.ui.RulingsHook
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
 * Decided for you (ov-304), drawn from a real board's plan:
 * `test/fixtures/plan-rulings-seeded.json` is the CLI's `plan --json` for a
 * scratch board given four rulings with `plan ruling add` and `set`. The rows
 * are the app's own (`planItems` with its rulings hook). Run only under
 * `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class RulingCaptureTest {
    private val seeded = Json.parseToJsonElement(repositoryFile("test/fixtures/plan-rulings-seeded.json")).jsonObject
    private val plan = Plan.decode(seeded["plan"]!!.jsonObject)

    @Test fun rulings() = Capture.both("plan-rulings") {
        LazyColumn(Modifier.fillMaxSize()) {
            item { PlanSwitch(showsPlan = true, onChange = {}) }
            planItems(PlanReadState.Loaded(plan), emptyMap(), onOpen = {}, onRetry = {}, rulings = RulingsHook(copy = {}, canMark = true, canAsk = true, pastOpen = true))
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
