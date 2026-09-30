package com.farcooler.model

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The front door: the inbox counts' decode, then the rollup as the front door
 * renders it — items labeled by workspace and merged by rank, what "Nothing
 * needs you" may claim, an older runner's derived items, the Workspaces list,
 * and each item's buttons.
 *
 * All pure, which is why they live in `model/` rather than inside the
 * composable: a phone can prove none of this and a JVM can prove all of it.
 *
 * The rank numbers below are real ones: the item scale is a tier per kind —
 * ask, blocked, decision, review — `100_000_000` wide, then the OLDEST first
 * inside a tier (spec §2.2). Writing them out rather than naming them is the
 * point: these pass only if this app reads that arithmetic the way the runner
 * wrote it.
 */
class NeedsYouTest {
    // ---- the wire ----

    /** The same configuration `Connection` decodes with. */
    private val json = Json { ignoreUnknownKeys = true }

    /**
     * Transcribed key for key from `Session::changes_inbox` in
     * `crates/client/src/session.rs`, including the three keys this app
     * deliberately does not read and the top-level `elsewhere`. A rename on
     * either end fails here rather than going quiet on a phone as a permanently
     * zero count.
     *
     * Values are non-default throughout: `false` where the default is `false`
     * is a test that passes when the decode does nothing at all.
     */
    private val payload = """
        {
          "items": [
            {
              "worktree_id": "8f14e45f-ce5b-4a5e-9c2b-000000000001",
              "short": "8f14e4",
              "task_name": "Widen the model",
              "branch": "feat/widen-the-model",
              "changed_since_reviewed": true,
              "insertions": 82,
              "deletions": 13
            }
          ],
          "elsewhere": 0
        }
    """.trimIndent()

    @Test
    fun `every key the inbox sends is decoded under the name this app reads it by`() {
        val reply = json.decodeFromString(InboxReply.serializer(), payload)
        assertEquals(1, reply.items.size)
        val row = reply.items[0]
        assertEquals("8f14e45f-ce5b-4a5e-9c2b-000000000001", row.worktreeId)
        assertTrue("changed_since_reviewed must survive the snake_case", row.changedSinceReviewed)
        assertEquals(82, row.insertions)
        assertEquals(13, row.deletions)
        assertTrue(row.hasDiff)
    }

    /**
     * A daemon that answers with nothing but the id — every other field is
     * `#[serde(default)]`-shaped on the wire, and a proto-3 zero is simply
     * absent. This must be a clean, empty row rather than a decode failure that
     * costs the whole poll.
     */
    @Test
    fun `a row with only an id decodes to a clean empty one`() {
        val reply = json.decodeFromString(
            InboxReply.serializer(),
            """{"items":[{"worktree_id":"w"}]}""",
        )
        val row = reply.items[0]
        assertFalse(row.changedSinceReviewed)
        assertEquals(0, row.insertions)
        assertEquals(0, row.deletions)
        assertFalse("no numbers is no diff", row.hasDiff)
    }

    /** A newer daemon adding a key must not cost an older app its counts. */
    @Test
    fun `an unknown key is ignored rather than fatal`() {
        val reply = json.decodeFromString(
            InboxReply.serializer(),
            """{"items":[{"worktree_id":"w","insertions":3,"conflicts":7}],"nudges":1}""",
        )
        assertEquals(3, reply.items[0].insertions)
    }

    /** A reply with no items at all is empty, not a failure. */
    @Test
    fun `an empty inbox decodes`() {
        assertTrue(json.decodeFromString(InboxReply.serializer(), "{}").items.isEmpty())
    }

    /** Deletions alone are still a diff. A file emptied is not a clean worktree. */
    @Test
    fun `deletions alone count as a diff`() {
        assertTrue(InboxRow("w", insertions = 0, deletions = 4).hasDiff)
    }

    // ---- the items ----

