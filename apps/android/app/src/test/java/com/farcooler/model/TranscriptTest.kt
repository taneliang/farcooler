package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The Swift suite in `apps/shared/AgentKit/Tests/AgentKitTests/
 * TranscriptTests.swift`, translated case for case.
 *
 * Translated rather than summarised on purpose. The reducer is the one piece of
 * this app that MUST agree with the Mac and iOS bit for bit — two clients that
 * fold the same events differently disagree about one conversation, which is
 * the exact failure the daemon-side derivation model exists to prevent. Two
 * suites that merely test "the same sort of thing" would not catch that; two
 * suites asserting the same outcomes do.
 */
class TranscriptTest {
    private fun seq(n: Long, event: AgentEvent) = Sequenced(n, event)

    private fun message(text: String, role: Role = Role.AGENT, parent: String? = null) =
        AgentEvent.Message(role, text, parent)

    private fun toolCall(
        id: String,
        title: String = "Tool",
        status: ToolStatus = ToolStatus.PENDING,
        parent: String? = null,
        subagent: Boolean = false,
    ) = AgentEvent.ToolCall(id, title, "kind", status, emptyList(), parent, subagent)

    private fun toolUpdate(
        id: String,
        status: ToolStatus = ToolStatus.COMPLETED,
        title: String? = null,
        content: String? = null,
        diff: Diff? = null,
        parent: String? = null,
        subagent: SubagentSummary? = null,
    ) = AgentEvent.ToolUpdate(id, status, title, content, diff, emptyList(), parent, subagent)

    private val TranscriptRow.messageText: String?
        get() = (kind as? TranscriptRow.Kind.Message)?.text

    private val TranscriptRow.tool: ToolRow?
        get() = (kind as? TranscriptRow.Kind.Tool)?.tool

    private val TranscriptRow.block: SubagentBlock?
        get() = (kind as? TranscriptRow.Kind.Subagent)?.block

    @Test
    fun consecutiveChunksOfOneMessageBecomeOneRow() {
        // The agent streams a sentence as many chunks. One row per chunk would
        // render a column of one-word paragraphs.
        val t = Transcript()
        t.apply(listOf(seq(0, message("Hello, ")), seq(1, message("world"))))
        assertEquals(1, t.rows.size)
        val row = t.rows[0].kind as TranscriptRow.Kind.Message
        assertEquals(Role.AGENT, row.role)
        assertEquals("Hello, world", row.text)
    }

    @Test
    fun aRoleChangeStartsANewRow() {
        val t = Transcript()
        t.apply(listOf(seq(0, message("a")), seq(1, message("b", role = Role.THOUGHT))))
        assertEquals(2, t.rows.size)
    }

    @Test
    fun aToolUpdateMutatesItsCallRatherThanAppending() {
        // A tool that reports progress four times must stay one row, or the
        // transcript fills with duplicates of the same call.
        val t = Transcript()
        t.apply(listOf(seq(0, toolCall("t1", title = "Edit main.rs")), seq(1, toolUpdate("t1"))))
        assertEquals(1, t.rows.size)
        assertEquals(ToolStatus.COMPLETED, t.rows[0].tool?.status)
    }

    @Test
    fun aDiffAttachesToTheToolItBelongsTo() {
        val t = Transcript()
        val diff = Diff("main.rs", "old", "new")
        t.apply(listOf(seq(0, toolCall("t1", title = "Edit")), seq(1, toolUpdate("t1", diff = diff))))
        assertEquals(diff, t.rows[0].tool?.diff)
    }

    @Test
    fun aGapIsItsOwnRowAndIsNeverMergedAway() {
        // If a gap could merge into a neighbouring message the user would never
        // learn that history is missing, which is the one thing this design
        // promises never to hide.
        val t = Transcript()
        t.apply(
            listOf(
                seq(0, message("a")),
                seq(1, AgentEvent.Gap(GapReason.RingTrimmed)),
                seq(2, message("b")),
            )
        )
        assertEquals(3, t.rows.size)
        assertTrue(t.rows[1].kind is TranscriptRow.Kind.Gap)
    }

