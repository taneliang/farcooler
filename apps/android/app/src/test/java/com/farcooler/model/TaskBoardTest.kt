package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The board's rules on Android, which are AgentKit's rules transcribed
 * (`TaskBoardModelTests`, `TaskBoardAgentsTests`, `RunnerBoardsTests`). The
 * phone, the iPhone and the Mac must say the same thing about the same card,
 * and the only way to hold that on this side is to assert the same answers.
 */
class TaskBoardTest {
    private val task = "0198f2c0-0000-7000-8000-00000000a001"
    private val other = "0198f2c0-0000-7000-8000-00000000a002"

    private fun pane(
        taskId: String? = task,
        state: String = "running",
        preset: String = "claude",
        paneMode: String? = null,
        id: String = "p-${taskId}-$state-$preset",
    ) = Terminal(id = id, preset = preset, state = state, paneMode = paneMode, taskId = taskId)

    private fun row(
        id: String = task,
        status: TaskStatus = TaskStatus.IN_PROGRESS,
        acceptance: List<Boolean> = emptyList(),
        since: Long = 0L,
        createdAt: Long? = null,
        updatedAt: Long? = null,
    ) = TaskRow(
        id = id,
        key = "-$id",
        title = "A task",
        status = status,
        statusSince = since,
        acceptance = acceptance.mapIndexed { i, met -> TaskAcceptanceLine("a$i", "line $i", met) },
        createdAt = createdAt,
        updatedAt = updatedAt,
    )

    // ---- the wire ----

    /** Transcribed from `tasks_json::task_json`, snake_case and all. */
    private val listJson = """
        {"tasks": [
          {"id": "t1", "short": "a001", "repository_id": "r", "resource_version": 3,
           "key": "-19", "title": "Board in the sidebar", "status": "needs_decision",
           "status_since": 1000, "stale_for_seconds": 60,
           "created_at": 500, "updated_at": 900, "intent": "why",
           "acceptance": [{"id": "a1", "text": "one", "met": true},
                          {"id": "a2", "text": "two", "met": false}],
           "constraints": [], "labels": ["ios"], "workspace_id": "w1"},
          {"id": "t2", "key": "-20", "title": "Phones", "status": "in_progress",
           "status_since": 2000, "created_at": 0, "updated_at": 0,
           "acceptance": [], "labels": []},
          {"id": "t3", "key": "-9", "title": "From the future", "status": "parked"},
          {"id": "t4", "title": "No key, so not drawable", "status": "todo"}
        ]}
    """.trimIndent()

    @Test
    fun everyKeyTheBoardReadsLandsOnTheRow() {
        val board = TaskBoard.decode(listJson)
        val first = board.row("t1")!!
        assertEquals("-19", first.key)
        assertEquals(TaskStatus.NEEDS_DECISION, first.status)
        assertEquals(1000L, first.statusSince)
        assertEquals("why", first.intent)
        assertEquals(listOf("ios"), first.labels)
        assertEquals("w1", first.workspaceId)
        assertEquals(listOf(true, false), first.acceptance.map { it.met })
        assertEquals("two", first.acceptance[1].text)
        assertEquals(500L, first.createdAt)
        assertEquals(900L, first.updatedAt)
    }

    /**
     * A runner too old to say when: `null` (what `tasks_json` sends), an
     * older producer's zero, or no key at all. None of them is 1970.
     */
    @Test
    fun aClockTheRunnerDidNotSendIsNotTheEpoch() {
        val board = TaskBoard.decode(listJson)
        val second = board.row("t2")!!
        assertNull(second.createdAt)
        assertNull(second.updatedAt)
        assertNull(second.timeNote(3000))
        val bare = TaskBoard.decode(
            """{"tasks": [{"id": "b", "key": "-1", "title": "x", "status": "backlog"},
                          {"id": "n", "key": "-2", "title": "y", "status": "backlog",
                           "created_at": null, "updated_at": null}]}""",
        )
        for (id in listOf("b", "n")) {
            assertNull(id, bare.row(id)!!.createdAt)
            assertNull(id, bare.row(id)!!.updatedAt)
            assertNull(id, bare.row(id)!!.timeNote(3000))
        }
    }

