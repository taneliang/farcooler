package com.farcooler.capture

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.HorizontalDivider
import androidx.compose.ui.Modifier
import com.farcooler.model.HarnessAvailability
import com.farcooler.model.LfsNotice
import com.farcooler.model.RunnerCount
import com.farcooler.model.RunnerLink
import com.farcooler.model.idleWithoutOrchestrator
import com.farcooler.model.reassurance
import com.farcooler.ui.SeatAction
import com.farcooler.model.TaskAcceptanceLine
import com.farcooler.model.TaskBoard
import com.farcooler.model.TaskBoardColumn
import com.farcooler.model.TaskRow
import com.farcooler.model.TaskStatus
import com.farcooler.model.WorktreeScope
import com.farcooler.model.WorkspaceSummary
import com.farcooler.ui.BoardBlank
import com.farcooler.ui.BoardList
import com.farcooler.ui.BoardListEntry
import com.farcooler.ui.LfsNoticeCard
import com.farcooler.ui.NoRepositories
import com.farcooler.ui.OrchestratorEmpty
import com.farcooler.ui.OrchestratorSeat
import com.farcooler.ui.ReassuranceBlock
import com.farcooler.ui.SectionHeader
import com.farcooler.ui.TaskCardRow
import com.farcooler.ui.WorktreesEmpty
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * One capture per screen ov-245 and ov-15 item 6 name, at a Pixel-sized phone,
 * built from fake values and no connection. Run only under
 * `-Pfarcooler.captures` (see `app/build.gradle.kts`).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class ScreenCaptureTest {
    @Test fun workspaceNoOrchestrator() = Capture.both("workspace-no-orchestrator") {
        OrchestratorEmpty(
            seat = OrchestratorSeat.Empty(canStart = true),
            actions = setOf(SeatAction.START),
            refusal = null,
            availability = HarnessAvailability(null),
            runner = "studio-mac",
            missing = null,
            onStart = {},
            onReplace = {},
            onRestart = {},
        )
    }

    @Test fun needsYouNoAgentsWorking() = Capture.both("needs-you-no-agents-working") {
        // What the screen computes, not a fake sentence: with no orchestrator it
        // leads with "No agents are working yet." (ov-266).
        val idle = listOf(RunnerCount(RunnerLink.ANSWERING, 0, true))
        ReassuranceBlock(
            detail = reassurance(idle, " on studio-mac", worktrees = 1, noOrchestrator = true),
            showsAgentRows = idleWithoutOrchestrator(idle, noOrchestrator = true),
            caveat = null,
        )
    }

    @Test fun needsYouNoRepositories() = Capture.both("needs-you-no-repositories") {
        NoRepositories(runner = "studio-mac", canAdd = true, onAdd = {})
    }

    @Test fun worktreesEmpty() = Capture.both("worktrees-empty") {
        WorktreesEmpty(WorktreeScope.OfWorkspace("h", workspace))
    }

    @Test fun boardBlankWithOrchestrator() = Capture.both("board-blank-orchestrator-running") {
        BoardBlank(led = true, orchestratorRunning = true, onShowOrchestrator = {})
    }

    @Test fun boardBlankNoOrchestrator() = Capture.both("board-blank-no-orchestrator") {
        BoardBlank(led = true, orchestratorRunning = false, onShowOrchestrator = {})
    }

    @Test fun boardBlankImplicit() = Capture.both("board-blank-implicit-workspace") {
        BoardBlank(led = false, orchestratorRunning = false, onShowOrchestrator = null)
    }

    /** ov-15 item 6: the task board, as `BoardTab` lists it, from fake rows. */
    @Test fun taskBoard() = Capture.both("task-board") {
        val now = System.currentTimeMillis()
        fun task(n: Int, status: TaskStatus, title: String, lines: List<Boolean> = emptyList(), hoursAgo: Int = 1) =
            TaskRow(
                id = "t$n", key = "ov-$n", title = title, status = status,
                statusSince = now - hoursAgo * 3_600_000L,
                acceptance = lines.mapIndexed { i, met -> TaskAcceptanceLine("a$i", "Line $i", met) },
            )
        val board = TaskBoard(
            listOf(
                TaskBoardColumn(TaskStatus.NEEDS_DECISION, listOf(task(251, TaskStatus.NEEDS_DECISION, "Relay: which push provider carries the watch", hoursAgo = 5))),
                TaskBoardColumn(TaskStatus.IN_PROGRESS, listOf(
                    task(245, TaskStatus.IN_PROGRESS, "Phones: empty states are scannable rows", listOf(true, false)),
                    task(238, TaskStatus.IN_PROGRESS, "Android: first run explains the orchestrator", listOf(true, true, false), hoursAgo = 30),
                )),
                TaskBoardColumn(TaskStatus.IN_REVIEW, listOf(task(241, TaskStatus.IN_REVIEW, "iOS: the board's glance has a review rung", listOf(true, true)))),
                TaskBoardColumn(TaskStatus.TODO, listOf(task(260, TaskStatus.TODO, "Docs: how pairing works, in one page"))),
                TaskBoardColumn(TaskStatus.DONE, listOf(task(200, TaskStatus.DONE, "Mac: sidebar rows"))),
            )
        )
        LazyColumn(Modifier.fillMaxSize()) {
            for (entry in BoardList.entries(board, toggled = emptySet())) {
                when (entry) {
                    is BoardListEntry.Header -> item(key = entry.key) { SectionHeader(entry) {} }
                    is BoardListEntry.Card -> item(key = entry.key) {
                        TaskCardRow(
                            row = entry.row, agents = emptyList(), orchestrator = null, speaks = false,
                            presence = entry.row.agentPresence(0, false), onOpen = {}, onJump = {},
                        )
                        HorizontalDivider()
                    }
                    else -> Unit
                }
            }
        }
    }

    /** ov-199: the worktree's Changes says large files weren't downloaded, with Try again. */
    @Test fun changesLfsNotice() = Capture.both("changes-lfs-notice") {
        LfsNoticeCard(LfsNotice(pointers = 2, canRetry = true), onRetry = {})
    }

    /** ov-199: a read grant sees the sentence and no button. */
    @Test fun changesLfsNoticeReadGrant() = Capture.both("changes-lfs-notice-read-grant") {
        LfsNoticeCard(LfsNotice(pointers = 2, canRetry = false), onRetry = {})
    }

    private val workspace = WorkspaceSummary(id = "w1", name = "overnight", repository = "r1")
}
