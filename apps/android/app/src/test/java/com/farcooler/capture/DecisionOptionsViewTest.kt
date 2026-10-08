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
import com.farcooler.model.NeedsYouAction
import com.farcooler.ui.DecisionOptions
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
 * A decision's options are a radio list with their whole text, and Answer
 * sends the chosen one (ov-431). The app's own `DecisionOptions`, pressed
 * through its semantics actions (what a tap and TalkBack call), and the
 * captures, light and dark. Run only under `-Pfarcooler.captures`, which is
 * JDK 21's. Other screens: `-Pfarcooler.captureQualifiers=w320dp-h640dp-xxhdpi`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class DecisionOptionsViewTest {
    private val options = listOf(
        NeedsYouAction("overlay", "Overlay on hover: the actions float over the row's trailing edge with a fade, so the text uses the full width"),
        NeedsYouAction("keep", "Keep the reserved width"),
        NeedsYouAction("dim", "Always show the actions, dimmed until the pointer is over the row"),
        NeedsYouAction("fourth", "A fourth option, which the old buttons put behind a More menu"),
    )

    private fun roots(view: View): List<RootForTest> = when {
        view is RootForTest -> listOf(view)
        view is ViewGroup -> (0 until view.childCount).flatMap { roots(view.getChildAt(it)) }
        else -> emptyList()
    }

    private fun nodes(activity: ComponentActivity): Map<String, SemanticsNode> {
        val out = LinkedHashMap<String, SemanticsNode>()
        fun walk(node: SemanticsNode) {
            node.config.getOrNull(SemanticsProperties.TestTag)?.let { out.putIfAbsent(it, node) }
            node.children.forEach(::walk)
        }
        walk(roots(activity.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode)
        return out
    }

    private fun drawn(sent: MutableList<String>, body: (tags: () -> Map<String, SemanticsNode>, press: (String) -> Unit) -> Unit) {
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity {
                it.setContent {
                    FarCoolerTheme { Column(Modifier.padding(16.dp)) { DecisionOptions(options, null, onAnswer = { a -> sent.add(a.id) }) } }
                }
            }
            val tags = {
                ShadowLooper.idleMainLooper()
                var seen = emptyMap<String, SemanticsNode>()
                scenario.onActivity { seen = nodes(it) }
                seen
            }
            val press = { tag: String ->
                val node = tags()[tag] ?: throw AssertionError("$tag isn't drawn")
                scenario.onActivity { node.config[SemanticsActions.OnClick].action!!.invoke() }
                ShadowLooper.idleMainLooper()
            }
            body(tags, press)
        }
    }

    @Test
    fun `choosing sends nothing and Answer sends the chosen option`() {
        val sent = mutableListOf<String>()
        drawn(sent) { tags, press ->
            assertTrue("every option is a row, none behind a menu: ${tags().keys}", options.all { "decision-option-${it.id}" in tags() })
            assertTrue("Answer is off before a choice", SemanticsProperties.Disabled in tags()["decision-answer"]!!.config)
            press("decision-option-keep")
            assertTrue("choosing sent nothing", sent.isEmpty())
            assertTrue("Answer is on after a choice", SemanticsProperties.Disabled !in tags()["decision-answer"]!!.config)
            press("decision-answer")
            assertEquals(listOf("keep"), sent)
        }
    }

    @Test
    fun `a long option takes the lines its text needs and is stacked`() {
        val rects = SemanticsProbe.tagged { FarCoolerTheme { Column(Modifier.padding(16.dp)) { DecisionOptions(options, null, onAnswer = {}) } } }
        val long = rects["decision-option-overlay"]!!
        val short = rects["decision-option-keep"]!!
        assertTrue("wraps: ${long.height} vs ${short.height}", long.height > short.height * 1.5f)
        assertTrue("stacked", short.top >= long.bottom - 1f)
    }

    @Test
    fun captures() {
        Capture.both("decision-options") { Column(Modifier.padding(16.dp)) { DecisionOptions(options, null, onAnswer = {}) } }
    }
}