    /** A status this build does not know is shown, not dropped; a row it cannot draw is skipped. */
    @Test
    fun anUnknownStatusIsKeptAndAnUndrawableRowIsSkipped() {
        val board = TaskBoard.decode(listJson)
        assertEquals(listOf("parked"), board.unreadable.map { it.status })
        assertNull(board.row("t4"))
        assertEquals(2, board.rows.size)
    }

    // ---- order and counts ----

    /** Needs Decision first, then the order work moves, and only statuses with tasks. */
    @Test
    fun theBoardListsStatusesWithTasksNeedsDecisionFirst() {
        val board = TaskBoard.decode(listJson)
        assertEquals(listOf(TaskStatus.NEEDS_DECISION, TaskStatus.IN_PROGRESS), board.listed.map { it.status })
        assertEquals(1, board.waitingOnYou)
        assertTrue(TaskBoard.EMPTY.listed.isEmpty())
    }

    /** The whole order, not just its first two: Needs Decision, then the order work moves. */
    @Test
    fun theStatusOrderIsNeedsDecisionThenTheLifecycle() {
        assertEquals(
            listOf("needs_decision", "backlog", "todo", "in_progress", "in_review", "done", "cancelled"),
            TaskStatus.ORDER.map { it.wire },
        )
        assertEquals(
            listOf("Needs Decision", "Backlog", "To Do", "In Progress", "In Review", "Done", "Canceled"),
            TaskStatus.ORDER.map { it.title },
        )
        // A decoded board's columns come out in that order too.
        assertEquals(TaskStatus.ORDER, TaskBoard.decode(listJson).columns.map { it.status })
    }

    @Test
    fun theWaitingSentenceAgreesAndIsNothingAtZero() {
        assertNull(TaskBoard.waitingSentence(0))
        assertEquals("1 task is waiting on you", TaskBoard.waitingSentence(1))
        assertEquals("2 tasks are waiting on you", TaskBoard.waitingSentence(2))
    }

    // ---- acceptance ----

    @Test
    fun acceptanceReadsAsACountAndAsAllMetOnceItHolds() {
        assertNull(row(acceptance = emptyList()).acceptanceProgress)
        assertEquals("0 of 2", row(acceptance = listOf(false, false)).acceptanceProgress!!.sentence)
        assertEquals("2 of 5", row(acceptance = listOf(true, true, false, false, false)).acceptanceProgress!!.sentence)
        val all = row(acceptance = listOf(true, true, true, true, true)).acceptanceProgress!!
        assertEquals("All 5 met", all.sentence)
        assertTrue(all.isComplete)
        assertEquals("Met", row(acceptance = listOf(true)).acceptanceProgress!!.sentence)
        assertFalse(row(acceptance = listOf(true, false)).acceptanceProgress!!.isComplete)
    }

    // ---- staleness and the ask ----

    /**
     * Only In Progress and In Review go stale: the two where an agent is meant
     * to be working (ov-28). Each status a week old; a rule written back as
     * "every unfinished status" turns Backlog, To Do and Needs Decision red.
     */
    @Test
    fun onlyActiveWorkGoesStale() {
        val week = 7 * TaskRow.DAY_MS
        for (status in TaskStatus.entries) {
            val expected = status == TaskStatus.IN_PROGRESS || status == TaskStatus.IN_REVIEW
            val card = row(status = status, since = 0)
            assertEquals("$status", expected, card.isStale(week))
            assertEquals("$status", expected, card.stalenessNote(week) != null)
        }
    }

    // ---- the time line ----

    @Test
    fun aCardNothingHasHappenedToSaysWhenItWasAdded() {
        val day = TaskRow.DAY_MS
        val card = row(status = TaskStatus.BACKLOG, since = 0, createdAt = 0, updatedAt = 0)
        assertEquals("Added 3d ago", card.timeNote(3 * day))
    }

