package com.farcooler.capture

import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.width
import androidx.compose.ui.Modifier
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ActivityScenario
import com.farcooler.ceremony.CodeImage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowLooper

/**
 * The add-a-device QR fills its square (ov-849): laid out at a fixed width, the
 * image node is as tall as it is wide, not the bitmap's few-pixel height. Run
 * only under `-Pfarcooler.captures`, which is JDK 21's.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class CodeImageViewTest {
    private fun roots(view: View): List<RootForTest> = when {
        view is RootForTest -> listOf(view)
        view is ViewGroup -> (0 until view.childCount).flatMap { roots(view.getChildAt(it)) }
        else -> emptyList()
    }

    private fun image(node: SemanticsNode): SemanticsNode? =
        if (node.config.getOrNull(SemanticsProperties.ContentDescription)
                ?.contains("A code for another device to scan") == true
        ) node
        else node.children.firstNotNullOfOrNull { image(it) }

    @Test
    fun `the code image is as tall as it is wide`() {
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity {
                it.setContent { CodeImage("https://example.com/join?code=abc123", Modifier.width(300.dp)) }
            }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { activity ->
                val root = roots(activity.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode
                val node = image(root)
                assertNotNull("no image node in the tree", node)
                val size = node!!.size
                assertEquals("the image's height, against its width ${size.width}", size.width.toDouble(), size.height.toDouble(), size.width * 0.05)
            }
        }
    }
}
