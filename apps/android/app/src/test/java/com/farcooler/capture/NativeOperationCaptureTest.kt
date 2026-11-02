package com.farcooler.capture

import com.farcooler.model.AgentRow
import com.farcooler.net.AgentRowStore
import com.farcooler.net.FakeRowSource
import com.farcooler.net.InMemoryPaneViews
import com.farcooler.net.NativePaneModel
import com.farcooler.net.TestScope
import com.farcooler.net.eventually
import com.farcooler.ui.NativeToolGroupRow
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.After
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class NativeOperationCaptureTest {
    private val running = TestScope()
    @After fun close() = running.close()
    @Test fun completedFixtureGroupOnly() {
        var root: File? = File(System.getProperty("user.dir")).absoluteFile
        while (root != null && !File(root, "test/fixtures/agent-rows-ov893-producer.json").isFile) root = root.parentFile
        val fixture = requireNotNull(root).resolve("test/fixtures/agent-rows-ov893-producer.json")
        val source = FakeRowSource()
        val model = NativePaneModel(terminal = "fixture", store = AgentRowStore(running.scope, retryDelayMs = 1, followWaitMs = 1), source = source, sink = { _, _, _ -> false }, memory = InMemoryPaneViews(), scope = running.scope)
        source.answerPage(Json.parseToJsonElement(fixture.readText()).jsonObject)
        model.setOnScreen(true)
        eventually("fixture rows") { model.store.shown.value.rows.isNotEmpty() }
        val tools = model.store.shown.value.rows.filter { it.kind is AgentRow.Kind.OfTool }
        check(tools.size == 2)
        Capture.both("ov893-group", settleMs = 100) { NativeToolGroupRow("fixture", tools) }
    }
}
