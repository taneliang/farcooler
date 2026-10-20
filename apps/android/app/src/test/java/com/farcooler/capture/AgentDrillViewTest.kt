package com.farcooler.capture

import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.test.core.app.ActivityScenario
import com.farcooler.model.AgentOpening
import com.farcooler.model.Fleet
import com.farcooler.model.RunningAgents
import com.farcooler.model.SubagentEntry
import com.farcooler.model.Terminal
import com.farcooler.model.Worktree
import com.farcooler.net.AgentRowStore
import com.farcooler.net.ClientCall
import com.farcooler.net.FakeRowSource
import com.farcooler.net.InMemoryPaneViews
import com.farcooler.net.NativePanes
import com.farcooler.net.RowJson
import com.farcooler.net.TestScope
import com.farcooler.net.eventually
import com.farcooler.ui.AgentsList
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.NativeAgentView
import com.farcooler.ui.carryOut
import kotlinx.serialization.json.JsonObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowLooper

/**
 * A subagent's own conversation (ov-641, folding ov-657), as the app draws it: a tap on its row in the agents view opens it
 * in its parent's place, through the same `AgentOpening` and `carryOut` the workspace screen runs, with a header and the
 * way back and no box to type into; and its captures at 411 dp and at 1.6x font, light and dark. Nothing is typed; the tap
 * is the row's own click action, since input injection is gone on SDK 37 (see [Capture]). Run only under
 * `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class AgentDrillViewTest {
    private val running = TestScope()
    private val source = FakeRowSource()
    private val child = FakeRowSource()
    private val panes = NativePanes(
        running.scope, ClientCall { _, _ -> throw AssertionError("no call goes to a core here") },
        sourceFor = { source.also { it.children["agent-a1"] = child } },
    )
    private val memory = InMemoryPaneViews()

    /** See `NativeAgentViewTest.noAnimations`. */
    @Before
    fun noAnimations() {
        android.provider.Settings.Global.putFloat(
            androidx.test.core.app.ApplicationProvider.getApplicationContext<android.content.Context>().contentResolver,
            android.provider.Settings.Global.ANIMATOR_DURATION_SCALE,
            0f,
        )
    }

    @After
    fun tearDown() = running.close()

    private val fleet = Fleet(
        runtimeHealthy = true, livePanes = 1,
        worktrees = listOf(
            Worktree(
                id = "checkout", workspace = "ws",
                terminals = listOf(
                    Terminal(
                        id = "t1", state = "running", activity = "working", role = "orchestrator", workspace = "ws",
                        subagents = listOf("Review ov-616"), subagentIds = listOf("toolu_1"), subagentsRunning = 1,
                    ),
                ),
            ),
        ),
    )

    private val now get() = System.currentTimeMillis()

    private fun pane(): com.farcooler.net.NativePaneModel {
        source.answerPage(
            RowJson.page(
                4, 12, RowJson.turn(0, "Split the feedback into lanes", outcome = null, startedMs = now - 200_000), RowJson.prose(1, "Four lanes are running."),
                RowJson.subagent(2, "toolu_1", "agent-a1", description = "Review ov-616", startedMs = now - 190_000),
            ),
        )
        child.answerPage(
            RowJson.page(
                5, 8,
                RowJson.turn(0, "Review ov-616: read the diff and say what to change", origin = "Other", outcome = null, startedMs = now - 190_000),
                RowJson.tool(1, "Read", "apps/android/app/src/main/java/com/farcooler/ui/WorkspaceCards.kt", input = "path: WorkspaceCards.kt"),
                RowJson.tool(2, "Grep", "AgentOpening", result = "3 files"),
                RowJson.prose(3, "The card counts the larger of the runner's figure and the names. One thing to change: the sheet's rows should share a height."),
            ),
        )
        val model = panes.model("t1", memory)
        model.offer(rich = false, interrupts = false, subagents = true)
        model.setOnScreen(true)
        eventually("rows") { model.store.shown.value.phase == AgentRowStore.Phase.Live }
        return model
    }

    /** The agents view over the conversation, wired as the workspace screen wires them. */
    @Composable
    private fun Screen(model: com.farcooler.net.NativePaneModel) = FarCoolerTheme {
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surface) {
            val shown by model.store.shown.collectAsStateWithLifecycle()
            val entries = remember(shown.rows) { SubagentEntry.of(shown.rows) }
            val agents = RunningAgents.workspace("ws", "r", fleet) { if (it == "t1") entries else null }
            Column(Modifier.fillMaxSize()) {
                AgentsList(
                    agents,
                    onChoose = { AgentOpening.of(it, "t1", wide = false, ownOffered = true).carryOut(panes, false, goTo = { _, _ -> }, focusChat = {}) },
                    modifier = Modifier.height(240.dp),
                )
                NativeAgentView(model, rememberLazyListState(), showTerminal = {}, modifier = Modifier.fillMaxWidth())
            }
        }
    }

    private fun roots(view: View): List<RootForTest> =
        if (view is RootForTest) listOf(view) else if (view is ViewGroup) (0 until view.childCount).flatMap { roots(view.getChildAt(it)) } else emptyList()

    private fun find(activity: ComponentActivity, tag: String): SemanticsNode? {
        fun walk(node: SemanticsNode): SemanticsNode? =
            if (node.config.getOrNull(SemanticsProperties.TestTag) == tag) node else node.children.firstNotNullOfOrNull(::walk)
        return roots(activity.window.decorView).firstOrNull()?.semanticsOwner?.unmergedRootSemanticsNode?.let(::walk)
    }

    @Test
    fun `a tap on a subagent's row opens its own conversation, with the way back and no box, and Back returns`() {
        val model = pane()
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { Screen(model) } }
            ShadowLooper.idleMainLooper()
            var row: SemanticsNode? = null
            var tries = 0
            while (row == null && tries++ < 400) {
                scenario.onActivity { row = find(it, "agent-row-t1-sub:toolu_1") }
                if (row == null) { Thread.sleep(5); ShadowLooper.idleMainLooper() }
            }
            assertNotNull("the subagent's row is in the agents view", row)
            scenario.onActivity { activity ->
                assertNull("nothing is open yet", model.opened)
                assertNull(find(activity, "native-agent-header"))
                assertNotNull("the pane's own box is there", find(activity, "native-composer-stack"))
                find(activity, "agent-row-t1-sub:toolu_1")!!.config[SemanticsActions.OnClick].action!!()
            }
            eventually("the agent opens") { model.opened?.agentId == "agent-a1" }
            ShadowLooper.idleMainLooper()
            eventually("its own rows") { model.opened?.store?.shown?.value?.rows?.isNotEmpty() == true }
            var settled = false
            tries = 0
            while (!settled && tries++ < 400) {
                ShadowLooper.idleMainLooper()
                scenario.onActivity { settled = find(it, "native-agent-header") != null && find(it, "native-row-prose:3") != null }
                if (!settled) Thread.sleep(5)
            }
            scenario.onActivity { activity ->
                assertTrue("the header and the agent's own words are drawn", settled)
                assertNull("nothing to send to an agent", find(activity, "native-composer-stack"))
                find(activity, "native-agent-back")!!.config[SemanticsActions.OnClick].action!!()
            }
            eventually("back to the pane") { model.opened == null }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { activity ->
                assertNull(find(activity, "native-agent-header"))
                assertNotNull("the box is back", find(activity, "native-composer-stack"))
            }
        }
    }

    @Test
    fun `a subagent row in the transcript opens it too`() {
        val model = pane()
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { Screen(model) } }
            ShadowLooper.idleMainLooper()
            var tries = 0
            var node: SemanticsNode? = null
            while (node == null && tries++ < 400) {
                scenario.onActivity { node = find(it, "native-subagent-open-sub:toolu_1") }
                if (node == null) { Thread.sleep(5); ShadowLooper.idleMainLooper() }
            }
            assertNotNull("the inline row is a button where the runner serves subagent rows", node)
            scenario.onActivity { find(it, "native-subagent-open-sub:toolu_1")!!.config[SemanticsActions.OnClick].action!!() }
            eventually("the agent opens") { model.opened?.agentId == "agent-a1" }
        }
    }

    // Captures: the agent's conversation, opened.

    private fun opened(): com.farcooler.net.NativePaneModel = pane().also {
        assertTrue(it.openAgent("sub:toolu_1", "agent-a1"))
        eventually("its rows") { it.opened?.store?.shown?.value?.phase == AgentRowStore.Phase.Live }
    }

    @Test
    fun capture() {
        val model = opened()
        Capture.both("agent-conversation-411", settleMs = 100) { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) }
    }

    @Test
    @Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi", fontScale = 1.6f)
    fun captureLargeFont() {
        val model = opened()
        Capture.both("agent-conversation-411-large-font", settleMs = 100) { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) }
    }
}
