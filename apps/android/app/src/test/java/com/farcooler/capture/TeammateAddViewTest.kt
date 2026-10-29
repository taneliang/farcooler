package com.farcooler.capture

import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.test.core.app.ActivityScenario
import com.farcooler.ceremony.CeremonyTeammate
import com.farcooler.ceremony.RunnerRow
import com.farcooler.ceremony.TeammateChoice
import com.farcooler.data.Reach
import com.farcooler.data.Runner
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.TeammateConfirmScreen
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
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
 * The confirmation for a teammate's device (ov-854): the app's own
 * `TeammateConfirmScreen`, its words, Add Teammate off until a runner is
 * ticked, a runner the phone only controls listed with why and not tickable,
 * and the pictures with a long email and long names. Run only under
 * `-Pfarcooler.captures`, which is JDK 21's.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class TeammateAddViewTest {
    private val buildBox = Runner(id = "r1", label = "build-box", reach = Reach.Direct("build-box.example.com", 22), user = "alice")
    private val gpu = Runner(id = "r2", label = "gpu-workstation", reach = Reach.Direct("gpu.example.com", 22), user = "alice")
    private val fingerprint = "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4"

    private fun teammate(email: String = "bob.smith@example.com") = TeammateChoice(
        who = CeremonyTeammate("user_bob", email, "Platform"), directory = "{}", grantedBy = "alice@example.com",
    )

    private fun rows(vararg runners: Runner) = runners.map {
        RunnerRow(
            it, picked = false,
            blocked = if (it.id == gpu.id) "Only an Owner or an Admin can add people to ${it.label}." else null,
        )
    }

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

    private fun enabled(node: SemanticsNode): Boolean = node.config.getOrNull(SemanticsProperties.Disabled) == null

    /** A host that keeps the screen's state, the way the store does. */
    @androidx.compose.runtime.Composable
    private fun Host(
        start: TeammateChoice,
        initialRows: List<RunnerRow>,
        confirmed: () -> Unit = {},
        seen: (TeammateChoice) -> Unit = {},
    ) {
        var choice by remember { mutableStateOf(start) }
        var shown by remember { mutableStateOf(initialRows) }
        seen(choice)
        TeammateConfirmScreen(
            deviceName = "Bob’s Pixel",
            teammate = choice,
            fingerprint = fingerprint,
            publicKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 bob",
            rows = shown,
            onToggle = { id -> shown = shown.map { if (it.runner.id == id && it.blocked == null) it.copy(picked = !it.picked) else it } },
            onChange = { change -> choice = change(choice) },
            onConfirm = confirmed,
            onCancel = {},
        )
    }

    @Test
    fun `the confirmation names the directory's person, ticks nothing, and writes only on Add`() {
        var confirmed = 0
        var current: TeammateChoice? = null
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity {
                it.setContent {
                    FarCoolerTheme { Host(teammate(), rows(buildBox, gpu), { confirmed += 1 }, { c -> current = c }) }
                }
            }
            ShadowLooper.idleMainLooper()
            fun root(): SemanticsNode {
                var node: SemanticsNode? = null
                scenario.onActivity { node = roots(it.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode }
                return node!!
            }
            val said = mutableListOf<String>().also { texts(root(), it) }
            assertTrue(said.toString(), "Add your teammate’s device?" in said)
            assertTrue(said.toString(), said.any { it.contains("bob.smith@example.com, who’s on your team “Platform”") })
            assertTrue(said.toString(), "Add teammate" in said)
            assertEquals("a suggested name", "Bob Smith", current!!.name)
            assertFalse("Add teammate on with no runner ticked", enabled(tagged(root(), "teammate-add")!!))

            // A runner this phone only controls says why and can't be ticked.
            assertTrue(said.toString(), said.any { it.startsWith("Only an Owner or an Admin can add people to gpu-workstation") })
            tagged(root(), "teammate-runner-gpu-workstation")!!.config.getOrNull(SemanticsActions.OnClick)
                ?.action?.invoke()
            ShadowLooper.idleMainLooper()
            assertFalse("a blocked runner was ticked", enabled(tagged(root(), "teammate-add")!!))

            tagged(root(), "teammate-runner-build-box")!!.config[SemanticsActions.OnClick].action!!.invoke()
            ShadowLooper.idleMainLooper()
            assertTrue(enabled(tagged(root(), "teammate-add")!!))
            assertEquals("nothing is written before a press", 0, confirmed)

            // A read line is what is sent when Viewer is chosen.
            tagged(root(), "teammate-access-viewer")!!.config[SemanticsActions.OnClick].action!!.invoke()
            ShadowLooper.idleMainLooper()
            assertEquals(com.farcooler.model.PersonAccess.VIEWER, current!!.access)
            assertEquals(com.farcooler.ceremony.TeammateScope.READ, current!!.scope)
            val after = mutableListOf<String>().also { texts(root(), it) }
            assertTrue(after.toString(), "Can see the board and progress, but not terminals." in after)

            tagged(root(), "teammate-add")!!.config[SemanticsActions.OnClick].action!!.invoke()
        }
        assertEquals(1, confirmed)
    }

    @Test
    fun `a handle a runner would refuse keeps Add off and says why`() {
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity {
                it.setContent {
                    FarCoolerTheme {
                        Host(teammate().copy(handle = "Bob Smith"), rows(buildBox).map { r -> r.copy(picked = true) })
                    }
                }
            }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { activity ->
                val root = roots(activity.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode
                assertFalse(enabled(tagged(root, "teammate-add")!!))
                assertNotNull(tagged(root, "teammate-handle-note"))
            }
        }
    }

    @Test
    fun `a valid handle says nothing`() {
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { FarCoolerTheme { Host(teammate(), rows(buildBox)) } } }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { activity ->
                val root = roots(activity.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode
                assertNull(tagged(root, "teammate-handle-note"))
            }
        }
    }

    @Test fun confirmation() = Capture.both("teammate-add") { Host(teammate(), rows(buildBox, gpu)) }

    @Test fun confirmationLong() = Capture.both("teammate-add-long") {
        Host(
            teammate("bartholomew.montgomery-fitzgerald@platform-infrastructure.example-corporation.com"),
            rows(
                Runner(id = "r3", label = "Platform Builds, EU West (Shared) and Staging Runner 7", reach = Reach.Direct("eu.example.com", 22), user = "alice"),
                gpu,
            ),
        )
    }
}
