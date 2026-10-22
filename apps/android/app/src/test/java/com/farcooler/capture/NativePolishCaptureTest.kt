package com.farcooler.capture

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.unit.dp
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.core.view.drawToBitmap
import androidx.test.core.app.ActivityScenario
import com.farcooler.model.AgentRow
import com.farcooler.model.AgentRowPage
import com.farcooler.net.AgentRowStore
import com.farcooler.net.FakeRowSource
import com.farcooler.net.InMemoryPaneViews
import com.farcooler.net.NativePaneModel
import com.farcooler.net.PromptImageSource
import com.farcooler.net.PromptImageStore
import com.farcooler.net.RowJson
import com.farcooler.net.TestScope
import com.farcooler.net.eventually
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.FoldedText
import com.farcooler.ui.LocalPromptImages
import com.farcooler.ui.NativeAgentView
import com.farcooler.ui.NativeToolGroupRow
import com.farcooler.ui.PromptImageStrip
import java.io.ByteArrayOutputStream
import kotlinx.coroutines.runBlocking
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
 * The conversation view's text-first polish as it draws (ov-459, ov-458): a long run
 * of tool calls as one line that opens to each call and its input and result, a task
 * checklist, a scheduled task and a long prompt that fold, and a prompt's images as
 * thumbnails that open at full size. Light and dark; `-Pfarcooler.captureQualifiers`
 * redraws every capture on a narrower screen. Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class NativePolishCaptureTest {
    private val running = TestScope()
    private val source = FakeRowSource()

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

    /** A picture of a colour per index, as the runner would send it back. */
    private val pictures = PromptImageSource { _, _, index ->
        val bitmap = Bitmap.createBitmap(640, 480, Bitmap.Config.ARGB_8888)
        val colors = intArrayOf(0xFF5B8DEF.toInt(), 0xFFE2725B.toInt(), 0xFF59A869.toInt(), 0xFFB07CC6.toInt(), 0xFFE0B341.toInt())
        Canvas(bitmap).apply {
            drawColor(colors[index % colors.size])
            drawCircle(320f, 240f, 120f, android.graphics.Paint().apply { color = Color.WHITE; alpha = 160 })
        }
        ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
    }

    private val longLine = "Fold this where it wraps past six lines. ".repeat(9) + "And one more."

    private fun polishPage(): JsonObject {
        val commands = listOf(
            "Map the conversation view code" to "command: rg -n NativeRows\ndescription: Map the conversation view code",
            "Find the row source and paste handling" to "command: rg -n RowSource\ndescription: Find the row source",
            "Read the tool rows" to "command: sed -n 1,80p NativeToolRows.kt",
            "Check the file sizes" to "command: wc -l apps/android/**/*.kt",
            "README check" to "command: head README.md",
            "Run the unit tests" to "command: ./gradlew testInstrumentedUnitTest",
            "Build the app" to "command: ./gradlew assembleDebug",
            "Lint the copy" to "command: python3 scripts/copy-lint.py",
        )
        val rows = mutableListOf<JsonObject>()
        rows += RowJson.turn(
            0, "Compare these [Image #1] [Image #2] [Image #3] [Image #4] [Image #5] and tell me which reads best.",
            images = List(5) { "image/png" },
        )
        rows += RowJson.thinking(1)
        commands.forEachIndexed { n, (summary, input) ->
            rows += RowJson.tool(2L + n * 2, "Bash", summary, input, "ok: ${summary.lowercase()}")
            if (n < commands.size - 1) rows += RowJson.thinking(3L + n * 2)
        }
        rows += RowJson.tasks(30, "Re-evaluate the plan" to "Completed", "Re-check the themes and lanes" to "InProgress", "Learn from the last check-in" to "Pending")
        rows += RowJson.prose(31, "Check-in done: nothing has changed. Main is green and no lanes are running.")
        rows += RowJson.turn(32, longLine, origin = "Scheduled")
        rows += RowJson.turn(33, longLine)
        return RowJson.page(4, 40, *rows.toTypedArray())
    }

    private fun loaded(page: JsonObject): NativePaneModel {
        val model = NativePaneModel(
            terminal = "t1",
            store = AgentRowStore(running.scope, retryDelayMs = 1, followWaitMs = 1),
            source = source,
            sink = { _, _, _ -> false },
            memory = InMemoryPaneViews(),
            scope = running.scope,
            imageSource = pictures,
        )
        model.offer(rich = true, interrupts = false, readsImages = true)
        source.answerPage(page)
        model.setOnScreen(true)
        eventually("rows") { model.store.shown.value.rows.isNotEmpty() }
        eventually("live") { model.store.shown.value.phase == AgentRowStore.Phase.Live }
        return model
    }

    @Test
    fun conversationPolish() {
        val model = loaded(polishPage())
        Capture.both("native-polish", settleMs = 150) { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) }
    }

    /** The person's bubble, short and long, under the system's increased contrast (ov-624). */
    @Test
    fun conversationPolishHighContrast() {
        val model = loaded(polishPage())
        Capture.bothHighContrast("native-polish") { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) }
    }

    /** The conversation in Light and Dark over a dark terminal theme, and under increased contrast (ov-690). */
    @Test
    fun conversationFollowsAppearance() {
        val model = loaded(polishPage())
        Capture.byAppearance("native-polish") { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) }
    }

    @Test
    fun conversationPolishRunOpen() {
        val page = AgentRowPage.decode(polishPage())
        val tools = page.rows.filter { it.kind is AgentRow.Kind.OfTool || it.kind is AgentRow.Kind.OfThinking }
        Capture.both("native-polish-run-open") {
            androidx.compose.foundation.layout.Column(androidx.compose.ui.Modifier.padding(16.dp)) {
                NativeToolGroupRow("tools:${tools.first().id}", tools.drop(1).take(5), startOpen = true)
            }
        }
    }

    @Test
    fun promptImagesFullSize() {
        val store = PromptImageStore("t1").also { it.source = pictures }
        runBlocking { repeat(5) { store.load("turn:0", it) } }
        val images = List(5) { AgentRow.PromptImage("image/png") }
        Capture.bothWithDialog("native-prompt-image-full", settleMs = 150) {
            PromptImageStrip("turn:0", images, store, startOpen = 1)
        }
    }

    // What the semantics tree says, with the group's own tap invoked: no Espresso here (see [Capture]).

    private class Tree(val root: SemanticsNode) {
        fun all(node: SemanticsNode = root): List<SemanticsNode> = listOf(node) + node.children.flatMap { all(it) }
        fun tagged(tag: String) = all().filter { it.config.getOrNull(SemanticsProperties.TestTag) == tag }
    }

    private fun roots(view: View): List<RootForTest> = when {
        view is RootForTest -> listOf(view)
        view is ViewGroup -> (0 until view.childCount).flatMap { roots(view.getChildAt(it)) }
        else -> emptyList()
    }

    private fun withContent(content: @androidx.compose.runtime.Composable () -> Unit, body: (ActivityScenario<ComponentActivity>, () -> Tree) -> Unit) {
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity {
                it.setContent {
                    FarCoolerTheme { androidx.compose.material3.Surface(androidx.compose.ui.Modifier.fillMaxSize()) { content() } }
                }
            }
            // Twice, a frame apart: a fold is known after the text's first layout, and shows on the next.
            // Drawn, as the captures are: text is laid out when the window is drawn, and a fold is known after it.
            repeat(4) {
                Thread.sleep(50)
                scenario.onActivity { it.window.decorView.drawToBitmap() }
                ShadowLooper.idleMainLooper(32, java.util.concurrent.TimeUnit.MILLISECONDS)
            }
            body(scenario) {
                var tree: Tree? = null
                scenario.onActivity { tree = Tree(roots(it.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode) }
                tree!!
            }
        }
    }

    @Test
    fun aGroupOpensToItsCallsWhenTapped() {
        val page = AgentRowPage.decode(polishPage())
        val tools = page.rows.filter { it.kind is AgentRow.Kind.OfTool || it.kind is AgentRow.Kind.OfThinking }.drop(1).take(5)
        withContent({ NativeToolGroupRow("g", tools) }) { scenario, tree ->
            val header = tree().all().first { SemanticsActions.OnClick in it.config }
            assertEquals("Collapsed", header.config.getOrNull(SemanticsProperties.StateDescription))
            assertEquals(
                "Ran 3 commands: map the conversation view code, find the row source and paste handling, read the tool rows",
                header.config.getOrNull(SemanticsProperties.ContentDescription)?.joinToString(),
            )
            assertTrue(tree().tagged("native-tool-group-rows").isEmpty())
            scenario.onActivity { header.config[SemanticsActions.OnClick].action!!.invoke() }
            ShadowLooper.idleMainLooper()
            assertEquals(1, tree().tagged("native-tool-group-rows").size)
            assertEquals(3, tree().all().count { it.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("native-tool-tool:") == true })
            assertEquals("Expanded", tree().all().first { SemanticsActions.OnClick in it.config }.config.getOrNull(SemanticsProperties.StateDescription))
        }
    }

    @Test
    fun aLongMessageFoldsWhereItWrapsPastSixLinesAndAShortOneDoesNot() {
        // The page's two long messages (a scheduled task's, folded at three lines, and a prompt's, at six)
        // fold; its short prompt, its images' words and its reply don't.
        val model = loaded(polishPage())
        withContent({ NativeAgentView(model, rememberLazyListState(), showTerminal = {}) }) { scenario, tree ->
            // Laid out when drawn, and shown a frame after: drawn until it is, never bounded under 30 s.
            eventually("both long messages folded") {
                scenario.onActivity { it.window.decorView.drawToBitmap() }
                ShadowLooper.idleMainLooper(32, java.util.concurrent.TimeUnit.MILLISECONDS)
                tree().tagged("native-show-more").size == 2
            }
        }
    }

    @Test
    fun noThumbnailsAreDrawnWhereTheRunnerServesNoImages() {
        val model = loaded(polishPage())
        model.offer(rich = true, interrupts = false, readsImages = false)
        withContent({
            CompositionLocalProvider(LocalPromptImages provides model.promptImages) {
                NativeAgentView(model, rememberLazyListState(), showTerminal = {})
            }
        }) { _, tree ->
            assertFalse(tree().tagged("native-prompt-images").isNotEmpty())
        }
    }
}
