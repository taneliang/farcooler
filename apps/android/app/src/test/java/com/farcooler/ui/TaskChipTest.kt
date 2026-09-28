package com.farcooler.ui

import androidx.compose.runtime.AbstractApplier
import androidx.compose.runtime.BroadcastFrameClock
import androidx.compose.runtime.Composition
import androidx.compose.runtime.Recomposer
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import com.farcooler.model.TaskBoard
import com.farcooler.model.TaskBoardColumn
import com.farcooler.model.TaskRef
import com.farcooler.model.TaskRow
import com.farcooler.model.TaskStatus
import com.farcooler.model.Terminal
import com.farcooler.model.WorkspaceSummary
import com.farcooler.model.Worktree
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test

/**
 * `rememberTaskChip` in a real composition: what the top bar's chip does when
 * tapped. The same plain-JVM harness as `MinuteClockLifecycleTest`: the chip's
 * state draws nothing, so a no-op applier runs it whole, and its board reads
 * run in virtual time.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class TaskChipTest {
    private val main = StandardTestDispatcher()

    @Before
    fun mainIsTheTestScheduler() = Dispatchers.setMain(main)

    @After
    fun mainIsMainAgain() = Dispatchers.resetMain()

    private class NoNodes : AbstractApplier<Unit>(Unit) {
        override fun insertTopDown(index: Int, instance: Unit) {}
        override fun insertBottomUp(index: Int, instance: Unit) {}
        override fun remove(index: Int, count: Int) {}
        override fun move(from: Int, to: Int, count: Int) {}
        override fun onClear() {}
    }

    private val invoice = TaskRef("t-9", "bil-9", "Invoice PDF export", "in_progress")
    private val card = TaskRow("t-9", "bil-9", "Invoice PDF export", TaskStatus.IN_PROGRESS, 0L)
    private val pane = Terminal(id = "p", preset = "claude", state = "running", workspace = "ws-pane")
    private val worktree = Worktree(id = "w", workspace = "ws-owner", repository = "repo", openTasks = listOf(invoice))
    private val list = listOf(
        WorkspaceSummary(id = "ws-pane", repository = "repo"),
        WorkspaceSummary(id = "ws-owner", repository = "repo"),
        WorkspaceSummary(id = "ws-held", repository = "repo"),
    )

    /** One pane's chip, composed, over a runner whose boards are [onRunner]. */
    private inner class Bar(scope: TestScope, private val onRunner: Map<String, TaskBoard>) {
        var boards by mutableStateOf(emptyMap<String, TaskBoard>())
        val reads = mutableListOf<String>()
        val routes = mutableListOf<Route>()
        var chip: TaskChip? = null
        private val frames = BroadcastFrameClock()
        private val recomposer = Recomposer(scope.backgroundScope.coroutineContext + frames)
        private val composition = Composition(NoNodes(), recomposer)
        private val scope = scope

        init {
            scope.backgroundScope.launch(frames) { recomposer.runRecomposeAndApplyChanges() }
            scope.runCurrent()
            composition.setContent {
                chip = rememberTaskChip(
                    hostId = "studio",
                    terminal = pane,
                    worktree = worktree,
                    boards = boards,
                    boardsNow = { boards },
                    boardList = { list },
                    readBoard = { ws ->
                        reads += ws.id
                        onRunner[ws.id]?.let { boards = boards + (ws.id to it) }
                    },
                    navigate = { routes += it },
                )
            }
            settle()
        }

        fun settle() {
            repeat(4) {
                scope.runCurrent()
                Snapshot.sendApplyNotifications()
                scope.runCurrent()
                frames.sendFrame(scope.testScheduler.currentTime * 1_000_000)
                scope.runCurrent()
            }
        }

        fun dispose() {
            composition.dispose()
            recomposer.cancel()
        }
    }

    private fun bar(onRunner: Map<String, TaskBoard>, body: (Bar) -> Unit) = runTest(main) {
        val bar = Bar(this, onRunner)
        try {
            body(bar)
        } finally {
            bar.dispose()
        }
    }

    private fun holding(vararg rows: TaskRow) = TaskBoard(listOf(TaskBoardColumn(TaskStatus.IN_PROGRESS, rows.toList())))

    @Test
    fun `the chip reads the task's board, then opens its card`() = bar(
        mapOf("ws-pane" to holding(), "ws-owner" to holding(card), "ws-held" to holding()),
    ) { bar ->
        assertEquals("read until a board held it, and no further", listOf("ws-pane", "ws-owner"), bar.reads)
        val chip = bar.chip!!
        assertEquals(invoice, chip.task)
        chip.open()
        assertEquals(listOf<Route>(Route.BoardTask("studio", "ws-owner", "t-9")), bar.routes)
    }

    @Test
    fun `a task no board holds gets no chip`() = bar(
        mapOf("ws-pane" to holding(), "ws-owner" to holding(), "ws-held" to holding()),
    ) { bar ->
        assertEquals(listOf("ws-pane", "ws-owner", "ws-held"), bar.reads)
        assertNull(bar.chip)
    }
}
