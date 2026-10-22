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
        assertEquals(6, studio.size)

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
        assertEquals(listOf(NeedsYouAction("open", "Open")), blocked.actions)

        val decision = studio[2]
        assertEquals(NeedsYouKind.DECISION, decision.kindValue)
        assertEquals("Postgres or SQLite for the queue?", decision.question)
        assertEquals("bil-9", decision.task?.key)
        assertNull(decision.terminal)
        assertNull(decision.worktree)
        assertEquals(listOf("Postgres", "SQLite"), decision.actions.map { it.id })

        // The two pull-request kinds (ov-315).
        val thread = studio[3]
        assertEquals(NeedsYouKind.PR_THREAD, thread.kindValue)
        assertEquals("pr-thread:01000000-0000-7000-8000-00000000000e", thread.id)
        assertEquals(399_999_099L, thread.rank)
        assertEquals("bil-11", thread.task?.key)
        assertEquals("alice asked, says the agent: “Why not retry on 5xx?”", thread.question)
        assertEquals("acme/billing #212", thread.detail)
        assertEquals("https://github.com/acme/billing/pull/212#discussion_r1", thread.link)
        assertEquals(listOf("Retry"), thread.actions.map { it.id })

        val request = studio[4]
        assertEquals(NeedsYouKind.PR_REVIEW, request.kindValue)
        assertEquals("pr-review:0123456789abcdef", request.id)
        assertEquals(499_998_199L, request.rank)
        assertNull(request.task)
        assertNull(request.terminal)
        assertNull(request.workspaceId)
        assertEquals("Add refund endpoint", request.question)
        assertEquals("acme/billing #213 · bob", request.detail)
        assertEquals("https://github.com/acme/billing/pull/213", request.link)
        assertEquals(emptyList<NeedsYouAction>(), request.actions)
        assertEquals(emptyList<String?>(), studio.filter { it.kindValue != NeedsYouKind.PR_REVIEW && it.kindValue != NeedsYouKind.PR_THREAD }.mapNotNull { it.link })

        val review = studio[5]
        assertEquals(NeedsYouKind.REVIEW, review.kindValue)
        assertEquals("+18 −40", review.detail)
        assertEquals("in_review", review.task?.status)
        assertEquals(599_996_399L, review.rank)

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
    fun `an item's concern decodes, and a null one is nobody's (ov-633)`() {
        val studio = fixture().getValue("studio")
        assertEquals(NeedsYouConcern("alice", "Alice Chen", yours = true), studio[0].concern)
        assertEquals(false, studio[0].isOthers)
        assertNull(studio[0].othersName)
        assertEquals(NeedsYouConcern("bob", "Bob Okafor", yours = false), studio[1].concern)
        assertEquals("Bob Okafor", studio[1].othersName)
        assertNull(studio[2].concern)
        assertEquals(false, studio[2].isOthers)
    }

    @Test
    fun `a list splits into Yours and Others only when something is someone else's (ov-633)`() {
        fun row(id: String, concern: NeedsYouConcern?) =
            NeedsYouRow(RunnerNeedsYouItem("studio", NeedsYouItem(id = id, concern = concern)), "Billing", null)
        val bob = NeedsYouConcern("bob", "Bob Okafor", yours = false)
        val owner = NeedsYouConcern("", "", yours = false)
        val mine = NeedsYouConcern("alice", "Alice Chen", yours = true)
        val (yours, others) = NeedsYou.grouped(listOf(row("a", bob), row("b", null), row("c", mine), row("d", owner)))
        assertEquals(listOf("b", "c"), yours.map { it.item.id })
        assertEquals(listOf("a", "d"), others.map { it.item.id })
        assertEquals(listOf("Bob Okafor", "Owner"), others.map { it.item.othersName })
        val (solo, none) = NeedsYou.grouped(listOf(row("a", null), row("b", null)))
        assertEquals(listOf("a", "b"), solo.map { it.item.id })
        assertEquals(emptyList<NeedsYouRow>(), none)
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
                "studio/pr-thread:01000000-0000-7000-8000-00000000000e",
                "studio/pr-review:0123456789abcdef",
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
    fun `the pull request kinds sort between a decision and a local review`() {
        val tier = NeedsYouItems.TIER_SPAN
        fun item(kind: String, tiers: Long) = NeedsYouItem(id = kind, kind = kind, rank = tiers * tier + 5)
        val merged = NeedsYouItems.merge(
            mapOf(
                "a" to listOf(item("review", 5), item("pr_review", 4), item("decision", 2)),
                "b" to listOf(item("pr_thread", 3), item("ask", 0), item("blocked", 1)),
            ),
        )
        assertEquals(
            listOf("ask", "blocked", "decision", "pr_thread", "pr_review", "review"),
            merged.map { it.item.id },
        )
    }

    @Test
    fun `a thread is answered like a decision beside Review on GitHub and Reply`() {
        val options = listOf(NeedsYouAction("Retry", "Retry"), NeedsYouAction("Leave it", "Leave it"))
        val link = "https://github.com/acme/billing/pull/212#discussion_r1"
        val thread = NeedsYouItem(id = "t", kind = "pr_thread", actions = options, task = TaskRef(id = "t-1"), link = link)
        val github = NeedsYouButton.GitHub(link)
        // Reply only where the runner posts and this device holds Control.
        assertEquals(listOf(github, NeedsYouButton.Choose(options)), NeedsYouAnswer.buttons(thread, mayAnswer = true))
        assertEquals(listOf(github, NeedsYouButton.Reply, NeedsYouButton.Choose(options)), NeedsYouAnswer.buttons(thread, mayAnswer = true, mayReply = true))
        assertEquals(listOf(github, NeedsYouButton.Write), NeedsYouAnswer.buttons(thread.copy(actions = emptyList()), true))
        // Below Control the runner sends no link and no actions: Open alone, and never a Reply.
        val redacted = thread.copy(actions = emptyList(), link = null)
        assertEquals(listOf(NeedsYouButton.Open("Open")), NeedsYouAnswer.buttons(redacted, mayAnswer = false, mayReply = true))
        // A thread with no card has nothing to reply on.
        assertEquals(listOf(github, NeedsYouButton.Choose(options)), NeedsYouAnswer.buttons(thread.copy(task = null), true, mayReply = true))
    }

    @Test
    fun `a review request opens GitHub, and only a github dot com link does`() {
        val url = "https://github.com/a/b/pull/1"
        val request = NeedsYouItem(id = "r", kind = "pr_review", link = url)
        for (may in listOf(true, false)) {
            assertEquals(listOf<NeedsYouButton>(NeedsYouButton.GitHub(url, review = true)), NeedsYouAnswer.buttons(request, may))
        }
        for (bad in listOf(null, "", "http://github.com/a/b/pull/1", "https://github.com.evil.example/a/b/pull/1", "https://github.com@evil.example/a/b/pull/1", "https://github.com:8443/a/b", "https://github.com/a b", "javascript:alert(1)", "https://notgithub.com/a/b/pull/1")) {
            assertEquals("$bad", emptyList<NeedsYouButton>(), NeedsYouAnswer.buttons(request.copy(link = bad), true))
        }
    }

    @Test
    fun `a review request on two runners shows once from the lowest rank`() {
        fun request(id: String, rank: Long) = NeedsYouItem(id = id, kind = "pr_review", rank = rank)
        val merged = NeedsYouItems.merge(
            mapOf(
                "studio" to listOf(request("pr-review:aaaa", 500_000_000), request("pr-review:bbbb", 700)),
                "build-box" to listOf(request("pr-review:aaaa", 400_000_000)),
            ),
        )
        assertEquals(listOf("build-box/pr-review:aaaa", "studio/pr-review:bbbb").sorted(), merged.map { it.key }.sorted())
        assertEquals(listOf("pr-review:bbbb", "pr-review:aaaa"), merged.map { it.item.id })
        // Only a request is: the same id elsewhere is two items.
        val ask = NeedsYouItem(id = "ask:x", kind = "ask", rank = 5)
        assertEquals(2, NeedsYouItems.merge(mapOf("a" to listOf(ask), "b" to listOf(ask))).size)
    }

    @Test
    fun `a thread counts as a decision even as an also`() {
        assertEquals(true, NeedsYouKind.PR_THREAD.isDecision)
        assertEquals(false, NeedsYouKind.PR_REVIEW.isDecision)
        val ask = NeedsYouItem(id = "ask:a", kind = "ask", also = listOf("pr_thread"), workspaceId = "w")
        val workspace = WorkspaceSummary(id = "w", name = "W")
        assertEquals(1, RunnerBoards.decisions(workspace, listOf(ask)))
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
        // The ask carries a decision in `also`: five items, not six signals. The
        // pull request that wants a review is in no workspace.
        assertEquals(5, NeedsYouItems.count(merged, "studio", "01000000-0000-7000-8000-000000000001"))
        assertEquals(1, NeedsYouItems.count(merged, "build-box", null))
        // The studio's pull request that wants a review is its one item in no workspace.
        assertEquals(1, NeedsYouItems.count(merged, "studio", null))
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
            listOf("ask", "blocked", "blocked", "decision", "pr_thread", "pr_review", "review"),
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
