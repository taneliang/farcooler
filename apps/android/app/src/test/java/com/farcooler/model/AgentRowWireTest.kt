package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The wire decoder, against the file the Rust boundary writes (AgentKit's `theSharedRowFixtureDecodes`). */
class AgentRowWireTest {
    @Test
    fun `the hint fixture decodes to claude's Try example on its own row`() {
        val page = AgentRowPage.decode(AgentRowFixture.hintPage)
        assertEquals(listOf(AgentRow.HINT_ID), page.rows.map { it.id })
        assertEquals(AgentRow.Kind.OfHint(AgentRow.Hint("Try \"how does <filepath> work?\"")), page.rows[0].kind)
    }

    @Test
    fun `the shared fixture decodes every row kind to the values it holds`() {
        val page = AgentRowPage.decode(AgentRowFixture.page)
        assertEquals(4L, page.epoch)
        assertEquals(9L, page.rev)
        assertTrue(page.moreBefore)
        assertEquals(
            listOf("turn:p1", "prose:1", "think:1", "tool:toolu_1", "sub:toolu_2", "ask:1", "queued:1", "notice:1", "handoff:1", "gap:1"),
            page.rows.map { it.id },
        )

        val turn = (page.rows[0].kind as AgentRow.Kind.OfTurn).turn
        assertEquals("Fix the build\nand the tests", turn.prompt)
        assertEquals("Typed", turn.origin)
        assertEquals("Busy", turn.activity)
        assertEquals(1, turn.backgroundRunning)
        assertEquals(1_000L, turn.startedMs)
        assertEquals(60_000L, turn.durationMs)
        assertEquals(AgentRow.Turn.Outcome.Failed("API error"), turn.outcome)
        assertEquals("run the tests again", turn.suggestion)
        assertNull(page.rows[0].turn)
        assertEquals("turn:p1", page.rows[1].turn)

        val tool = (page.rows[3].kind as AgentRow.Kind.OfTool).tool
        assertEquals("Edit", tool.name)
        assertEquals("src/main.rs", tool.summary)
        assertEquals(AgentRow.Status.Done, tool.status)
        assertEquals(5_000L, tool.startedMs)
        assertEquals(5_400L, tool.endedMs)
        assertEquals("/w/src/main.rs", tool.filePath)
        assertEquals(listOf(AgentRow.Hunk(3, 1, 3, 1, listOf("-a", "+b"))), tool.diff)

        val sub = (page.rows[4].kind as AgentRow.Kind.OfSubagent).subagent
        assertEquals("Explore", sub.agentType)
        assertEquals("Find the callers", sub.description)
        assertTrue(sub.background)
        assertEquals(AgentRow.Status.Ended("Killed"), sub.status)
        assertEquals(7, sub.toolCount)
        assertEquals("Grep fn main", sub.currentAction)
        assertEquals(6_000L, sub.startedMs)
        assertNull(sub.endedMs)
        assertEquals(9_000L, sub.lastMs)

        assertEquals(
            // A question the runner's hook holds (ov-370): its id and its options.
            AgentRow.Kind.OfAsk(
                AgentRow.Ask(
                    "Question", "Which color?", "AskUserQuestion", 7_000L, false, held = "hook-ask-1",
                    questions = listOf(
                        AgentRow.Ask.Question(
                            "Which color?", "Color",
                            listOf(AgentRow.Ask.Option("Red", "Warm"), AgentRow.Ask.Option("Blue", "Calm")), multiSelect = false,
                        ),
                    ),
                ),
            ),
            page.rows[5].kind,
        )
        assertEquals(AgentRow.Kind.OfQueued(AgentRow.Queued("and then the docs", "Waiting", 8_000L)), page.rows[6].kind)
        assertEquals(AgentRow.Kind.OfNotice(AgentRow.Notice("Compacted", "Context compacted", null)), page.rows[7].kind)
        assertEquals(AgentRow.Kind.OfHandoff(AgentRow.Handoff("A panel is open", 9_500L)), page.rows[8].kind)
        assertEquals(AgentRow.Kind.OfGap(AgentRow.Gap("Unknown x-new", 2)), page.rows[9].kind)
        assertEquals(AgentRow.Kind.OfThinking(AgentRow.Thinking(2_500L, 4_500L)), page.rows[2].kind)
        assertEquals(AgentRow.Kind.OfProse(AgentRow.Prose("Looking at **main.rs**.", false, 2_000L)), page.rows[1].kind)

        val follow = AgentRowChanges.decode(AgentRowFixture.follow)
        assertEquals(4L, follow.epoch)
        assertEquals(12L, follow.rev)
        assertFalse(follow.reset)
        assertEquals(3, follow.changes.size)
        assertTrue(follow.changes[0] is AgentRowChanges.Change.Insert)
        assertTrue(follow.changes[1] is AgentRowChanges.Change.Update)
        assertEquals(AgentRowChanges.Change.Remove("queued:1", 12L), follow.changes[2])
    }

    @Test
    fun `a kind or a state this build doesn't know decodes to unknown instead of failing the page`() {
        val body = Json.parseToJsonElement(
            """{"epoch":1,"rev":2,"moreBefore":false,"rows":[
                {"id":"a","ord":0,"rev":1,"provisional":false,"kind":{"Hologram":{"x":1}}},
                {"id":"b","ord":1,"rev":2,"provisional":true,"kind":{"Tool":{"name":"X","summary":"","status":"Teleported"}}},
                {"ord":2,"kind":"Turn"}]}""",
        ).jsonObject
        val page = AgentRowPage.decode(body)
        // The row with no id is dropped; the other two are kept.
        assertEquals(listOf("a", "b"), page.rows.map { it.id })
        assertEquals(AgentRow.Kind.Unknown("Hologram"), page.rows[0].kind)
        assertEquals(AgentRow.Status.Ended("Teleported"), (page.rows[1].kind as AgentRow.Kind.OfTool).tool.status)
        assertTrue(page.rows[1].provisional)
    }

    @Test
    fun `an empty or unreadable answer is an empty page, not a crash`() {
        assertEquals(emptyList<AgentRow>(), AgentRowPage.decode(JsonObject(emptyMap())).rows)
        assertEquals(emptyList<AgentRowChanges.Change>(), AgentRowChanges.decode(JsonObject(emptyMap())).changes)
    }

    @Test
    fun `a tool's input and result, a task list and a prompt's images decode (ov-452, ov-454)`() {
        val body = Json.parseToJsonElement(
            """{"epoch":1,"rev":3,"moreBefore":false,"rows":[
                {"id":"turn:0","ord":0,"rev":1,"kind":{"Turn":{"prompt":"See [Image #1]","origin":"Scheduled","images":[{"mime":"image/png"},{"mime":"image/jpeg"}]}}},
                {"id":"tool:1","ord":1,"rev":2,"kind":{"Tool":{"name":"Bash","summary":"Get time","status":"Done","diff":[],"input":"command: date","result":"Sat"}}},
                {"id":"tool:2","ord":2,"rev":2,"kind":{"Tool":{"name":"Bash","summary":"","status":"Running","diff":[]}}},
                {"id":"tasks:3","ord":3,"rev":3,"kind":{"Tasks":{"items":[{"subject":"A","status":"Completed"},{"subject":"B","status":"InProgress"}]}}}]}""",
        ).jsonObject
        val rows = AgentRowPage.decode(body).rows
        val turn = (rows[0].kind as AgentRow.Kind.OfTurn).turn
        assertEquals(listOf("image/png", "image/jpeg"), turn.images.map { it.mime })
        assertTrue(AgentConversation.isScheduled(turn))
        val opened = (rows[1].kind as AgentRow.Kind.OfTool).tool
        assertEquals("command: date", opened.input)
        assertEquals("Sat", opened.result)
        assertTrue(opened.opens)
        assertFalse("a call with nothing to show doesn't open", (rows[2].kind as AgentRow.Kind.OfTool).tool.opens)
        assertEquals(
            AgentRow.Tasks(listOf(AgentRow.TaskItem("A", "Completed"), AgentRow.TaskItem("B", "InProgress"))),
            (rows[3].kind as AgentRow.Kind.OfTasks).tasks,
        )
    }
}
