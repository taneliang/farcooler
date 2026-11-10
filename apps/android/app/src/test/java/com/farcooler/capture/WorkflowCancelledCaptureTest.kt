package com.farcooler.capture

import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Surface
import androidx.compose.ui.unit.dp
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.WorkflowPresentations
import com.farcooler.ui.WorkflowRunCard
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** A cancelled run with a long reason at the narrowest phone width, from the shared core's own card (ov-1053). */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w320dp-h1200dp-xxhdpi")
class WorkflowCancelledCaptureTest {
    private fun card(name: String): String {
        val root = generateSequence(File("").absoluteFile) { it.parentFile }
            .first { File(it, "crates/client/src/testdata/workflow-presentation.json").exists() }
        val all = Json.parseToJsonElement(File(root, "crates/client/src/testdata/workflow-presentation.json").readText()) as JsonArray
        return all.first { it.jsonObject.getValue("name").jsonPrimitive.content == name }
            .jsonObject.getValue("response").jsonObject.getValue("card").toString()
    }

    private fun shoot(name: String, open: Boolean, file: String) {
        val run = WorkflowPresentations.decode(card(name), "workspace").runs.single()
        Capture.both(file) {
            FarCoolerTheme { Surface { androidx.compose.foundation.layout.Box(androidx.compose.ui.Modifier.padding(16.dp)) { WorkflowRunCard(run, enabled = true, detailsOpen = open) {} } } }
        }
    }

    @Test fun `a cancelled run says why, narrow`() = shoot("cancelled", false, "workflow-cancelled-narrow")
    /** A run that waits on the viewer: one sentence and its button, above the steps (ov-1060). */
    @Test fun `an approval is one block with its button, narrow`() = shoot("approval", false, "workflow-approval-narrow")
    @Test fun `a failed run with Details open, narrow`() = shoot("failed", true, "workflow-failed-details-narrow")
}
