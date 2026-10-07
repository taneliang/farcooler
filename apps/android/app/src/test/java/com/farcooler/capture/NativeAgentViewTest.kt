package com.farcooler.capture

import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.test.core.app.ActivityScenario
import com.farcooler.core.CoreException
import com.farcooler.model.AgentConversation
import com.farcooler.model.Capability
import com.farcooler.model.DaemonBuild
import com.farcooler.model.RunnerRefusal
import com.farcooler.model.Terminal
import com.farcooler.net.AgentRowStore
import com.farcooler.net.AgentRowsUnavailable
import com.farcooler.net.ClientCall
import com.farcooler.net.ConversationSink
import com.farcooler.net.FakeRowSource
import com.farcooler.net.InMemoryPaneViews
import com.farcooler.net.NativePaneModel
import com.farcooler.net.NativePanes
import com.farcooler.net.RowJson
import com.farcooler.net.TestScope
import com.farcooler.net.eventually
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.NativeLayer
import com.farcooler.ui.NativeSwitchButton
import com.farcooler.ui.rememberNativePane
import org.junit.After
import org.junit.Before
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowLooper

/**
 * The conversation view of a claude pane, drawn by the app's own composables
 * (ov-374): the switch and what it keeps, a reconnect, the setting going off and
 * on, notice turns, the stale banner, a pane removed.
 *
 * Compose's test rule can't run here (its idling goes through Espresso, whose
 * input injection is gone on SDK 37, see [Capture]), so the activity is launched
 * as [Capture] does, state is driven through the model, which is what a tap
 * calls, and the semantics tree is read. Nothing is clicked or typed.
 *
 * `PaneHarness` stands in for `TerminalPane` only where the pane needs a
 * connection and the native core, which Robolectric doesn't have: its terminal is
 * a counted stand-in. The conversation, its switch, the layer that covers the
 * terminal, the model and the store are the app's own, and the gate is
 * [AgentConversation.offered], the function the pane calls. Run only under
 * `-Pfarcooler.captures` (see `app/build.gradle.kts`), which is JDK 21's.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class NativeAgentViewTest {
    private val running = TestScope()
    private val source = FakeRowSource()
    private val memory = InMemoryPaneViews()
    private val composed = mutableListOf<String>()
    private val panes = NativePanes(
        running.scope,
        ClientCall { _, _ -> throw AssertionError("no call goes to a core here") },
        sourceFor = { source },
        sink = ConversationSink { _, text, _ -> composed.add(text); false },
    )

    /**
     * No animation scale: a spinner animating forever re-posts its frame at the
     * current time, and the native renderer draws every one of them, so a test
     * waiting while one is on screen waits for ever. With the scale at zero
     * Compose lands every animation on its end.
     */
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

    private val serving = DaemonBuild("1", true, "linux", setOf(Capability.AGENT_ROWS.wire, Capability.AGENT_COMPOSE.wire))
    private val notServing = DaemonBuild("1", true, "linux", setOf(Capability.TERMINALS.wire))
    private val claude = Terminal(id = "t1", preset = "claude", state = "running")

    /** What the harness reads and the test flips, as a pane's state and its connection's. */
    private class Pane {
        var daemon by mutableStateOf<DaemonBuild?>(null)
        var last by mutableStateOf<DaemonBuild?>(null)
        var live by mutableStateOf(true)
        var present by mutableStateOf(true)
        var keyboardDismissed by mutableIntStateOf(0)
        var terminalMounts = 0
        var terminalDisposals = 0
    }

    /** The pane's own native glue: `TerminalPane`'s lines about the conversation, around the app's own pieces. */
    @Composable
    private fun PaneHarness(pane: Pane) {
        if (!pane.present) return
        val native = rememberNativePane(
            terminalId = "t1",
            claudeInTerminal = true,
            offered = AgentConversation.offered(pane.daemon, pane.last, claude),
            live = pane.live,
            rich = AgentConversation.rich(pane.daemon ?: pane.last),
            interrupts = AgentConversation.interrupts(pane.daemon ?: pane.last),
            panes = panes,
            memory = memory,
            onCovered = { pane.keyboardDismissed += 1 },
        )
        Column(Modifier.fillMaxSize()) {
            if (native.switchable) NativeSwitchButton(showing = native.covered, onClick = { native.toggle() })
            Box(Modifier.weight(1f)) {
                NativeLayer(native, floatingSwitch = false) {
                    // The stand-in for the terminal: counted, so a remount shows.
                    DisposableEffect(Unit) {
                        pane.terminalMounts += 1
                        onDispose { pane.terminalDisposals += 1 }
                    }
                    Box(Modifier.fillMaxSize().testTag("terminal-surface")) { Text("terminal") }
                }
            }
        }
    }

    /**
     * A page with nothing animating in it: no older page to spin for, no open turn.
     * A spinner on screen re-posts its frame forever, and the native renderer draws
     * every one of them.
     */
    private fun plain() = RowJson.page(4, 12, RowJson.turn(0, "Fix the build"), RowJson.prose(1, "Done."))

    private class Seen(
        val merged: Map<String, String>,
        val unmerged: Map<String, String>,
        val disabled: Set<String>,
    ) {
        /** Reachable by TalkBack. */
        fun reachable(tag: String) = tag in merged

        /** In the tree at all, reachable or not. */
        fun composed(tag: String) = tag in unmerged
    }

    private fun read(activity: ComponentActivity): Seen {
        fun roots(view: View): List<RootForTest> = when {
            view is RootForTest -> listOf(view)
            view is ViewGroup -> (0 until view.childCount).flatMap { roots(view.getChildAt(it)) }
            else -> emptyList()
        }
        val disabled = HashSet<String>()
        /** A node's words: its own, or its descendants' in order. */
        fun words(node: SemanticsNode): String =
            node.config.getOrNull(SemanticsProperties.EditableText)?.text
                ?: node.config.getOrNull(SemanticsProperties.Text)?.joinToString(" ") { it.text }
                ?: node.children.map(::words).filter { it.isNotEmpty() }.joinToString(" ")
        fun walk(node: SemanticsNode, out: MutableMap<String, String>) {
            node.config.getOrNull(SemanticsProperties.TestTag)?.let { tag ->
                out[tag] = words(node)
                if (SemanticsProperties.Disabled in node.config) disabled.add(tag)
            }
            node.children.forEach { walk(it, out) }
        }
        val root = roots(activity.window.decorView).first()
        val merged = LinkedHashMap<String, String>()
        val unmerged = LinkedHashMap<String, String>()
        walk(root.semanticsOwner.rootSemanticsNode, merged)
        walk(root.semanticsOwner.unmergedRootSemanticsNode, unmerged)
        return Seen(merged, unmerged, disabled)
    }

    /**
     * Run what is due, moving the clock a frame at a time: a spinner animating
     * forever re-posts its frame at the current time, so an idle that doesn't move
     * the clock never returns while one is on screen.
     */
    private fun idle() {
        ShadowLooper.idleMainLooper(32, java.util.concurrent.TimeUnit.MILLISECONDS)
        ShadowLooper.idleMainLooper(32, java.util.concurrent.TimeUnit.MILLISECONDS)
    }

    /** Pump the main looper until [condition] holds on what is drawn. Never bounded under 30 s. */
    private fun ActivityScenario<ComponentActivity>.settle(what: String, condition: (Seen) -> Boolean) {
        var seen: Seen? = null
        eventually(what) {
            idle()
            onActivity { seen = read(it) }
            condition(seen!!)
        }
    }

    private fun ActivityScenario<ComponentActivity>.look(): Seen {
        idle()
        var seen: Seen? = null
        onActivity { seen = read(it) }
        return seen!!
    }

    private fun session(body: (ActivityScenario<ComponentActivity>, Pane, NativePaneModel) -> Unit) {
        val pane = Pane()
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { FarCoolerTheme { PaneHarness(pane) } } }
            idle()
            body(scenario, pane, panes.model("t1", memory))
        }
    }

    /** The runner serves the view, and has answered the page of fixture rows. */
    private fun Pane.serve() {
        daemon = serving
        last = serving
    }

    @Test
    fun `the switch keeps the draft and the terminal, and the covered terminal is out of TalkBack's reach`() = session { scenario, pane, model ->
        source.answerPage(plain())
        pane.serve()
        scenario.settle("the conversation") { it.composed("native-composer") }
        model.onDraft("half a sentence")
        scenario.settle("the draft in the box") { it.merged["native-composer"] == "half a sentence" }
        assertEquals(1, pane.terminalMounts)

        // Covered: composed under the conversation, and out of TalkBack's reach.
        val covered = scenario.look()
        assertTrue(covered.composed("terminal-surface"))
        assertFalse("TalkBack must not reach the covered terminal", covered.reachable("terminal-surface"))

        // The switch is the pane's, and the terminal gives up the keyboard to the conversation.
        model.switchTo(false)
        scenario.settle("the terminal") { it.reachable("terminal-surface") && !it.composed("native-composer") }
        assertTrue("the switch stays, to come back", scenario.look().composed("native-switch"))
        model.switchTo(true)
        scenario.settle("the conversation again") { it.composed("native-composer") }

        assertEquals("half a sentence", scenario.look().merged["native-composer"])
        assertEquals("the terminal was never rebuilt", 1, pane.terminalMounts)
        assertEquals(0, pane.terminalDisposals)
    }

    @Test
    fun `the terminal gives up the keyboard whenever the conversation covers it, switch or not`() = session { scenario, pane, model ->
        source.answerPage(plain())
        pane.serve()
        scenario.settle("covered") { it.composed("native-composer") }
        val first = pane.keyboardDismissed
        assertTrue("covering raised the dismiss", first >= 1)
        model.switchTo(false)
        scenario.settle("the terminal") { it.reachable("terminal-surface") }
        model.switchTo(true)
        scenario.settle("covered again") { it.composed("native-composer") }
        assertTrue(pane.keyboardDismissed > first)
        // Covered by claude starting in the pane, not by the switch: the pane wasn't offered.
        pane.daemon = notServing
        pane.last = notServing
        scenario.settle("the terminal") { it.reachable("terminal-surface") }
        val before = pane.keyboardDismissed
        pane.serve()
        scenario.settle("covered by the pane coming back") { it.composed("native-composer") }
        assertTrue(pane.keyboardDismissed > before)
    }

    @Test
    fun `a reconnect that has not read the build yet keeps the conversation on screen`() = session { scenario, pane, model ->
        source.answerPage(plain())
        pane.serve()
        scenario.settle("the conversation") { it.composed("native-composer") }
        model.onDraft("typing when the link blinked")
        scenario.settle("the draft") { it.merged["native-composer"] == "typing when the link blinked" }
        val stops = model.stops

        // A link coming up clears the build until `host` answers; the last known one stays.
        pane.daemon = null
        val during = scenario.look()
        assertTrue("the conversation stays", during.composed("native-composer"))
        assertFalse("the terminal doesn't come up behind it", during.reachable("terminal-surface"))
        assertEquals("the follow isn't stopped for the round trip", stops, model.stops)
        assertEquals("typing when the link blinked", during.merged["native-composer"])

        pane.daemon = serving
        assertTrue(scenario.look().composed("native-composer"))
        assertEquals(stops, model.stops)
        assertEquals(0, pane.terminalDisposals)
    }

    @Test
    fun `the setting turned off shows the terminal, and on again follows the conversation again`() = session { scenario, pane, model ->
        source.answerPage(plain())
        source.failFollow(AgentRowsUnavailable())
        pane.serve()
        scenario.settle("the stale banner, the runner having stopped serving rows") { it.composed("native-stale") }

        // Off: the reconnected hello no longer offers rows. The terminal shows.
        pane.daemon = notServing
        pane.last = notServing
        scenario.settle("the terminal") { it.reachable("terminal-surface") && !it.composed("native-composer") }
        assertFalse("nothing is read for a pane that isn't offered", model.store.isFollowing)

        // On: offered again. It follows again and the banner goes.
        source.answerNothing()
        pane.serve()
        scenario.settle("the conversation, following again") { it.composed("native-composer") && !it.composed("native-stale") }
        eventually("live again") { model.store.shown.value.phase == AgentRowStore.Phase.Live }
    }

    @Test
    fun `a conversation that stops being offered stops following, and the terminal shows`() = session { scenario, pane, model ->
        source.answerPage(plain())
        pane.serve()
        scenario.settle("following") { it.composed("native-composer") }
        eventually("a follow in flight") { source.followCalls.get() >= 1 && model.store.isFollowing }

        // The setting went off and the link reconnected: this pane isn't offered it.
        pane.daemon = notServing
        pane.last = notServing
        scenario.settle("the terminal") { it.reachable("terminal-surface") && !it.composed("native-composer") }
        assertFalse("nothing holds a call on the runner for a pane that isn't offered", model.store.isFollowing)
        assertFalse(scenario.look().composed("native-switch"))
    }

    @Test
    fun `a pane whose rows the runner won't serve shows its terminal with no switch, never an empty view`() = session { scenario, pane, _ ->
        source.failPage(AgentRowsUnavailable())
        pane.serve()
        scenario.settle("the terminal") { it.reachable("terminal-surface") && !it.composed("native-composer") }
        assertFalse(scenario.look().composed("native-switch"))
    }

    @Test
    fun `a turn nobody typed is drawn as a notice, never as the person's message`() = session { scenario, pane, _ ->
        source.answerPage(
            RowJson.page(
                1, 3,
                RowJson.turn(0, "Fix the build"),
                RowJson.turn(1, "\"build\" finished", origin = "Notification", outcome = null),
            ),
        )
        pane.serve()
        scenario.settle("both turns") { it.composed("native-notice-turn") && it.composed("native-prompt") }
        val seen = scenario.look()
        assertEquals("build finished", seen.unmerged["native-notice-turn"])
        assertEquals("Fix the build", seen.unmerged["native-prompt"])
    }

    @Test
    fun `rows held while the runner stops answering are said to be stale, and the box waits`() = session { scenario, pane, model ->
        source.answerPage(plain())
        pane.serve()
        scenario.settle("rows") { it.composed("native-row-prose:1") }
        model.onDraft("hello")
        scenario.settle("a box that can send") { "native-send" !in it.disabled && it.composed("native-send") }
        assertFalse(scenario.look().composed("native-stale"))

        source.failFollow(CoreException("The runner took too long to answer.", RunnerRefusal.TIMED_OUT_WORD))
        scenario.settle("the stale banner") { it.composed("native-stale") }
        assertEquals(AgentConversation.STALE_TROUBLE + " Show terminal", scenario.look().unmerged["native-stale"])
        assertTrue("Send waits while the rows may be old", "native-send" in scenario.look().disabled)

        source.answerPage(plain())
        scenario.settle("the banner gone") { !it.composed("native-stale") }
    }

    @Test
    fun `a pane removed stops its follow, and its draft is there when it comes back`() = session { scenario, pane, model ->
        source.answerPage(plain())
        pane.serve()
        scenario.settle("following") { it.composed("native-composer") }
        eventually("a follow in flight") { source.followCalls.get() >= 1 }
        model.onDraft("keep me")

        pane.present = false
        idle()
        assertFalse("a removed pane holds no call on the runner", model.store.isFollowing)
        assertEquals(1, model.stops)

        pane.present = true
        scenario.settle("the pane again") { it.merged["native-composer"] == "keep me" }
    }

    @Test
    fun `the follow stops in the background and resumes in front`() = session { scenario, pane, model ->
        source.answerPage(plain())
        pane.serve()
        scenario.settle("following") { it.composed("native-composer") }
        eventually("following") { model.store.isFollowing }
        pane.live = false
        idle()
        assertFalse(model.store.isFollowing)
        pane.live = true
        idle()
        eventually("following again") { model.store.isFollowing }
    }

    @Test
    fun `a dialog claude is showing is a handoff row with a way to the terminal`() = session { scenario, pane, model ->
        source.answerPage(plain())
        pane.serve()
        scenario.settle("rows") { it.composed("native-composer") }
        model.issue = AgentConversation.SendIssue.Handoff
        scenario.settle("the handoff") { it.composed("native-handoff-show-terminal") }
        model.issue = AgentConversation.SendIssue.DraftInTerminal
        scenario.settle("the draft-in-terminal line") { it.composed("native-send-issue") && !it.composed("native-handoff") }
    }

    @Test
    fun `a pane that is not claude in a terminal, or whose runner doesn't serve the view, is the terminal and nothing more`() = session { scenario, pane, _ ->
        pane.daemon = notServing
        pane.last = notServing
        val seen = scenario.look()
        assertTrue(seen.reachable("terminal-surface"))
        assertFalse(seen.composed("native-switch"))
        assertFalse(seen.composed("native-composer"))
        assertEquals("nothing is read", 0, source.pageCalls.get())
        assertNotNull(seen)
    }
}
