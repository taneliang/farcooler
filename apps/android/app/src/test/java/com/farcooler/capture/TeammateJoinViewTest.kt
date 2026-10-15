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
import com.farcooler.ceremony.CeremonyRunner
import com.farcooler.data.Reach
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.TeammateJoinPrompt
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowLooper

/**
 * The prompt a teammate's grant asks before this phone joins (ov-543): the
 * app's own `TeammateJoinPrompt`, its words, Cancel pressed through its
 * semantics action adding nothing, and the pictures with a long email and long
 * runner names. Run only under `-Pfarcooler.captures`, which is JDK 21's.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class TeammateJoinViewTest {
    private val email = "alexandra.montgomery-fitzgerald@platform-infrastructure.example-corporation.com"

    private fun runner(label: String) = CeremonyRunner(
        id = "alice@$label", label = label, alias = label, user = "alice", hostKey = "",
        reach = Reach.Direct("$label.example.com", 22), pending = false,
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

    private fun tagged(node: SemanticsNode, tag: String): SemanticsNode? =
        if (node.config.getOrNull(SemanticsProperties.TestTag) == tag) node
        else node.children.firstNotNullOfOrNull { tagged(it, tag) }

    @Test
    fun `the prompt asks, and Cancel adds nothing`() {
        var joined = 0
        var cancelled = 0
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity {
                it.setContent {
                    FarCoolerTheme {
                        TeammateJoinPrompt("alice@example.com", listOf(runner("build-box")), { joined += 1 }, { cancelled += 1 })
                    }
                }
            }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { activity ->
                val root = roots(activity.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode
                val said = mutableListOf<String>().also { texts(root, it) }
                assertTrue(said.toString(), "Join your teammate’s runner “build-box”?" in said)
                assertTrue(said.toString(), said.any { it.startsWith("This code says alice@example.com added") })
                assertEquals("nothing joined before a press", 0, joined)
                tagged(root, "join-teammate-cancel")!!.config[SemanticsActions.OnClick].action!!.invoke()
            }
            ShadowLooper.idleMainLooper()
            assertEquals(1, cancelled)
            assertEquals("Cancel joined", 0, joined)
        }
    }

    @Test fun oneRunnerLong() = Capture.both("teammate-join-one-long") {
        TeammateJoinPrompt(email, listOf(runner("Platform Builds, EU West (Shared) and Staging Runner 7")), {}, {})
    }

    @Test fun oneRunner() = Capture.both("teammate-join-one") {
        TeammateJoinPrompt("alice@example.com", listOf(runner("build-box")), {}, {})
    }

    @Test fun manyRunnersLong() = Capture.both("teammate-join-many-long") {
        TeammateJoinPrompt(
            email,
            listOf(
                runner("Platform Builds, EU West (Shared) and Staging Runner 7"),
                runner("build-box"),
                runner("gpu-workstation-with-an-unreasonably-long-name-for-a-machine"),
            ),
            {}, {},
        )
    }
}