    /**
     * Spec §6.2: "each item is a row labeled with its workspace", in one list
     * across runners, by rank. The ask on the second runner is the most urgent
     * thing in the fleet and comes first, above the first runner's decision,
     * whichever runner was listed first; and each row names its workspace —
     * the one the runner sent, the one this phone knows by id, the repository
     * on a runner without workspaces, or Unclaimed.
     */
    @Test
    fun `items are labeled by workspace and ordered by rank`() {
        val studio = runner(
            "studio",
            items = listOf(
                item("decision:t1", "decision", rank = 299_999_699, workspaceId = "ws-billing", workspaceName = "Billing"),
                item("review:t2", "review", rank = 399_996_399, workspaceId = null, repositoryId = "repo"),
            ),
            workspaces = listOf(WorkspaceSummary(id = "ws-billing", name = "Billing", repository = "repo")),
        )
        val box = runner(
            "build-box",
            // The runner sent no name for its workspace; this phone knows it.
            items = listOf(item("ask:a1", "ask", rank = 99_999_989, workspaceId = "ws-main")),
            workspaces = listOf(WorkspaceSummary(id = "ws-main", name = "Main", isMain = true, repository = "r2")),
        )
        val older = runner(
            "old",
            items = listOf(item("blocked:p", "blocked", rank = 100_000_050, workspaceId = null, repositoryId = "r3")),
            workspaces = null,
            repositories = listOf(Repository(id = "r3", displayName = "overnight")),
        )

        val rows = NeedsYou.rows(listOf(studio, box, older))
        assertEquals(listOf("ask:a1", "blocked:p", "decision:t1", "review:t2"), rows.map { it.item.id })
        assertEquals(listOf("Main", "overnight", "Billing", "Unclaimed"), rows.map { it.place })
        assertEquals(listOf("build-box", "old", "studio", "studio"), rows.map { it.runner })

        // One runner is named nowhere: its name says nothing about which row is which.
        assertEquals(listOf(null, null), NeedsYou.rows(listOf(studio)).map { it.runner })
    }

    /**
     * The sentence is a claim about every item, so a decision alone keeps it
     * off the screen. The old front door said "Nothing needs you" over a Board
     * row counting a decision in amber (ov-56).
     */
    @Test
    fun `nothing needs you is never shown above a decision`() {
        val deciding = runner("h", items = listOf(item("decision:t", "decision", rank = 299_000_000)))
        val reviewing = runner("h", items = listOf(item("review:t", "review", rank = 399_000_000)))
        assertFalse(NeedsYou.nothingNeedsYou(NeedsYou.rows(listOf(deciding))))
        assertFalse(NeedsYou.nothingNeedsYou(NeedsYou.rows(listOf(reviewing))))
        assertTrue(NeedsYou.nothingNeedsYou(NeedsYou.rows(listOf(runner("h", items = emptyList())))))
        // A runner that hasn't answered adds nothing, and invents nothing.
        assertTrue(NeedsYou.nothingNeedsYou(NeedsYou.rows(listOf(NeedsYouRunner("h", "h", reading = null)))))
    }

    /**
     * Ruling 1: a finished agent is not an item. On a runner too old to send
     * its items, only its blocked agents are derived — the Done agent beside
     * the blocked one keeps its glyph and its notification, and isn't here.
     * The old front door listed it under its worktree.
     */
    @Test
    fun `a finished agent is not an item`() {
        val fleet = Fleet(
            worktrees = listOf(
                Worktree(
                    id = "w",
                    repository = "repo",
                    terminals = listOf(
                        agent("stuck", "blocked", rank = 60),
                        agent("finished", "done", rank = 100_000_030),
                        agent("busy", "working", rank = 200_000_000),
                    ),
                ),
            ),
        )
        val reading = NeedsYou.derivedReading(fleet)
        assertTrue(reading.derived)
        val rows = NeedsYou.rows(listOf(NeedsYouRunner("h", "old", reading, fleet)))
        assertEquals(listOf("blocked:stuck"), rows.map { it.item.id })
        assertEquals(listOf("old"), NeedsYou.olderRunners(listOf(NeedsYouRunner("h", "old", reading, fleet))))
        assertEquals(
            "Update Far Cooler on old to see decisions and asks here.",
            NeedsYou.olderRunnerNote("old"),
        )
    }

    // ---- the Workspaces list ----