    @Test
    fun aCardThatChangedSaysWhenItWasUpdated() {
        val hour = 3_600_000L
        val card = row(status = TaskStatus.TODO, since = 0, createdAt = 1, updatedAt = 10 * hour)
        assertEquals("Updated 2h ago", card.timeNote(12 * hour + 60_000))
    }

    @Test
    fun aCardWithOnlyOneClockSaysWhatItCan() {
        // `updated_at` alone cannot tell an update from a creation.
        assertNull(row(status = TaskStatus.BACKLOG, updatedAt = 5).timeNote(10))
        assertEquals("Added 1m ago", row(status = TaskStatus.BACKLOG, createdAt = 0).timeNote(90_000))
    }

    /** The stale sentence already says the time; the two lines are never both drawn. */
    @Test
    fun aStaleCardSaysHasntMovedInsteadOfItsTime() {
        val day = TaskRow.DAY_MS
        val stuck = row(status = TaskStatus.IN_PROGRESS, since = 0, createdAt = 0, updatedAt = 0)
        assertEquals("Hasn’t moved in 3 days", stuck.stalenessNote(3 * day))
        assertNull(stuck.timeNote(3 * day))
        val waiting = row(status = TaskStatus.BACKLOG, since = 0, createdAt = 0, updatedAt = 0)
        assertNull(waiting.stalenessNote(3 * day))
        assertEquals("Added 3d ago", waiting.timeNote(3 * day))
    }

    /** AgentKit's `theRelativeTimeIsShortAndNeverRoundsUp`, value for value. */
    @Test
    fun theRelativeTimeIsShortAndNeverRoundsUp() {
        val m = 60_000L
        val h = 60 * m
        val d = TaskRow.DAY_MS
        assertEquals("just now", TaskRow.ago(-30_000))
        assertEquals("just now", TaskRow.ago(0))
        assertEquals("just now", TaskRow.ago(59_000))
        assertEquals("1m ago", TaskRow.ago(m))
        assertEquals("59m ago", TaskRow.ago(h - 1_000))
        assertEquals("1h ago", TaskRow.ago(h))
        assertEquals("1h ago", TaskRow.ago(2 * h - 1_000))
        assertEquals("23h ago", TaskRow.ago(d - 1_000))
        assertEquals("1d ago", TaskRow.ago(d))
        assertEquals("29d ago", TaskRow.ago(30 * d - 1_000))
        assertEquals("1mo ago", TaskRow.ago(30 * d))
        assertEquals("12mo ago", TaskRow.ago(365 * d - 1_000))
        assertEquals("1y ago", TaskRow.ago(365 * d))
        assertEquals("2y ago", TaskRow.ago(800 * d))
    }

    @Test
    fun aCardThatStoppedMovingSaysSoButAFinishedOneDoesNot() {
        val day = TaskRow.DAY_MS
        assertNull(row(since = 0).stalenessNote(day - 1))
        assertEquals("Hasn’t moved in a day", row(since = 0).stalenessNote(day))
        assertEquals("Hasn’t moved in 3 days", row(since = 0).stalenessNote(3 * day))
        assertNull(row(status = TaskStatus.DONE, since = 0).stalenessNote(30 * day))
        // A runner whose clock is ahead of this one has not moved a task in the future.
        assertNull(row(since = 10 * day).stalenessNote(0))
    }

    @Test
    fun onlyNeedsDecisionAsksAnything() {
        for (status in TaskStatus.entries) {
            val ask = row(status = status).callToAction
            if (status == TaskStatus.NEEDS_DECISION) assertEquals("Answer to unblock this", ask)
            else assertNull("$status", ask)
        }
    }

    // ---- which pane is working which card ----

    @Test
    fun aPaneWorksItsTaskWhileItIsRunningStartingOrUnknown() {
        for (state in listOf("running", "starting", "unknown")) {
            assertTrue(state, TaskAgentLink.isWorking(pane(state = state), task))
        }
        for (state in listOf("exited", "error", "LOST", "")) {
            assertFalse(state, TaskAgentLink.isWorking(pane(state = state), task))
        }
        assertFalse(TaskAgentLink.isWorking(pane(taskId = other), task))
        assertFalse(TaskAgentLink.isWorking(pane(taskId = null), task))
        assertFalse(TaskAgentLink.isWorking(pane(taskId = ""), ""))
    }

