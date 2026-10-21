package com.farcooler.capture

import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.ui.Modifier
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ActivityScenario
import com.farcooler.model.HostHealth
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.HealthSection
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowLooper

/**
 * The owner's warning that their shell's startup files export a Claude key
 * (ov-651), drawn in Runner Settings' Health section with the long
 * three-name sentence. Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class HealthWarningViewTest {
    private val sentence =
        "Your shell’s startup files set ANTHROPIC_API_KEY, CLAUDE_CODE_OAUTH_TOKEN and ANTHROPIC_AUTH_TOKEN, " +
            "so a member typing claude in their own shell would use your account."

    private fun health(warning: String?) = HostHealth(
        platform = "darwin-aarch64", daemonVersion = "0.9.0+abc1234", protocolVersion = 1, healthy = true,
        livePanes = 3, claudeKeyWarning = warning,
    )

    private fun roots(view: View): List<RootForTest> = when {
        view is RootForTest -> listOf(view)
        view is ViewGroup -> (0 until view.childCount).flatMap { roots(view.getChildAt(it)) }
        else -> emptyList()
    }

    private fun texts(node: SemanticsNode, out: MutableList<String>) {
        node.config.getOrNull(SemanticsProperties.Text)?.forEach { out.add(it.text) }
        node.children.forEach { texts(it, out) }
    }

    private fun said(h: HostHealth): List<String> {
        val out = mutableListOf<String>()
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { FarCoolerTheme { Column(Modifier.padding(16.dp)) { HealthSection(h) } } } }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { activity ->
                texts(roots(activity.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode, out)
            }
        }
        return out
    }

    @Test
    fun `the warning is drawn when the runner sends one`() {
        val out = said(health(sentence))
        assertTrue(out.toString(), sentence in out)
    }

    @Test
    fun `no warning is drawn when the runner sends none`() {
        val out = said(health(null))
        assertFalse(out.toString(), out.any { it.contains("startup files") })
    }

    @Test
    fun healthWithTheWarning() = Capture.both("health-claude-key-warning") {
        Column(Modifier.padding(16.dp)) { HealthSection(health(sentence)) }
    }
}
