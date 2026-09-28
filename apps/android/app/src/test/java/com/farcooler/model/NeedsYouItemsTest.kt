package com.farcooler.model

import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * The needs-you shape, read from the fixture the client core's own test says
 * it writes (`test/fixtures/needs-you.json`), and the merge across runners.
 *
 * Every field is asserted by value. The fixture sets every optional between
 * its items, so a key misspelled here decodes to its default and fails.
 */
class NeedsYouItemsTest {
    /** The same configuration `Connection` decodes a fleet with. */
    private val json = Json { ignoreUnknownKeys = true }

    private fun fixture(): Map<String, List<NeedsYouItem>> {
        val root = json.parseToJsonElement(repositoryFile("test/fixtures/needs-you.json")).jsonObject
        return root.getValue("runners").jsonArray.associate { runner ->
            val o = runner.jsonObject
            o.getValue("runner").jsonPrimitive.content to
                json.decodeFromJsonElement(NeedsYouList.serializer(), o.getValue("needs_you")).items
        }
    }

    @Test
    fun `the shared fixture decodes to the values it holds`() {
        val runners = fixture()
        assertEquals(listOf("studio", "build-box"), runners.keys.toList())
        val studio = runners.getValue("studio")
        assertEquals(4, studio.size)

        val ask = studio[0]
        assertEquals("ask:hook-ask-01000000-0000-7000-8000-000000000029", ask.id)
        assertEquals(NeedsYouKind.ASK, ask.kindValue)
        assertEquals(listOf(NeedsYouKind.DECISION), ask.alsoValues)
        assertEquals(99_999_939L, ask.rank)
        assertEquals(1_789_999_940_000L, ask.since)
        assertEquals("01000000-0000-7000-8000-000000000001", ask.workspaceId)
        assertEquals("Billing", ask.workspaceName)
        assertEquals("01000000-0000-7000-8000-000000000002", ask.repositoryId)
        assertEquals(
            TaskRef("01000000-0000-7000-8000-000000000003", "bil-7", "Invoice PDF export", "needs_decision"),
            ask.task,
        )
        assertEquals(
            NeedsYouTerminal(
                id = "01000000-0000-7000-8000-000000000004",
                worktreeId = "01000000-0000-7000-8000-000000000005",
                label = "claude",
                role = "agent",
                paneMode = "terminal",
                chatCapable = true,
            ),
            ask.terminal,
        )
        assertEquals(
            NeedsYouWorktree("01000000-0000-7000-8000-000000000005", "fc-3-webhooks", "bil/webhooks", 18, 40),
            ask.worktree,
        )
        assertEquals("Allow touch x", ask.question)
        assertNull(ask.detail)
        assertEquals("hook-ask-01000000-0000-7000-8000-000000000029", ask.askId)
        assertEquals(
            listOf(
                NeedsYouAction("allow", "Allow touch x", destructive = false, primary = true),
                NeedsYouAction("deny", "Deny", destructive = true, primary = false),
            ),
            ask.actions,
        )

        val blocked = studio[1]
        assertEquals(NeedsYouKind.BLOCKED, blocked.kindValue)
        assertEquals("Run the migration now?", blocked.question)
        assertEquals(199_999_879L, blocked.rank)
        assertNull(blocked.task)
        assertNull(blocked.askId)
        assertEquals("orchestrator", blocked.terminal?.role)
        assertEquals(false, blocked.terminal?.chatCapable)
        assertEquals(listOf(NeedsYouAction("open", "Open")), blocked.actions)

        val decision = studio[2]
        assertEquals(NeedsYouKind.DECISION, decision.kindValue)
        assertEquals("Postgres or SQLite for the queue?", decision.question)
        assertEquals("bil-9", decision.task?.key)
        assertNull(decision.terminal)
        assertNull(decision.worktree)
        assertEquals(listOf("Postgres", "SQLite"), decision.actions.map { it.id })

        val review = studio[3]
        assertEquals(NeedsYouKind.REVIEW, review.kindValue)
        assertEquals("+18 −40", review.detail)
        assertEquals("in_review", review.task?.status)
        assertEquals(399_996_399L, review.rank)

        // Every item a runner sent is its own, not one this phone made up.
        assertEquals(false, runners.values.flatten().any { it.isDerived })

        val box = runners.getValue("build-box")
        assertEquals(listOf(NeedsYouKind.ASK, NeedsYouKind.REVIEW), box.map { it.kindValue })
        // Below Control scope: no buttons, no ask id, and a fixed sentence.
        assertEquals(emptyList<NeedsYouAction>(), box[0].actions)
        assertNull(box[0].askId)
        assertEquals("claude is asking to use a tool", box[0].question)
        // A task on no workspace: null, never the nil uuid, and an empty name.
        assertNull(box[1].workspaceId)
        assertEquals("", box[1].workspaceName)
        assertEquals("ops-2", box[1].task?.key)
    }

