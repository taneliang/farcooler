package com.farcooler.capture

import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.test.core.app.ActivityScenario
import com.farcooler.ui.AddTeammateRow
import com.farcooler.ui.FarCoolerTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowLooper

/** Add Teammate in People (ov-865): the app's own row, pressed through semantics, alone on a runner or not. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class AddTeammateRowTest {
    private fun roots(view: View): List<RootForTest> = when {
        view is RootForTest -> listOf(view)
        view is ViewGroup -> (0 until view.childCount).flatMap { roots(view.getChildAt(it)) }
        else -> emptyList()
    }

    private fun tagged(node: SemanticsNode, tag: String): SemanticsNode? =
        if (node.config.getOrNull(SemanticsProperties.TestTag) == tag) node
        else node.children.firstNotNullOfOrNull { tagged(it, tag) }

    private fun shown(alone: Boolean, add: () -> Unit, then: (SemanticsNode) -> Unit) {
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { FarCoolerTheme { AddTeammateRow(alone, add) } } }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { then(roots(it.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode) }
            ShadowLooper.idleMainLooper()
        }
    }

    @Test
    fun `an owner alone on a runner finds People and presses Add teammate`() {
        var pressed = 0
        shown(alone = true, add = { pressed += 1 }) { root ->
            assertNotNull("People says it is just the owner", tagged(root, "runner-people-alone"))
            tagged(root, "people-add-teammate")!!.config[SemanticsActions.OnClick].action!!.invoke()
        }
        assertEquals(1, pressed)
    }

    @Test
    fun `a shared runner's People offers Add teammate without the alone line`() {
        var pressed = 0
        shown(alone = false, add = { pressed += 1 }) { root ->
            assertNull(tagged(root, "runner-people-alone"))
            tagged(root, "people-add-teammate")!!.config[SemanticsActions.OnClick].action!!.invoke()
        }
        assertEquals(1, pressed)
    }
}
