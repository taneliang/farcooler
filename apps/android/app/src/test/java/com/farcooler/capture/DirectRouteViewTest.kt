package com.farcooler.capture

import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.ui.Modifier
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ActivityScenario
import com.farcooler.model.DirectRoute
import com.farcooler.ui.DirectRouteQuestion
import com.farcooler.ui.FarCoolerTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowLooper

/**
 * The owner's question when Tailcat fails (ov-485): the app's own
 * `DirectRouteQuestion`, its words, and Connect directly pressed through its
 * semantics action. Run only under `-Pfarcooler.captures`, which is JDK 21's.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class DirectRouteViewTest {
    private val offer = DirectRoute.Offer("192.168.1.180")

    private fun roots(view: View): List<RootForTest> = when {
        view is RootForTest -> listOf(view)
        view is ViewGroup -> (0 until view.childCount).flatMap { roots(view.getChildAt(it)) }
        else -> emptyList()
    }

    private fun texts(node: SemanticsNode, out: MutableList<String>) {
        node.config.getOrNull(SemanticsProperties.Text)?.forEach { out.add(it.text) }
        node.children.forEach { texts(it, out) }
    }

    private fun tagged(node: SemanticsNode, tag: String): SemanticsNode? =
        if (node.config.getOrNull(SemanticsProperties.TestTag) == tag) node
        else node.children.firstNotNullOfOrNull { tagged(it, tag) }

    @Test
    fun `the question is the owner's and only a press moves the runner`() {
        var moved = 0
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity {
                it.setContent { FarCoolerTheme { Column(Modifier.padding(16.dp)) { DirectRouteQuestion(offer) { moved += 1 } } } }
            }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { activity ->
                val root = roots(activity.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode
                val said = mutableListOf<String>().also { texts(root, it) }
                assertTrue(said.toString(), "Connecting through Tailcat failed." in said)
                assertTrue(said.toString(), "Connect directly to 192.168.1.180 instead?" in said)
                assertEquals("nothing moved before the press", 0, moved)
                tagged(root, "connect-directly")!!.config[SemanticsActions.OnClick].action!!.invoke()
            }
            ShadowLooper.idleMainLooper()
            assertEquals(1, moved)
        }
    }

    @Test
    fun captures() {
        Capture.both("android-failure-prompt") {
            Column(Modifier.padding(16.dp)) { DirectRouteQuestion(offer) {} }
        }
    }
}