    @Test
    fun aPendingPermissionIsExposedAndClearedWhenAnswered() {
        val t = Transcript()
        val options = listOf(PermissionOption("allow", "Allow", "allow_once"))
        t.apply(listOf(seq(0, AgentEvent.Permission("r1", "t1", options))))
        assertEquals("r1", t.pendingPermission?.id)

        t.apply(listOf(seq(1, AgentEvent.Resolved("r1", "allow"))))
        assertNull(t.pendingPermission)
    }

    @Test
    fun thePlanIsReplacedWholesaleNotAppended() {
        // The daemon sends the whole plan every time, so appending would show
        // every historical version of the todo list stacked up.
        val t = Transcript()
        t.apply(listOf(seq(0, AgentEvent.Plan(listOf(PlanEntry("one", "high", "pending"))))))
        t.apply(listOf(seq(1, AgentEvent.Plan(listOf(PlanEntry("two", "high", "done"))))))
        assertEquals(1, t.plan.size)
        assertEquals("two", t.plan[0].content)
    }

    @Test
    fun theCursorTracksTheHighestSeqSeen() {
        // Reconnect asks for everything after this. An off-by-one repeats a
        // message or skips one.
        val t = Transcript()
        t.apply(listOf(seq(0, message("a")), seq(7, message("b", role = Role.USER))))
        assertEquals(8L, t.cursor)
    }

    @Test
    fun aTranscriptWithOnlyAGapStillHasSomethingToDraw() {
        // The empty-state and the gap-state are different. Rendering "no
        // messages yet" over a gap would tell the user nothing happened when in
        // fact something happened and was lost.
        val t = Transcript()
        t.apply(listOf(seq(0, AgentEvent.Gap(GapReason.LoadUnsupported))))
        assertEquals(1, t.rows.size)
    }

    @Test
    fun commandsArriveOnTheirOwnEventAndReplaceRatherThanAccumulate() {
        // The agent resends its whole menu every turn. Appending would grow the
        // picker without bound, showing every command several times over.
        val menu = listOf(
            AgentChoice("init", "init", "Set up a project"),
            AgentChoice("review", "review", "Review the diff"),
        )
        val t = Transcript()
        t.apply(listOf(seq(0, AgentEvent.CommandsAvailable(menu))))
        t.apply(listOf(seq(1, AgentEvent.CommandsAvailable(menu))))
        assertEquals(menu, t.availableCommands)

        // And it is not a gap: nothing was lost, so nothing should be drawn.
        assertTrue(t.rows.isEmpty())
    }

    @Test
    fun aNewTurnStartsANewRowRatherThanJoiningTheLastReply() {
        // Two turns' replies are both agent and adjacent, so plain coalescing
        // glues them into one paragraph — "…prints hi to the console.Hello! I'm
        // Claude Code…" — with no seam where a whole turn began.
        val t = Transcript()
        t.apply(
            listOf(
                seq(0, message("First reply.")),
                seq(1, AgentEvent.TurnEnded("EndTurn")),
                seq(2, message("Second reply.")),
            )
        )
        assertEquals(2, t.rows.size)
    }

    @Test
    fun askingTheSameThingTwiceMakesTwoRowsWithDifferentIds() {
        // Identity was derived from content, so a repeated question produced
        // two rows sharing an id. A keyed list over duplicate ids does not
        // merely look odd: it renders blank bands and repeats rows in the wrong
        // places, which is exactly what a user sees when they scroll back.
        val t = Transcript()
        t.appendLocalUserMessage("what does main.rs do?")
        t.apply(listOf(seq(0, AgentEvent.TurnEnded("EndTurn"))))
        t.appendLocalUserMessage("what does main.rs do?")

        assertEquals(2, t.rows.size)
        assertNotEquals(t.rows[0].id, t.rows[1].id)
        assertEquals(t.rows.size, t.rows.map { it.id }.toSet().size)
    }

    @Test
    fun aSentMessageAppearsImmediatelyRatherThanVanishing() {
        // The adapter echoes user text only when replaying a loaded session,
        // never during a live turn. Without a local echo the message disappears
        // the instant it is sent.
        val t = Transcript()
        t.appendLocalUserMessage("hello")
        val row = t.rows.first().kind as TranscriptRow.Kind.Message
        assertEquals(Role.USER, row.role)
        assertEquals("hello", row.text)
    }

