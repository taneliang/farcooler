package com.farcooler.capture

import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.runtime.remember
import com.farcooler.core.CoreException
import com.farcooler.model.AgentConversation
import com.farcooler.model.AgentRowFixture
import com.farcooler.net.AgentRowStore
import com.farcooler.net.FakeRowSource
import com.farcooler.net.InMemoryPaneViews
import com.farcooler.net.NativePaneModel
import com.farcooler.net.RowJson
import com.farcooler.net.TestScope
import com.farcooler.net.eventually
import com.farcooler.ui.NativeAgentView
import kotlinx.serialization.json.JsonObject
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The conversation view as it draws, light and dark, from rows the real decoder
 * read (ov-374): a page holding every row kind, the stale banner over held rows,
 * and the box with its refusal lines. The views are the app's own, seeded through
 * the model; nothing is clicked. Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class NativeAgentCaptureTest {
    private val running = TestScope()
    private val source = FakeRowSource()

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

    private fun loaded(page: JsonObject): NativePaneModel {
        val model = NativePaneModel(
            terminal = "t1",
            store = AgentRowStore(running.scope, retryDelayMs = 1, followWaitMs = 1),
            source = source,
            sink = { _, _, _ -> false },
            memory = InMemoryPaneViews(),
            scope = running.scope,
        )
        source.answerPage(page)
        model.setOnScreen(true)
        eventually("rows") { model.store.shown.value.rows.isNotEmpty() }
        eventually("live") { model.store.shown.value.phase == AgentRowStore.Phase.Live }
        return model
    }

    @Test
    fun conversationEveryRowKind() {
        val model = loaded(AgentRowFixture.page)
        Capture.both("native-conversation") { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) }
    }

    @Test
    fun conversationNoticeTurn() {
        val model = loaded(
            RowJson.page(
                4, 12,
                RowJson.turn(0, "Count the lines in main.rs"),
                RowJson.prose(1, "There are **212** lines in `src/main.rs`."),
                RowJson.turn(2, "\"Count the lines\" finished", origin = "Notification", outcome = "Finished"),
            ),
        )
        model.onDraft("Thanks, now the tests")
        Capture.both("native-notice-turn") { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) }
    }

    @Test
    fun conversationStaleOverHeldRows() {
        val model = loaded(AgentRowFixture.page)
        source.failFollow(CoreException("The runner took too long to answer."))
        eventually("stale") { model.store.shown.value.isStale }
        model.onDraft("Is it still running?")
        Capture.both("native-stale") { NativeAgentView(model, rememberLazyListState(), showTerminal = {}) }
    }

    @Test
    fun conversationRefusalLines() {
        val model = loaded(RowJson.page(4, 12, RowJson.turn(0, "Fix the build"), RowJson.prose(1, "Looking at it.")))
        model.onDraft("Run the tests")
        model.issue = AgentConversation.SendIssue.Handoff
        Capture.both("native-handoff") { NativeAgentView(model, remember { androidx.compose.foundation.lazy.LazyListState() }, showTerminal = {}) }
        model.issue = AgentConversation.SendIssue.DraftInTerminal
        Capture.both("native-draft-in-terminal") { NativeAgentView(model, remember { androidx.compose.foundation.lazy.LazyListState() }, showTerminal = {}) }
        model.issue = AgentConversation.SendIssue.Said(AgentConversation.MAY_HAVE_BEEN_SENT)
        Capture.both("native-may-have-been-sent") { NativeAgentView(model, remember { androidx.compose.foundation.lazy.LazyListState() }, showTerminal = {}) }
    }
}
