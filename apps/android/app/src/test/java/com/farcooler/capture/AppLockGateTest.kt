package com.farcooler.capture

import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.material3.Text
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.test.core.app.ActivityScenario
import com.farcooler.ui.AppLockGate
import com.farcooler.ui.FarCoolerTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowLooper

/**
 * The app lock over the app (ov-856, §3a): locked at launch, opened by the device's own prompt, never in the way of a
 * phone with no screen lock or of someone who turned it off. The prompt and the device are given, so nothing here
 * needs a fingerprint. Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class AppLockGateTest {
    private fun roots(view: View): List<RootForTest> = when {
        view is RootForTest -> listOf(view)
        view is ViewGroup -> (0 until view.childCount).flatMap { roots(view.getChildAt(it)) }
        else -> emptyList()
    }

    private fun tagged(node: SemanticsNode, tag: String): SemanticsNode? =
        if (node.config.getOrNull(SemanticsProperties.TestTag) == tag) node else node.children.firstNotNullOfOrNull { tagged(it, tag) }

    private fun texts(node: SemanticsNode, out: MutableList<String>) {
        node.config.getOrNull(SemanticsProperties.Text)?.forEach { out.add(it.text) }
        node.children.forEach { texts(it, out) }
    }

    private fun withGate(
        on: Boolean,
        canAsk: Boolean,
        answers: List<Boolean>,
        check: (steps: Steps) -> Unit,
    ) {
        val asked = mutableListOf<Int>()
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity {
                it.setContent {
                    FarCoolerTheme {
                        AppLockGate(on = on, canAsk = { canAsk }, ask = { asked += 1; answers.getOrElse(asked.size - 1) { false } }) { Text("the runners") }
                    }
                }
            }
            ShadowLooper.idleMainLooper(500, java.util.concurrent.TimeUnit.MILLISECONDS)
            check(Steps(scenario, asked))
        }
    }

    private class Steps(val scenario: ActivityScenario<ComponentActivity>, val asked: List<Int>)

    private fun Steps.root(): SemanticsNode {
        var found: SemanticsNode? = null
        scenario.onActivity { found = roots(it.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode }
        return found!!
    }

    @Test
    fun `a launch is locked until the prompt is answered`() = withGate(on = true, canAsk = true, answers = listOf(false, true)) { s ->
        assertEquals("asked at launch", 1, s.asked.size)
        assertNotNull("covered", tagged(s.root(), "app-lock"))
        val said = mutableListOf<String>().also { texts(s.root(), it) }
        assertTrue("$said", "Your fingerprint, face or screen lock didn’t work. Try again." in said)
        tagged(s.root(), "app-lock-unlock")!!.config[SemanticsActions.OnClick].action!!.invoke()
        ShadowLooper.idleMainLooper(500, java.util.concurrent.TimeUnit.MILLISECONDS)
        assertEquals(2, s.asked.size)
        assertNull("open once the device says yes", tagged(s.root(), "app-lock"))
    }

    @Test
    fun `a phone with no screen lock is never locked out`() = withGate(on = true, canAsk = false, answers = emptyList()) { s ->
        assertNull(tagged(s.root(), "app-lock"))
        assertTrue("nothing to ask with, so nothing asked", s.asked.isEmpty())
    }

    @Test
    fun `turned off in Settings it never covers the app`() = withGate(on = false, canAsk = true, answers = emptyList()) { s ->
        assertNull(tagged(s.root(), "app-lock"))
        assertTrue(s.asked.isEmpty())
    }
}