    /** A shell in any spelling is not an agent, nor is a changes pane; the Mac's and iPhone's rule. */
    @Test
    fun anAgentIsNeitherAShellNorAChangesPane() {
        for (preset in listOf("claude", "codex:gpt-5", "cursor", "aider")) {
            assertTrue(preset, TaskAgentLink.runsAgent(preset, isChangesPane = false))
        }
        for (preset in listOf("shell", "zsh", "-zsh", "fish", "bash", "sh", "dash", "ksh", "ZSH", "")) {
            assertFalse(preset, TaskAgentLink.runsAgent(preset, isChangesPane = false))
        }
        assertFalse(TaskAgentLink.runsAgent("claude", isChangesPane = true))
        // The first non-empty piece names it, as Swift's split does.
        assertTrue(TaskAgentLink.runsAgent(":claude", isChangesPane = false))
        assertFalse(TaskAgentLink.runsAgent(":zsh", isChangesPane = false))
        assertFalse(TaskAgentLink.isWorking(pane(preset = "farcooler", paneMode = "changes"), task))
        assertFalse(TaskAgentLink.isWorking(pane(preset = "zsh"), task))
    }

    @Test
    fun aBoardSpeaksOfAgentsOnlyOnAConnectedRunnerThatRecordsThem() {
        val records = DaemonBuild("1", true, "", setOf("tasks", "terminal_task"))
        val doesNot = DaemonBuild("1", true, "", setOf("tasks"))
        assertTrue(TaskAgentLink.speaksOfAgents(true, records))
        assertFalse(TaskAgentLink.speaksOfAgents(false, records))
        assertFalse(TaskAgentLink.speaksOfAgents(true, doesNot))
        assertFalse(TaskAgentLink.speaksOfAgents(true, null))
    }

    @Test
    fun aCardSaysAgentsWhateverItsStatusAndNoAgentOnlyInProgress() {
        for (status in TaskStatus.entries) {
            assertEquals(TaskAgentPresence.Agents(1), row(status = status).agentPresence(1, true))
            val none = row(status = status).agentPresence(0, true)
            if (status == TaskStatus.IN_PROGRESS) assertEquals(TaskAgentPresence.NoAgent, none)
            else assertEquals("$status", TaskAgentPresence.Unsaid, none)
        }
        // A runner that cannot say says nothing, agents or not.
        assertEquals(TaskAgentPresence.Unsaid, row().agentPresence(2, false))
        assertEquals("Agent", TaskAgentPresence.Agents(1).title)
        assertEquals("2 agents", TaskAgentPresence.Agents(2).title)
        assertEquals("No agent", TaskAgentPresence.NoAgent.title)
        assertNull(TaskAgentPresence.Unsaid.title)
    }

    @Test
    fun menuItemsThatShareATitleGetTheirShortIds() {
        assertEquals(
            listOf("claude in lane (aaa1)", "claude in lane (bbb2)", "codex in lane"),
            TaskAgentLink.menuTitles(
                listOf("claude in lane", "claude in lane", "codex in lane"),
                listOf("aaa1", "bbb2", "ccc3"),
            ),
        )
    }

    // ---- the front door's rows ----

    private val both = DaemonBuild("1", true, "", setOf("workspaces", "tasks", "terminal_task"))
    private val repositories = listOf(
        Repository("r-busy", displayName = "overnight"),
        Repository("r-empty", displayName = "scratch"),
        Repository("r-unread", displayName = "never-read"),
    )
    private val busy = TaskBoard(
        TaskStatus.ORDER.map { status ->
            TaskBoardColumn(
                status,
                when (status) {
                    TaskStatus.NEEDS_DECISION -> listOf(row("1", status), row("2", status))
                    TaskStatus.IN_PROGRESS -> listOf(row("3", status), row("4", status))
                    else -> emptyList()
                },
            )
        },
    )
    private val boards = mapOf("r-busy" to busy, "r-empty" to TaskBoard.EMPTY)