    @Test
    fun aSessionTitleIsCarriedAndDrawsNoRow() {
        // It arrived unmodelled and became a Gap, which drew a "history
        // missing" break at the end of every turn for a title nobody had asked
        // for.
        val t = Transcript()
        t.apply(listOf(seq(0, AgentEvent.SessionInfo("Say ok."))))
        assertEquals("Say ok.", t.title)
        assertTrue(t.rows.isEmpty())
    }

    @Test
    fun contextUsageIsCarriedAndDrawsNoRow() {
        // Resent constantly as a turn burns context; one line of transcript per
        // report would bury the conversation.
        val t = Transcript()
        t.apply(listOf(seq(0, AgentEvent.Usage(50_000, 200_000))))
        assertEquals(0.25, t.contextFraction!!, 0.0001)
        assertTrue(t.rows.isEmpty())
    }

    @Test
    fun aToolCallIsRenamedAsItResolves() {
        // A Bash call starts life titled "Terminal" and is renamed to the
        // command it ran; a Read starts as "Read File" and becomes the file.
        // Keeping the placeholder left every row describing a category instead
        // of an action.
        val t = Transcript()
        t.apply(
            listOf(
                seq(0, toolCall("t1", title = "Terminal")),
                seq(1, toolUpdate("t1", title = "echo hello && ls", content = "hello\nmain.rs")),
            )
        )
        val tool = t.rows.first().tool!!
        assertEquals("echo hello && ls", tool.title)
        assertEquals("hello\nmain.rs", tool.content)
    }

    @Test
    fun choosingAConfigOptionShowsImmediately() {
        // The adapter applies `session/set_config_option` and says nothing
        // back, so a picker that waited for confirmation snapped to its old
        // value and read as a control that does nothing.
        val t = Transcript()
        t.apply(
            listOf(
                seq(
                    0,
                    AgentEvent.SessionStarted(
                        sessionId = "s",
                        agentMode = "default",
                        availableModes = emptyList(),
                        model = "haiku",
                        availableModels = emptyList(),
                        configOptions = listOf(
                            ConfigOption(
                                id = "model",
                                name = "Model",
                                description = "",
                                category = "model",
                                kind = "select",
                                currentValue = "haiku",
                                options = listOf(AgentChoice("opus", "Opus")),
                            )
                        ),
                        availableCommands = emptyList(),
                    ),
                )
            )
        )

        t.selectConfigOptionLocally("model", "opus")
        assertEquals("opus", t.configOptions.first().currentValue)
        assertEquals("opus", t.model)
    }

    @Test
    fun aRedeliveredBatchDoesNotRenderTheConversationTwice() {
        // The daemon numbers by position within an epoch, so a batch that
        // arrives again carries numbers already applied. It happens whenever a
        // delivery races a replay — which is exactly what a pane toggle does —
        // and applying it a second time put every message on screen twice.
        val t = Transcript()
        val batch = listOf(
            seq(0, message("hello", role = Role.USER)),
            seq(1, AgentEvent.TurnEnded("end_turn")),
            seq(2, message("hi")),
        )
        t.apply(batch)
        t.apply(batch)

        assertEquals(listOf("hello", "hi"), t.rows.mapNotNull { it.messageText })
    }

    @Test
    fun answeringAPermissionTakesTheCardDown() {
        // The agent resumes without acknowledging the request it was blocked
        // on, so a card that waited for a `Resolved` stayed up after the work
        // it was gating had already run.
        val t = Transcript()
        t.apply(
            listOf(
                seq(
                    0,
                    AgentEvent.Permission(
                        "p1",
                        "bash",
                        listOf(PermissionOption("allow", "Allow", "allow_once")),
                    ),
                )
            )
        )
        assertNotNull(t.pendingPermission)

        t.clearPendingPermission()
        assertNull(t.pendingPermission)
    }

    @Test
    fun aSubagentsWorkLivesInsideTheCallThatDispatchedIt() {
        // Before this, the subagent's message and its Read rendered as siblings
        // of the Task row, and the transcript claimed the top-level agent had
        // read the file itself.
        val t = Transcript()
        t.apply(
            listOf(
                seq(0, toolCall("task1", "Task", ToolStatus.IN_PROGRESS, subagent = true)),
                seq(1, message("I'll read the file.", parent = "task1")),
                seq(2, toolCall("r1", "Read main.rs", ToolStatus.COMPLETED, parent = "task1")),
            )
        )
        assertEquals(1, t.rows.size)
        val block = t.rows[0].block!!
        assertEquals(2, block.children.size)
        assertEquals("I'll read the file.", block.children[0].messageText)
    }

