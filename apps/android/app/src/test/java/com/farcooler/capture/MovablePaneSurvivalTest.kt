package com.farcooler.capture

import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.test.core.app.ActivityScenario
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.TreePanelState
import com.farcooler.ui.WideContent
import com.farcooler.ui.WideDestination
import com.farcooler.ui.WideWorkspaceFrame
import com.farcooler.ui.WorkspaceLayout
import com.farcooler.ui.rememberMovablePane
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowLooper

/**
 * ov-353 R2-1: does the orchestrator pane keep what it holds when the window
 * crosses 840 dp? `WorkspaceScreen` builds its one pane with `movableContentOf`
 * and calls it from the phone's `Scaffold` and from the wide frame's chat
 * (an `extraPane`, composed in the scaffold's own subcomposition). The pane here
 * is a stand-in that holds the three kinds of state the real one holds (a plain
 * `remember`, a `rememberSaveable` draft and a list's scroll), because a
 * connection needs the native core Robolectric doesn't have; everything around
 * it, the frame included, is the app's own. The layout kind is flipped on one
 * composition, which is what a window resize does to `WorkspaceScreen`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w1000dp-h700dp-xhdpi")
class MovablePaneSurvivalTest {
    private class Held {
        var plain by mutableIntStateOf(0)
        var draft by mutableStateOf("")
    }

    private var created = 0
    private var held: Held? = null
    private val kind = mutableStateOf(WorkspaceLayout.Kind.THREE_PANE)

    /** What `WorkspaceScreen` does with its pane: one movable block, one call site per layout. */
    @Composable
    private fun Screen(movable: Boolean) {
        val body: @Composable (Unit) -> Unit = {
            remember { Held().also { created++; held = it } }
            var saved by rememberSaveable { mutableStateOf("") }
            val list = rememberLazyListState()
            held?.draft = saved
            LazyColumn(Modifier.fillMaxSize(), state = list) { items(100) { Text("row $it") } }
            scroll = list
            setSaved = { saved = it }
        }
        val pane: @Composable (Unit) -> Unit = if (movable) rememberMovablePane(null, body) else body
        if (kind.value != WorkspaceLayout.Kind.PHONE) {
            WideWorkspaceFrame(
                kind = kind.value,
                destination = WideDestination.PLAN,
                panel = remember { TreePanelState() },
                onSelect = {},
                onBack = {},
                onNeedsYou = {},
                topBar = {},
            ) { content -> if (content == WideContent.CHAT) pane(Unit) else Box(Modifier) {} }
        } else {
            Scaffold { Box(Modifier.fillMaxSize()) { pane(Unit) } }
        }
    }

    private var scroll: androidx.compose.foundation.lazy.LazyListState? = null
    private var setSaved: (String) -> Unit = {}

    private fun crossing(movable: Boolean): Triple<Int, String, Int> {
        var result = Triple(0, "", 0)
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { FarCoolerTheme { Screen(movable) } } }
            ShadowLooper.idleMainLooper()
            scenario.onActivity {
                held!!.plain = 7
                setSaved("half a sentence")
            }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { kotlinx.coroutines.runBlocking { scroll!!.scrollToItem(40) } }
            ShadowLooper.idleMainLooper()
            // Folds and unfolds, and the 1200 dp crossing between the wide layouts.
            for (next in listOf(WorkspaceLayout.Kind.PHONE, WorkspaceLayout.Kind.THREE_PANE, WorkspaceLayout.Kind.TWO_PANE, WorkspaceLayout.Kind.PHONE, WorkspaceLayout.Kind.TWO_PANE)) {
                scenario.onActivity { kind.value = next }
                ShadowLooper.idleMainLooper()
                ShadowLooper.idleMainLooper()
            }
            scenario.onActivity { result = Triple(held!!.plain, held!!.draft, scroll!!.firstVisibleItemIndex) }
        }
        return result
    }

    @Test
    fun `the movable pane keeps its state across a fold and an unfold`() {
        val (plain, draft, index) = crossing(movable = true)
        println("RESULT movable: plain=$plain draft='$draft' scroll=$index created=$created")
        assertEquals(1, created)
        assertEquals(7, plain)
        assertEquals("half a sentence", draft)
        assertEquals(40, index)
    }

    @Test
    fun `without the movable block the same crossing loses it`() {
        val (plain, draft, index) = crossing(movable = false)
        println("RESULT plain: plain=$plain draft='$draft' scroll=$index created=$created")
        assertEquals("every crossing builds the pane again", true, created > 1)
        assertEquals(0, plain)
        assertEquals("", draft)
        assertEquals(0, index)
    }
}