    @Test
    fun `two runners merge by rank not by clock`() {
        val merged = NeedsYouItems.merge(fixture())
        // The build box's review began before anything on the studio, by the
        // clocks, and still goes after every ask, block and decision there:
        // its tier is review. Within a tier, the older item goes first.
        assertEquals(
            listOf(
                "studio/ask:hook-ask-01000000-0000-7000-8000-000000000029",
                "build-box/ask:hook-ask-01000000-0000-7000-8000-00000000002a",
                "studio/blocked:01000000-0000-7000-8000-00000000000a",
                "studio/decision:01000000-0000-7000-8000-00000000000c",
                "build-box/review:01000000-0000-7000-8000-00000000001e",
                "studio/review:01000000-0000-7000-8000-00000000000d",
            ),
            merged.map { it.key },
        )
        // The clock disagrees: sorting by `since` would put the build box's
        // review first. This is what makes the case above a test of rank.
        assertEquals(
            "build-box/review:01000000-0000-7000-8000-00000000001e",
            merged.minByOrNull { it.item.since ?: Long.MAX_VALUE }?.key,
        )
    }

    @Test
    fun `an unknown kind decodes and sorts last`() {
        val newer = NeedsYouItem(id = "later:1", kind = "later", rank = 1)
        val review = NeedsYouItem(id = "review:1", kind = "review", rank = 399_000_000)
        val merged = NeedsYouItems.merge(mapOf("a" to listOf(newer, review)))
        assertEquals(NeedsYouKind.UNKNOWN, newer.kindValue)
        assertEquals(listOf("review:1", "later:1"), merged.map { it.item.id })
    }

    @Test
    fun `a workspace's count is its items, not its signals`() {
        val merged = NeedsYouItems.merge(fixture())
        // The ask carries a decision in `also`: four items, not five signals.
        assertEquals(4, NeedsYouItems.count(merged, "studio", "01000000-0000-7000-8000-000000000001"))
        assertEquals(1, NeedsYouItems.count(merged, "build-box", null))
        assertEquals(0, NeedsYouItems.count(merged, "studio", null))
    }

    @Test
    fun `an older runner's blocked agent never outranks a real ask`() {
        // `Terminal.rank` tier 0 is Blocked: this agent has been stuck for an
        // hour, so copied as it stands it would read 99_996_399 and go above
        // an ask a minute old.
        val stuck = Terminal(
            id = "t1", preset = "codex", activity = "blocked", rank = 99_996_399L,
            workspace = "ws", role = "agent",
        )
        val finished = Terminal(id = "t2", preset = "claude", activity = "done", rank = 199_000_000L)
        val older = Worktree(id = "w1", task = "fc-3", terminals = listOf(stuck, finished), state = "hidden")
        val derived = NeedsYouItems.derived(listOf(older))

        assertEquals(1, derived.size)
        val item = derived.single()
        assertEquals(NeedsYouKind.BLOCKED, item.kindValue)
        assertEquals("blocked:t1", item.id)
        assertEquals(199_996_399L, item.rank)
        assertEquals("codex needs you", item.question)
        assertEquals("ws", item.workspaceId)
        assertEquals(true, item.isDerived)

        val merged = NeedsYouItems.merge(mapOf("old" to derived, "studio" to fixture().getValue("studio")))
        assertEquals(
            listOf("ask", "blocked", "blocked", "decision", "review"),
            merged.map { it.item.kindValue.wire },
        )
        assertEquals("studio", merged.first().hostId)
    }

    @Test
    fun `an older runner derives no decisions or reviews`() {
        // A task in Needs Decision and one In Review, each with a working
        // agent: an older runner still says nothing about either.
        val deciding = Terminal(id = "t1", preset = "claude", activity = "working", taskId = "task-1")
        val reviewing = Terminal(id = "t2", preset = "claude", activity = "idle", taskId = "task-2")
        val wt = Worktree(
            id = "w",
            terminals = listOf(deciding, reviewing),
            openTasks = listOf(
                TaskRef("task-1", "bil-1", "Pick a queue", "needs_decision"),
                TaskRef("task-2", "bil-2", "Webhook signatures", "in_review"),
            ),
        )
        assertEquals(emptyList<NeedsYouItem>(), NeedsYouItems.derived(listOf(wt)))
    }

    @Test
    fun `equal ranks hold still by runner, then by id`() {
        val a1 = NeedsYouItem(id = "blocked:1", kind = "blocked", rank = 150_000_000)
        val a2 = NeedsYouItem(id = "blocked:2", kind = "blocked", rank = 150_000_000)
        val b1 = NeedsYouItem(id = "blocked:1", kind = "blocked", rank = 150_000_000)
        val expected = listOf("alpha/blocked:1", "alpha/blocked:2", "beta/blocked:1")
        // Whatever order the runners and their items arrive in.
        assertEquals(expected, NeedsYouItems.merge(mapOf("beta" to listOf(b1), "alpha" to listOf(a2, a1))).map { it.key })
        assertEquals(expected, NeedsYouItems.merge(mapOf("alpha" to listOf(a1, a2), "beta" to listOf(b1))).map { it.key })
    }

    /** A file in this checkout, found by walking up from wherever Gradle runs. */
    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}")
    }
}