    /** Two agents on task 3 and one on task 4: two tasks moving, three panes. */
    private val panes = listOf(
        pane(taskId = "3", id = "x"), pane(taskId = "3", id = "y"), pane(taskId = "4", id = "z"),
        pane(taskId = "1", state = "exited", id = "gone"),
    )

    @Test
    fun onlyARepositoryWithSomethingOnItsBoardGetsARow() {
        val rows = RunnerBoards.rows("h", repositories, boards, panes, both, connected = true)
        assertEquals(listOf("r-busy"), rows.map { it.repository })
        assertEquals("overnight", rows.single().name)
    }

    @Test
    fun aRowCountsDecisionsAndTheTasksAgentsAreOn() {
        val row = RunnerBoards.rows("h", repositories, boards, panes, both, connected = true).single()
        assertEquals(2, row.decisions)
        assertEquals(2, row.agents)
        assertEquals("2 tasks need a decision, Agents are on 2 tasks", row.spoken)
    }

    @Test
    fun aRunnerThatCannotBeBelievedAboutItsPanesCountsNoAgents() {
        val dropped = RunnerBoards.rows("h", repositories, boards, panes, both, connected = false).single()
        assertEquals(0, dropped.agents)
        assertEquals(2, dropped.decisions)
        val older = DaemonBuild("1", true, "", setOf("workspaces", "tasks"))
        assertEquals(0, RunnerBoards.rows("h", repositories, boards, panes, older, connected = true).single().agents)
    }

    @Test
    fun aRunnerWithoutABoardGetsNoRows() {
        val old = DaemonBuild("1", true, "", setOf("workspaces"))
        assertTrue(RunnerBoards.rows("h", repositories, boards, panes, old, true).isEmpty())
        assertTrue(RunnerBoards.rows("h", repositories, boards, panes, null, true).isEmpty())
        // Silence is a runner older than capabilities, which has no board either.
        assertTrue(RunnerBoards.rows("h", repositories, boards, panes, DaemonBuild("1", true, ""), true).isEmpty())
    }

    /** A board whose only rows this build cannot place still has something on it, and a row. */
    @Test
    fun aBoardOfOnlyUnreadableRowsStillGetsARow() {
        val future = TaskBoard(
            TaskStatus.ORDER.map { TaskBoardColumn(it, emptyList()) },
            unreadable = listOf(UnreadableTaskRow("9", "-9", "x", "parked")),
        )
        val rows = RunnerBoards.rows(
            "h", listOf(Repository("r-new", displayName = "newer")), mapOf("r-new" to future),
            panes, both, connected = true,
        )
        assertEquals(listOf("r-new"), rows.map { it.repository })
        assertEquals(0, rows.single().decisions)
    }

    @Test
    fun aRowWithNothingToCountSaysNothing() {
        assertNull(BoardRow("h", "r", "n", 0, 0).spoken)
        assertEquals("1 task needs a decision", BoardRow("h", "r", "n", 1, 0).spoken)
    }

    // ---- a late build, and landing ----

    @Test
    fun aBuildThatLandsLateReadsTheBoardsItsLinkNeverRead() {
        val sweep = BoardSweep()
        sweep.linkCameUp()
        assertTrue(sweep.owedWhenBuildLands)
        sweep.swept()
        assertFalse(sweep.owedWhenBuildLands)
        sweep.linkCameUp()
        assertTrue(sweep.owedWhenBuildLands)
    }

    @Test
    fun anAgentLandsOnItsWorkspaceOrNowhereIfItsPaneHasClosed() {
        val workspaces = listOf(
            Workspace(id = "w1", task = "one", terminals = listOf(pane(id = "t1"))),
            Workspace(id = "w2", task = "two", terminals = listOf(pane(id = "t2"))),
        )
        assertEquals("w2", landingWorkspace("t2", workspaces))
        assertNull(landingWorkspace("gone", workspaces))
    }
}
