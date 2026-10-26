package com.farcooler.capture

import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.Modifier
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ActivityScenario
import com.farcooler.model.RunnerTeam
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.RemovedRow
import com.farcooler.ui.RunnerPeopleSection
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
 * People with roles, and the row a removed member's phone shows (ov-784): the app's own composables, their words,
 * the picker pressed through semantics, and the pictures with long names. Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class RunnerPeopleViewTest {
    private val longRunner = "Platform Builds, EU West (Shared) and Staging Runner 7"
    private val team = RunnerTeam(
        listOf(
            RunnerTeam.Member("alexandra-montgomery", "Alexandra Montgomery-Fitzgerald of the Platform Team", RunnerTeam.Role.MAINTAINER, 3),
            RunnerTeam.Member("bob", "Bob", RunnerTeam.Role.MEMBER, 0),
        ),
        you = "",
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

    private fun shown(content: @androidx.compose.runtime.Composable () -> Unit, then: (SemanticsNode, List<String>) -> Unit) {
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { FarCoolerTheme { content() } } }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { activity ->
                val root = roots(activity.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode
                then(root, mutableListOf<String>().also { texts(root, it) })
            }
            ShadowLooper.idleMainLooper()
        }
    }

    /** Every window's compose root, the menu's popup among them: a popup is a window of its own. */
    private fun everyRoot(): List<SemanticsNode> {
        val global = Class.forName("android.view.WindowManagerGlobal")
        val instance = global.getMethod("getInstance").invoke(null)
        val views = global.getDeclaredField("mViews").also { it.isAccessible = true }.get(instance) as List<*>
        return views.filterIsInstance<View>().flatMap { roots(it) }.map { it.semanticsOwner.unmergedRootSemanticsNode }
    }

    @Test
    fun `the owner picks a role from the menu, and the runner is asked by handle`() {
        val asked = mutableListOf<Pair<String, RunnerTeam.Role>>()
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { FarCoolerTheme { RunnerPeopleSection(team, null) { handle, role -> asked += handle to role } } } }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { activity ->
                val root = roots(activity.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode
                val said = mutableListOf<String>().also { texts(root, it) }
                assertTrue(said.toString(), RunnerTeam.ROLES_PURPOSE in said)
                tagged(root, "runner-person-role-bob")!!.config[SemanticsActions.OnClick].action!!.invoke()
            }
            ShadowLooper.idleMainLooper()
            scenario.onActivity {
                val item = everyRoot().firstNotNullOfOrNull { tagged(it, "runner-person-role-bob-maintainer") }
                assertNotNull("the menu offers Maintainer", item)
                item!!.config[SemanticsActions.OnClick].action!!.invoke()
            }
            ShadowLooper.idleMainLooper()
        }
        assertEquals(listOf("bob" to RunnerTeam.Role.MAINTAINER), asked)
    }

    @Test
    fun `a member sees roles in the rows and no picker`() {
        shown({ RunnerPeopleSection(team.copy(you = "bob"), null, null) }) { root, said ->
            assertNull(tagged(root, "runner-person-role-bob"))
            assertTrue(said.toString(), "alexandra-montgomery · maintainer · 3 reps" in said)
            assertTrue(said.toString(), RunnerTeam.ROLES_PURPOSE !in said)
        }
    }

    @Test
    fun `a removed phone is told so, and Check again asks`() {
        var asked = 0
        shown({ RemovedRow(longRunner, showLabel = true) { asked += 1 } }) { root, said ->
            assertTrue(said.toString(), RunnerTeam.REMOVED_TITLE in said)
            assertTrue(said.toString(), RunnerTeam.REMOVED_BODY in said)
            assertNotNull(tagged(root, "runner-removed"))
        }
        assertEquals(0, asked)
    }

    private fun screen(content: @androidx.compose.runtime.Composable () -> Unit) = @androidx.compose.runtime.Composable {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)) { content() }
    }

    @Test fun peopleOwner() = Capture.both("people-roles-owner", content = screen { RunnerPeopleSection(team, null) { _, _ -> } })

    @Test fun peopleMember() = Capture.both("people-roles-member", content = screen { RunnerPeopleSection(team.copy(you = "bob"), null, null) })

    @Test fun removed() = Capture.both("runner-removed", content = screen { RemovedRow(longRunner, showLabel = true) {} })
}
