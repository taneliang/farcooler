package com.farcooler.capture

import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.test.core.app.ActivityScenario
import com.farcooler.model.AgentConversation
import com.farcooler.net.AgentRowStore
import com.farcooler.net.AnswerSink
import com.farcooler.net.FakeRowSource
import com.farcooler.net.InMemoryPaneViews
import com.farcooler.net.NativePaneModel
import com.farcooler.net.RowJson
import com.farcooler.net.TestScope
import com.farcooler.net.eventually
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.NativeAgentView
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowLooper

/**
 * A held ask answered from its row in the conversation view (ov-370, R-33): the
 * app's own `NativeAgentView` over a model whose rows come through its own
 * store, each button pressed through its own semantics action (what a tap and
 * TalkBack both call, with no input injected), and what reached the runner's
 * `terminal.agent_answer` read back. Also the captures, light and dark. Run
 * only under `-Pfarcooler.captures`, which is JDK 21's.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class NativeAnswersViewTest {
    private val running = TestScope()
    private val source = FakeRowSource()
    private val answered = mutableListOf<String>()

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

    private fun loaded(which: String, held: String? = "hook-ask-1", answers: Boolean = true): NativePaneModel {
        val model = NativePaneModel(
            terminal = "t1",
            store = AgentRowStore(running.scope, retryDelayMs = 1, followWaitMs = 1),
            source = source,
            sink = { _, _, _ -> false },
            memory = InMemoryPaneViews(),
            scope = running.scope,
            answers = if (answers) AnswerSink { _, ask, option, given -> synchronized(answered) { answered.add("$ask $option $given") } } else null,
        )
        source.answerPage(RowJson.page(4, 12, RowJson.turn(0, "Make the sign-up button stand out."), RowJson.heldAsk(1, which, held)))
        model.setOnScreen(true)
        eventually("rows") { model.store.shown.value.rows.size == 2 }
        return model
    }

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

    private fun idle() {
        ShadowLooper.idleMainLooper(32, java.util.concurrent.TimeUnit.MILLISECONDS)
        ShadowLooper.idleMainLooper(32, java.util.concurrent.TimeUnit.MILLISECONDS)
    }

    /** The view of [model], and a way to press a tag and read what's drawn. */
    private fun drawn(model: NativePaneModel, body: (tags: () -> Map<String, SemanticsNode>, press: (String) -> Unit) -> Unit) {
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { FarCoolerTheme { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) } } }
            var seen = emptyMap<String, SemanticsNode>()
            val tags = {
                idle()
                scenario.onActivity { seen = nodes(it) }
                seen
            }
            eventually("the ask's row") { "native-row-ask:1" in tags() }
            val press = { tag: String ->
                val node = tags()[tag] ?: throw AssertionError("$tag isn't drawn: ${seen.keys}")
                scenario.onActivity { node.config[SemanticsActions.OnClick].action!!.invoke() }
                idle()
            }
            body(tags, press)
        }
    }

    private fun heard(count: Int): List<String> {
        eventually("$count answers") { synchronized(answered) { answered.size >= count } }
        return synchronized(answered) { answered.toList() }
    }

    @Test
    fun `a held permission's Allow and Deny answer it`() = drawn(loaded("permission")) { tags, press ->
        assertTrue(tags().keys.containsAll(listOf("native-ask-allow", "native-ask-deny", "native-ask-show-terminal")))
        press("native-ask-allow")
        assertEquals(listOf("hook-ask-1 allow {}"), heard(1))
        press("native-ask-deny")
        assertEquals(listOf("hook-ask-1 allow {}", "hook-ask-1 deny {}"), heard(2))
    }

    @Test
    fun `a held question waits for a pick, then sends it`() = drawn(loaded("question")) { tags, press ->
        val send = tags()["native-ask-send-answer"] ?: throw AssertionError("no Send answer")
        assertTrue("nothing picked yet", SemanticsProperties.Disabled in send.config)
        press("native-ask-option-0-1")
        eventually("Send answer enabled") { SemanticsProperties.Disabled !in tags()["native-ask-send-answer"]!!.config }
        press("native-ask-send-answer")
        assertEquals(listOf("hook-ask-1 answer {Which color should the button be?=Blue}"), heard(1))
    }

    @Test
    fun `a held plan shows the plan, and Approve plan and Keep planning answer it`() = drawn(loaded("plan")) { tags, press ->
        assertTrue(tags().keys.containsAll(listOf("native-ask-plan", "native-ask-approve", "native-ask-keep-planning")))
        press("native-ask-approve")
        press("native-ask-keep-planning")
        assertEquals(listOf("hook-ask-1 allow {}", "hook-ask-1 deny {}"), heard(2))
    }

    @Test
    fun `a hold that ended, or a runner that takes no answers, leaves Show terminal alone`() {
        for ((which, held, answers) in listOf(Triple("question", null, true), Triple("plan", "hook-ask-1", false))) {
            drawn(loaded(which, held, answers)) { tags, _ ->
                val seen = tags().keys
                assertTrue("$which: $seen", "native-ask-show-terminal" in seen)
                for (button in listOf("native-ask-allow", "native-ask-send-answer", "native-ask-approve", "native-ask-keep-planning")) {
                    assertFalse("$which: $button", button in seen)
                }
            }
        }
        assertTrue(answered.isEmpty())
    }

    @Test
    fun captures() {
        for (which in listOf("question", "plan", "permission")) {
            val model = loaded(which)
            Capture.both("native-held-$which") { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) }
        }
        val answeredOnMac = NativePaneModel(
            terminal = "t2",
            store = AgentRowStore(running.scope, retryDelayMs = 1, followWaitMs = 1),
            source = source,
            sink = { _, _, _ -> false },
            memory = InMemoryPaneViews(),
            scope = running.scope,
        )
        source.answerPage(RowJson.page(4, 12, RowJson.turn(0, "Make the sign-up button stand out."), RowJson.heldAsk(1, "permission", null, "Mac")))
        answeredOnMac.setOnScreen(true)
        eventually("rows") { answeredOnMac.store.shown.value.rows.size == 2 }
        assertEquals("Answered on Mac", AgentConversation.askTitle((answeredOnMac.store.shown.value.rows[1].kind as com.farcooler.model.AgentRow.Kind.OfAsk).ask))
        Capture.both("native-held-answered") { NativeAgentView(answeredOnMac, rememberLazyListState(), showTerminal = {}) }
    }
}