    @Test
    fun aToolInsideABlockIsUpdatedInPlaceRatherThanDuplicated() {
        // The flat lookup found nothing for a nested tool and appended a
        // second, half-built row beside the block — one tool rendering as two.
        val t = Transcript()
        t.apply(
            listOf(
                seq(0, toolCall("task1", "Task", ToolStatus.IN_PROGRESS, subagent = true)),
                seq(1, toolCall("r1", "Read", parent = "task1")),
                seq(
                    2,
                    toolUpdate("r1", title = "Read main.rs", content = "3 lines", parent = "task1"),
                ),
            )
        )
        assertEquals(1, t.rows.size)
        val block = t.rows[0].block!!
        assertEquals(1, block.children.size)
        val tool = block.children[0].tool!!
        assertEquals(ToolStatus.COMPLETED, tool.status)
        assertEquals("Read main.rs", tool.title)
    }

    @Test
    fun twoSubagentsRunningAtOnceDoNotBleedIntoEachOther() {
        // Frames from parallel subagents interleave in one stream. Routing by
        // "most recent block" instead of by parent id would file each line
        // under whichever subagent happened to speak last.
        val t = Transcript()
        t.apply(
            listOf(
                seq(0, toolCall("a", "Task", ToolStatus.IN_PROGRESS, subagent = true)),
                seq(1, toolCall("b", "Task", ToolStatus.IN_PROGRESS, subagent = true)),
                seq(2, message("from a", parent = "a")),
                seq(3, message("from b", parent = "b")),
                seq(4, message(" still a", parent = "a")),
            )
        )
        assertEquals(2, t.rows.size)
        assertEquals("from a still a", t.rows[0].block!!.children[0].messageText)
        assertEquals("from b", t.rows[1].block!!.children[0].messageText)
    }

    @Test
    fun aSubagentsWordsNeverJoinTheDispatchingAgentsSentence() {
        // Message chunks coalesce with the previous row. Coalescing across a
        // parent boundary would splice a subagent's sentence onto the end of
        // the agent's own, attributing it to the wrong speaker in the worst
        // way.
        val t = Transcript()
        t.apply(
            listOf(
                seq(0, message("I'll dispatch.")),
                seq(1, toolCall("task1", "Task", ToolStatus.IN_PROGRESS, subagent = true)),
                seq(2, message("inside", parent = "task1")),
            )
        )
        assertEquals(2, t.rows.size)
        assertEquals("I'll dispatch.", t.rows[0].messageText)
    }

    @Test
    fun aFrameWhoseParentWasNeverSeenIsShownRatherThanLost() {
        // A trimmed ring or a partial reload can leave a child with no block to
        // hang from. Rendering it at the top level is honest — nothing is
        // missing but the nesting. Dropping it would shorten the transcript
        // silently, and a gap would claim content was lost when none was.
        val t = Transcript()
        t.apply(listOf(seq(0, message("orphaned", parent = "never-seen"))))
        assertEquals(1, t.rows.size)
        assertEquals("orphaned", t.rows[0].messageText)
    }

    @Test
    fun anOrphanDoesNotJoinTheAgentsOwnSentence() {
        // Both land at the top level, so the container alone cannot keep them
        // apart — which is why the row carries its parent.
        val t = Transcript()
        t.apply(
            listOf(
                seq(0, message("mine.")),
                seq(1, message("theirs.", parent = "never-seen")),
            )
        )
        assertEquals(2, t.rows.size)
    }

    @Test
    fun aSubagentCutOffMidRunDoesNotRenderAsOneThatSucceeded() {
        // A cancelled turn never delivers the dispatch's completion. Leaving
        // the block alone would show a subagent whose fate nobody knows wearing
        // the same mark as one that reported back.
        val t = Transcript()
        t.apply(
            listOf(
                seq(0, toolCall("task1", "Task", ToolStatus.IN_PROGRESS, subagent = true)),
                seq(1, AgentEvent.TurnEnded("Cancelled")),
            )
        )
        assertTrue(t.rows[0].block!!.interrupted)
    }