    /**
     * Each repository's workspaces, Main first, each with its orchestrator and
     * its count; an empty Main still has its row (spec §8); worktrees nobody
     * owns are Unclaimed, hidden ones are Hidden, and an item nobody owns
     * counts under Unclaimed.
     */
    @Test
    fun `the workspaces list counts items and keeps an empty workspace`() {
        val orchestrator = Terminal(id = "o", preset = "claude", state = "running", role = "orchestrator", workspace = "ws-b")
        val fleet = Fleet(
            worktrees = listOf(
                Worktree(id = "main-co", repository = "repo", workspace = "ws-main", terminals = listOf(orchestrator)),
                Worktree(id = "w-b", repository = "repo", workspace = "ws-b"),
                Worktree(id = "w-loose", repository = "repo"),
                Worktree(id = "w-hid", repository = "repo", workspace = "ws-b", state = "hidden"),
            ),
            workspaces = listOf(
                WorkspaceSummary(id = "ws-b", name = "Billing", ordinal = 1, repository = "repo"),
                WorkspaceSummary(id = "ws-main", name = "Main", isMain = true, repository = "repo"),
                WorkspaceSummary(id = "ws-empty", name = "Search", ordinal = 2, repository = "repo"),
            ),
        )
        val runner = NeedsYouRunner(
            "h", "h",
            RunnerNeedsYou(
                listOf(
                    item("ask:1", "ask", 1, workspaceId = "ws-b", repositoryId = "repo"),
                    item("decision:2", "decision", 300_000_000, workspaceId = "ws-b", repositoryId = "repo"),
                    item("review:3", "review", 400_000_000, workspaceId = null, repositoryId = "repo"),
                )
            ),
            fleet,
            listOf(Repository(id = "repo", displayName = "overnight"), Repository(id = "bare", displayName = "bare")),
        )
        val merged = NeedsYou.rows(listOf(runner)).map { it.entry }
        val sections = NeedsYou.workspaces(runner, merged)

        val repo = sections.first { it.repository == "repo" }
        assertEquals("overnight", repo.name)
        assertEquals(listOf("Main", "Billing", "Search"), repo.workspaces.map { it.name })
        assertEquals(listOf(0, 2, 0), repo.workspaces.map { it.count })
        assertEquals("o", repo.workspaces[1].orchestrator?.id)
        assertNull(repo.workspaces[0].orchestrator)
        assertEquals(listOf("w-loose"), repo.unclaimed)
        assertEquals(1, repo.unclaimedCount)
        assertEquals(listOf("w-hid"), repo.hidden)

        // A registered repository with nothing in it still has its board's row.
        val bare = sections.first { it.repository == "bare" }
        assertEquals(listOf("bare"), bare.workspaces.map { it.name })
        assertTrue(bare.workspaces.single().workspace.isImplicit)
    }

    /** What each list of worktrees holds: a workspace's own, its Unclaimed, its Hidden. */
    @Test
    fun `a scope holds its own worktrees and hidden ones only under Hidden`() {
        val fleet = Fleet(workspaces = listOf(WorkspaceSummary(id = "ws", repository = "repo")))
        val mine = Worktree(id = "a", repository = "repo", workspace = "ws")
        val loose = Worktree(id = "b", repository = "repo")
        val hidden = Worktree(id = "c", repository = "repo", state = "hidden")
        val elsewhere = Worktree(id = "d", repository = "other")
        val hiddenMine = Worktree(id = "e", repository = "repo", workspace = "ws", state = "hidden")
        val all = listOf(mine, loose, hidden, elsewhere, hiddenMine)
        fun ids(scope: WorktreeScope) = all.filter { scope.includes(it, fleet) }.map { it.id }

        assertEquals(listOf("a"), ids(WorktreeScope.OfWorkspace("h", WorkspaceSummary(id = "ws", repository = "repo"))))
        assertEquals(listOf("b"), ids(WorktreeScope.OfRepository("h", "repo", hidden = false)))
        assertEquals(listOf("c", "e"), ids(WorktreeScope.OfRepository("h", "repo", hidden = true)))
        // A runner without workspaces: the implicit workspace holds them all.
        assertEquals(
            listOf("a", "b"),
            all.filter { WorktreeScope.OfWorkspace("h", WorkspaceSummary.implicit("repo")).includes(it, Fleet()) }.map { it.id },
        )
    }

    // ---- answering ----

    /** Spec §2.5, per kind, and a Read-scoped reader gets Open and nothing that writes. */
    @Test
    fun `each kind offers its own buttons`() {
        val allow = NeedsYouAction("allow", "Allow", primary = true)
        val deny = NeedsYouAction("deny", "Deny", destructive = true)
        val ask = item("ask:1", "ask", 1).copy(askId = "hook-ask-1", actions = listOf(deny, allow))
        assertEquals(listOf(NeedsYouButton.Answer(deny), NeedsYouButton.Answer(allow)), NeedsYouAnswer.buttons(ask, mayAnswer = true))
        assertEquals(listOf(NeedsYouButton.Open("Open")), NeedsYouAnswer.buttons(ask, mayAnswer = false))

        val options = (1..5).map { NeedsYouAction("o$it", "o$it") }
        val decision = item("decision:t", "decision", 3).copy(actions = options)
        assertEquals(
            options.take(3).map(NeedsYouButton::Answer) + NeedsYouButton.More(options.drop(3)),
            NeedsYouAnswer.buttons(decision, mayAnswer = true),
        )
        assertEquals(listOf(NeedsYouButton.Write), NeedsYouAnswer.buttons(decision.copy(actions = emptyList()), true))
        assertEquals(listOf(NeedsYouButton.Open("Open")), NeedsYouAnswer.buttons(decision, mayAnswer = false))

        // The inbox opens a review; it never approves one (ruling 2).
        val review = item("review:t", "review", 4).copy(actions = listOf(NeedsYouAction("open", "Open")))
        assertEquals(listOf(NeedsYouButton.Open("Review")), NeedsYouAnswer.buttons(review, mayAnswer = true))
        assertEquals(listOf(NeedsYouButton.Open("Open")), NeedsYouAnswer.buttons(item("blocked:p", "blocked", 2), true))
    }

