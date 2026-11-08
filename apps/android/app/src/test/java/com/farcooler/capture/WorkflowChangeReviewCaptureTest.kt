package com.farcooler.capture

import androidx.compose.material3.Surface
import com.farcooler.model.WorkflowChangeReview
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.WorkflowChangeReviewBody
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

/** The Review screen at the narrowest phone width, with the long change the daemon's own producer wrote (ov-978). */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w320dp-h2600dp-xxhdpi")
class WorkflowChangeReviewCaptureTest {
    private fun real(): String {
        var dir: File? = File("").absoluteFile
        while (dir != null && !File(dir, "crates/client/src/testdata/workflow-change-question.txt").exists()) dir = dir.parentFile
        return File(requireNotNull(dir), "crates/client/src/testdata/workflow-change-question.txt").readText()
    }

    @Test fun `the whole change, narrow`() {
        val review = requireNotNull(WorkflowChangeReview.parse(real()))
        Capture.both("workflow-change-review-narrow") {
            FarCoolerTheme {
                Surface { WorkflowChangeReviewBody(review, sending = false, refusal = null, onApprove = {}, onDecline = {}, onDismiss = {}) }
            }
        }
    }
}
