package com.farcooler.capture

import android.graphics.Bitmap
import android.graphics.Color as AndroidColor
import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.test.core.app.ActivityScenario
import com.farcooler.model.AgentConversation
import com.farcooler.model.OutgoingImage
import com.farcooler.net.AgentRowStore
import com.farcooler.net.ConversationSink
import com.farcooler.net.FakeRowSource
import com.farcooler.net.InMemoryPaneViews
import com.farcooler.net.InterruptSink
import com.farcooler.net.NativePaneModel
import com.farcooler.net.RowJson
import com.farcooler.net.TestScope
import com.farcooler.net.eventually
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.NativeAgentView
import java.io.ByteArrayOutputStream
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.serialization.json.JsonObject
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
 * The conversation composer as the app draws it (ov-404), through the model a
 * tap calls: the photo button only where the runner has `compose`, a chip per
 * photo with a way to take it out, Stop only while claude works and not under a
 * dialog, Send now on a Queued row. Nothing is clicked or typed (see
 * [NativeAgentViewTest]); each state is also captured, light and dark, from the
 * app's own views. Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class NativeComposerViewTest {
    private val running = TestScope()
    private val source = FakeRowSource()
    private val composed = CopyOnWriteArrayList<List<OutgoingImage>>()
    private val pressed = CopyOnWriteArrayList<String>()
    private var queues = false

    /** See `NativeAgentViewTest.noAnimations`. */
    @Before
    fun noAnimations() {
        android.provider.Settings.Global.putFloat(
            androidx.test.core.app.ApplicationProvider.getApplicationContext<android.content.Context>().contentResolver,
            android.provider.Settings.Global.ANIMATOR_DURATION_SCALE,
            0f,
        )
    }

    private val models = CopyOnWriteArrayList<NativePaneModel>()

    /**
     * Ends the test with nothing still writing Compose state. A send's or a key's
     * `finally` writes `sending` and `pressing` from [running]'s thread after the
     * call the test waited for, and a global snapshot write left pending as the
     * test ends keeps the process's one apply-notification flag set: every later
     * Robolectric test then misses its recompositions (the Wide workspace's Back
     * test failed in a full run, 5 runs of 6, with this class and in none without).
     */
    @After
    fun tearDown() {
        models.forEach { model -> eventually("the model to settle") { !model.sending && model.pressing == null } }
        idle()
        androidx.compose.runtime.snapshots.Snapshot.sendApplyNotifications()
        idle()
        running.close()
    }

    private fun loaded(page: JsonObject, rich: Boolean = true, interrupts: Boolean = true): NativePaneModel {
        val model = NativePaneModel(
            terminal = "t1",
            store = AgentRowStore(running.scope, retryDelayMs = 1, followWaitMs = 1),
            source = source,
            sink = ConversationSink { _, _, images -> composed.add(images); queues },
            memory = InMemoryPaneViews(),
            scope = running.scope,
            interruptSink = object : InterruptSink {
                override suspend fun interrupt(terminal: String) { pressed.add("stop") }
                override suspend fun sendNow(terminal: String) { pressed.add("sendnow") }
            },
        )
        model.offer(rich, interrupts)
        models.add(model)
        source.answerPage(page)
        model.setOnScreen(true)
        eventually("rows") { model.store.shown.value.rows.isNotEmpty() }
        eventually("live") { model.store.shown.value.phase == AgentRowStore.Phase.Live }
        return model
    }

    private fun idle() {
        ShadowLooper.idleMainLooper(32, java.util.concurrent.TimeUnit.MILLISECONDS)
        ShadowLooper.idleMainLooper(32, java.util.concurrent.TimeUnit.MILLISECONDS)
    }

    private class Seen(val text: Map<String, String>, val disabled: Set<String>) {
        fun composed(tag: String) = tag in text

        /** The chips drawn: one tag each, `native-image-chip`, `native-image-chip#2`, and not their strip. */
        fun chips() = text.keys.count { Regex("native-image-chip(#\\d+)?").matches(it) }
    }

    private fun read(activity: ComponentActivity): Seen {
        fun roots(view: View): List<RootForTest> = when {
            view is RootForTest -> listOf(view)
            view is ViewGroup -> (0 until view.childCount).flatMap { roots(view.getChildAt(it)) }
            else -> emptyList()
        }
        val text = LinkedHashMap<String, String>()
        val disabled = HashSet<String>()
        fun words(node: SemanticsNode): String =
            node.config.getOrNull(SemanticsProperties.EditableText)?.text
                ?: node.config.getOrNull(SemanticsProperties.Text)?.joinToString(" ") { it.text }
                ?: node.children.map(::words).filter { it.isNotEmpty() }.joinToString(" ")
        fun walk(node: SemanticsNode) {
            node.config.getOrNull(SemanticsProperties.TestTag)?.let { tag ->
                // A tag drawn more than once (a chip each) keeps a count under tag#n.
                var key = tag
                var n = 1
                while (key in text) key = "$tag#${++n}"
                text[key] = words(node)
                if (SemanticsProperties.Disabled in node.config) disabled.add(key)
            }
            node.children.forEach(::walk)
        }
        walk(roots(activity.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode)
        return Seen(text, disabled)
    }

    private fun look(model: NativePaneModel, body: (Seen) -> Unit) {
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity {
                it.setContent { FarCoolerTheme { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) } }
            }
            idle()
            var seen: Seen? = null
            eventually("the composer is drawn") {
                idle()
                scenario.onActivity { seen = read(it) }
                seen!!.composed("native-composer")
            }
            body(seen!!)
        }
    }

    /** A real PNG, so the chip decodes one. */
    private fun png(color: Int = AndroidColor.rgb(0, 150, 150)): ByteArray {
        val bitmap = Bitmap.createBitmap(64, 64, Bitmap.Config.ARGB_8888).apply { eraseColor(color) }
        return ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
    }

    private fun busy() = RowJson.page(4, 12, RowJson.turn(0, "Tidy the parser", outcome = null, activity = "Busy"))

    private fun add(model: NativePaneModel, vararg colors: Int) = kotlinx.coroutines.runBlocking {
        model.attachPicked(colors.map { png(it) }) { null }
    }

    @Test
    fun `the photo button is there only where the runner has compose`() {
        look(loaded(busy())) { assertTrue(it.composed("native-attach")) }
        look(loaded(busy(), rich = false)) { assertFalse("a photo button over a runner that takes none", it.composed("native-attach")) }
    }

    @Test
    fun `a photo is a chip with a button to take it out, and a photo alone can be sent`() {
        val model = loaded(busy())
        look(model) { assertTrue("Send waits for a word or a photo", "native-send" in it.disabled) }
        add(model, AndroidColor.rgb(0, 150, 150), AndroidColor.rgb(200, 80, 0))
        look(model) {
            assertEquals(2, it.chips())
            assertEquals(2, it.text.keys.count { k -> k.startsWith("native-image-remove") })
            assertFalse("a photo alone is a message", "native-send" in it.disabled)
        }
        Capture.both("native-composer-chips") { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) }
        model.detach(model.images.first().id)
        look(model) { assertEquals(1, it.chips()) }
    }

    @Test
    fun `Stop shows while claude works, and not under a dialog or on a runner that can't press it`() {
        val working = loaded(busy())
        look(working) { assertTrue("Stop while claude works", it.composed("native-stop")) }
        Capture.both("native-composer-stop") { NativeAgentView(working, rememberLazyListState(), showTerminal = {}) }
        working.stop()
        eventually("the press") { pressed.toList() == listOf("stop") }

        val waiting = loaded(RowJson.page(4, 12, RowJson.turn(0, "Tidy", outcome = null, activity = "Waiting")))
        look(waiting) { assertFalse("an Esc under a dialog would answer it No", it.composed("native-stop")) }

        val unserved = loaded(busy(), interrupts = false)
        look(unserved) { assertFalse(it.composed("native-stop")) }
    }

    @Test
    fun `a Queued row has Send now while claude works`() {
        queues = true
        val model = loaded(busy())
        model.onDraft("After this, the tests")
        model.send()
        eventually("queued") { model.queued == listOf("After this, the tests") }
        look(model) {
            assertTrue(it.composed("native-queued"))
            assertTrue("Send now on the Queued row", it.composed("native-send-now"))
        }
        model.sendNow()
        eventually("the press") { pressed.toList() == listOf("sendnow") }
    }

    @Test
    fun `a refusal is worded for what it says`() {
        val model = loaded(busy())
        model.issue = AgentConversation.keyIssue(AgentConversation.SendFailure.Refused("settling"), AgentConversation.PaneKey.Stop)
        look(model) { assertEquals("Claude is starting a step. Try Stop again in a moment. Dismiss", it.text["native-send-issue"]) }
        Capture.both("native-composer-refused") { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) }
        // A Queued row with Send now, drawn by the app's own transcript.
        queues = true
        val queuing = loaded(busy())
        queuing.onDraft("After this, the tests")
        queuing.send()
        eventually("queued") { queuing.queued == listOf("After this, the tests") }
        Capture.both("native-composer-send-now") { NativeAgentView(queuing, rememberLazyListState(), showTerminal = {}) }
    }

    private fun resting(suggestion: String?, activity: String = "Idle") =
        RowJson.page(4, 12, RowJson.turn(0, "Why does the parser test fail?", activity = activity, suggestion = suggestion))

    @Test
    fun `claude's suggestion stands in the empty box and a tap takes it as a draft (ov-409)`() {
        val model = loaded(resting("Run the tests again"))
        look(model) { assertEquals("Run the tests again", it.text["native-suggestion"]) }
        Capture.both("native-composer-suggestion") { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) }
        val long = loaded(
            resting("Add a regression test for the empty input case in parse.rs, run the whole suite, and then summarize what changed in the lexer"),
        )
        Capture.both("native-composer-suggestion-long") { NativeAgentView(long, rememberLazyListState(), showTerminal = {}) }
        assertTrue(model.takeSuggestion())
        look(model) {
            assertFalse("a draft replaces the suggestion", it.composed("native-suggestion"))
            assertEquals("Run the tests again", it.text["native-composer"])
        }
        Capture.both("native-composer-suggestion-taken") { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) }
        assertEquals("never sent", 0, composed.size)
        // While claude works, its dim line is a hint, not a prediction.
        look(loaded(resting("Run the tests again", activity = "Busy"))) { assertFalse(it.composed("native-suggestion")) }
    }

    @Test
    fun `Send now is not on a Queued row under a dialog`() {
        queues = true
        val model = loaded(RowJson.page(4, 12, RowJson.turn(0, "Tidy", outcome = null, activity = "Waiting")))
        model.onDraft("Wait for me")
        model.send()
        eventually("queued") { model.queued == listOf("Wait for me") }
        look(model) {
            assertTrue(it.composed("native-queued"))
            assertFalse("Send now over a dialog", it.composed("native-send-now"))
            assertFalse(it.composed("native-stop"))
        }
    }

    /**
     * A hardware keyboard's Ctrl+Enter sends, and Enter alone doesn't: the key event
     * goes to the composer's own field, focused through its semantics action (no
     * input is injected; see [NativeAgentViewTest]), so a broken handler is red.
     */
    @Test
    fun `Ctrl and Enter send from a hardware keyboard, and Enter alone is a new line`() {
        val model = loaded(busy())
        model.onDraft("From the keys")
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity {
                it.setContent { FarCoolerTheme { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) } }
            }
            idle()
            fun key(down: Boolean, meta: Int) = scenario.onActivity { activity ->
                val action = if (down) android.view.KeyEvent.ACTION_DOWN else android.view.KeyEvent.ACTION_UP
                val now = android.os.SystemClock.uptimeMillis()
                activity.window.decorView.dispatchKeyEvent(
                    android.view.KeyEvent(now, now, action, android.view.KeyEvent.KEYCODE_ENTER, 0, meta),
                )
            }
            fun focusComposer() = scenario.onActivity { activity ->
                fun roots(view: View): List<RootForTest> = when {
                    view is RootForTest -> listOf(view)
                    view is ViewGroup -> (0 until view.childCount).flatMap { roots(view.getChildAt(it)) }
                    else -> emptyList()
                }
                fun find(node: SemanticsNode): SemanticsNode? =
                    if (node.config.getOrNull(SemanticsProperties.TestTag) == "native-composer") node
                    else node.children.firstNotNullOfOrNull(::find)
                val node = find(roots(activity.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode)
                node?.config?.getOrNull(androidx.compose.ui.semantics.SemanticsActions.RequestFocus)?.action?.invoke()
            }
            focusComposer()
            idle()
            // Enter alone: the text field's own, never a send.
            key(true, 0); key(false, 0)
            idle()
            assertEquals("Enter alone sent", 0, composed.size)
            key(true, android.view.KeyEvent.META_CTRL_ON); key(false, android.view.KeyEvent.META_CTRL_ON)
            idle()
            eventually("Ctrl+Enter sent it") { composed.size == 1 }
        }
    }
}