    @Test
    fun `a refused answer says which refusal`() {
        assertEquals("Someone already answered this.", NeedsYouAnswer.refusal("not_held", "claude"))
        assertEquals("Couldn’t reach claude. Try again.", NeedsYouAnswer.refusal("not_delivered", "claude"))
        assertEquals("Couldn’t send that answer. Try again.", NeedsYouAnswer.refusal(null, "claude"))
    }

    // ---- a decision push's tap ----

    /**
     * A decision push names its task by key and no runner: the tap finds it
     * on whichever runner has it — its needs-you item first, which knows its
     * workspace, then any board read so far.
     */
    @Test
    fun `a decision push opens its task on the runner that has it`() {
        val decision = item("decision:t7", "decision", 300_000_000, workspaceId = "ws-b", repositoryId = "repo")
            .copy(task = TaskRef(id = "t7", key = "bil-7"))
        val sources = listOf(
            DecisionSource("studio", RunnerNeedsYou(emptyList()), emptyMap(), null),
            DecisionSource("box", RunnerNeedsYou(listOf(decision)), emptyMap(), null),
        )
        assertEquals(DecisionTarget("box", "ws-b", "repo", "t7"), DecisionLink.find("bil-7", sources))

        // Answered since, so not an item: its card on a board read still is.
        val board = TaskBoard(listOf(TaskBoardColumn(TaskStatus.NEEDS_DECISION, listOf(TaskRow("t7", "bil-7", "x", TaskStatus.NEEDS_DECISION, 0L)))))
        val onBoard = listOf(DecisionSource("box", RunnerNeedsYou(emptyList()), mapOf("ws-b" to board), null))
        assertEquals(DecisionTarget("box", "ws-b", null, "t7"), DecisionLink.find("bil-7", onBoard))

        assertNull(DecisionLink.find("bil-8", sources))
        assertNull(DecisionLink.find("", sources))
    }

    /**
     * A key on two runners goes to the one the push names, by its
     * `Host.runner_id`, however cased; with no name it is nobody's (ov-72).
     * A name nobody answers to waits, then falls back to the key alone: one
     * runner has it, so it opens; two, and it does not.
     */
    @Test
    fun `a decision push names its runner when the key is on two`() {
        fun on(host: String, id: String?, ws: String) = DecisionSource(
            host,
            RunnerNeedsYou(listOf(
                item("decision:t7", "decision", 300_000_000, workspaceId = ws, repositoryId = "repo")
                    .copy(task = TaskRef(id = "t7", key = "bil-7")),
            )),
            emptyMap(),
            id,
        )
        val both = listOf(on("a", "host-a", "ws-a"), on("b", "host-b", "ws-b"))
        assertEquals(DecisionTarget("b", "ws-b", "repo", "t7"), DecisionLink.find("bil-7", both, runner = "HOST-b"))
        assertNull(DecisionLink.find("bil-7", both))
        assertNull(DecisionLink.find("bil-7", both, runner = "host-gone"))
        assertNull(DecisionLink.find("bil-7", both, runner = "host-gone", waitEnded = true))
        val bare = listOf(on("a", "host-a", "ws-a"), DecisionSource("b", RunnerNeedsYou(emptyList()), emptyMap(), "host-b"))
        assertNull(DecisionLink.find("bil-7", bare, runner = "host-b", waitEnded = true))
        val unread = listOf(on("a", null, "ws-a"))
        assertNull(DecisionLink.find("bil-7", unread, runner = "host-a"))
        assertEquals(
            DecisionTarget("a", "ws-a", "repo", "t7"),
            DecisionLink.find("bil-7", unread, runner = "host-a", waitEnded = true),
        )
    }

    // ---- fixtures ----

    private fun runner(
        hostId: String,
        items: List<NeedsYouItem>,
        workspaces: List<WorkspaceSummary>? = emptyList(),
        repositories: List<Repository> = emptyList(),
    ) = NeedsYouRunner(hostId, hostId, RunnerNeedsYou(items), Fleet(workspaces = workspaces), repositories)

    private fun item(
        id: String,
        kind: String,
        rank: Long,
        workspaceId: String? = null,
        workspaceName: String = "",
        repositoryId: String? = null,
    ) = NeedsYouItem(
        id = id,
        kind = kind,
        rank = rank,
        workspaceId = workspaceId,
        workspaceName = workspaceName,
        repositoryId = repositoryId,
    )

    private fun agent(id: String, activity: String, rank: Long?) = Terminal(
        id = id,
        preset = "claude",
        state = "running",
        activity = activity,
        rank = rank,
    )
}