    @Test
    fun aFinishedBlockCarriesItsSummary() {
        val t = Transcript()
        t.apply(
            listOf(
                seq(0, toolCall("task1", "Task", ToolStatus.IN_PROGRESS, subagent = true)),
                seq(
                    1,
                    toolUpdate(
                        "task1",
                        title = "Count lines",
                        subagent = SubagentSummary(
                            "general-purpose", "claude-opus-5[1m]", 12479, 1, 4962, "completed"
                        ),
                    ),
                ),
            )
        )
        val block = t.rows[0].block!!
        assertEquals(12479L, block.summary?.tokens)
        assertEquals(ToolStatus.COMPLETED, block.tool.status)
        assertFalse(block.interrupted)
    }

    @Test
    fun anInterruptedBlockStopsCountingAsRunning() {
        // Interruption does not change the tool's status — a cut-off subagent
        // stays IN_PROGRESS forever — so a surface asking the status alone
        // keeps the block auto-expanded and spinning for the rest of the
        // session.
        val t = Transcript()
        t.apply(
            listOf(
                seq(0, toolCall("task1", "Task", ToolStatus.IN_PROGRESS, subagent = true)),
                seq(1, AgentEvent.TurnEnded("Cancelled")),
            )
        )
        val block = t.rows[0].block!!
        assertTrue(block.interrupted)
        assertEquals(
            "interruption must not fake a terminal status",
            ToolStatus.IN_PROGRESS,
            block.tool.status,
        )
        assertFalse(block.isRunning)
    }

    @Test
    fun aLiveBlockCountsAsRunning() {
        val t = Transcript()
        t.apply(listOf(seq(0, toolCall("task1", "Task", ToolStatus.IN_PROGRESS, subagent = true))))
        assertTrue(t.rows[0].block!!.isRunning)
    }

    @Test
    fun aSubtitleCountsInEnglish() {
        // Both Apple apps wrote this independently and both said "1 tools". It
        // is one sentence of formatting, which is exactly the kind of thing
        // that gets duplicated and then diverges.
        val tool = ToolRow("t", "Task", "think", ToolStatus.COMPLETED, emptyList())
        val one = SubagentBlock(
            tool = tool,
            summary = SubagentSummary("general-purpose", "m", 12570, 1, 3600, "completed"),
        )
        assertEquals("general-purpose · 1 tool · 12k tok · 3.6s", one.subtitle)

        val many = SubagentBlock(
            tool = tool,
            summary = SubagentSummary("general-purpose", "m", 900, 8, 31200, "completed"),
        )
        assertEquals("general-purpose · 8 tools · 900 tok · 31.2s", many.subtitle)
    }

    @Test
    fun anInterruptedBlockSaysSoRatherThanReportingNumbers() {
        // An interrupted block can still be carrying a partial summary.
        // Reporting its token count would present a subagent whose outcome
        // nobody knows as one that finished and reported.
        val block = SubagentBlock(
            tool = ToolRow("t", "Task", "think", ToolStatus.IN_PROGRESS, emptyList()),
            summary = SubagentSummary("general-purpose", "m", 100, 2, 1000, "in_progress"),
            interrupted = true,
        )
        assertEquals("interrupted", block.subtitle)
    }

    @Test
    fun aRunningBlockCountsItsStepsInEnglish() {
        val one = SubagentBlock(
            tool = ToolRow("t", "Task", "think", ToolStatus.IN_PROGRESS, emptyList()),
            children = listOf(TranscriptRow(0, TranscriptRow.Kind.Gap(GapReason.Unparsed))),
        )
        assertEquals("1 step", one.subtitle)
    }

    @Test
    fun anEpochResetClearsTheConversationButKeepsTheSelectors() {
        // The cursor a transcript holds counts positions in a stream that no
        // longer exists, so the rows built from it describe a conversation that
        // is not the one being served. The selectors survive because they
        // describe the AGENT rather than the stream; clearing them would blank
        // the pickers on every pane toggle.
        val t = Transcript()
        t.apply(
            listOf(
                seq(
                    0,
                    AgentEvent.SessionStarted(
                        "s", "default", listOf(AgentChoice("default", "Manual")),
                        "haiku", emptyList(), emptyList(), emptyList(),
                    ),
                ),
                seq(1, message("something")),
            )
        )
        t.resetForNewEpoch()

        assertTrue(t.rows.isEmpty())
        assertEquals(0L, t.cursor)
        assertEquals(listOf(AgentChoice("default", "Manual")), t.availableModes)
    }
}
