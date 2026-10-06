package com.farcooler.capture

import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.test.core.app.ActivityScenario
import org.robolectric.shadows.ShadowLooper

/**
 * Reads what a composable drew: each test tag and the rectangle it took, in
 * pixels. Compose's own test rule can't run here, because its idling goes
 * through Espresso, whose input injection is gone on SDK 37 (see [Capture]),
 * so this launches the activity as [Capture] does and walks the semantics
 * tree of the compose view directly. Nothing is clicked or typed.
 */
object SemanticsProbe {
    fun tagged(content: @androidx.compose.runtime.Composable () -> Unit): Map<String, Rect> {
        var found = emptyMap<String, Rect>()
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { content() } }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { activity ->
                val root = roots(activity.window.decorView).first()
                val out = LinkedHashMap<String, Rect>()
                walk(root.semanticsOwner.unmergedRootSemanticsNode, out)
                found = out
            }
        }
        return found
    }

    private fun roots(view: View): List<RootForTest> {
        if (view is RootForTest) return listOf(view)
        if (view !is ViewGroup) return emptyList()
        return (0 until view.childCount).flatMap { roots(view.getChildAt(it)) }
    }

    private fun walk(node: SemanticsNode, out: MutableMap<String, Rect>) {
        node.config.getOrNull(SemanticsProperties.TestTag)?.let { out[it] = node.boundsInRoot }
        node.children.forEach { walk(it, out) }
    }
}
