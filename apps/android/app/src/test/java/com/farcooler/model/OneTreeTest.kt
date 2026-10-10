package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * The One tree and the plan strip on Android (ov-300), the ports of AgentKit's
 * `OneTree`, `PhoneTree` and `PlanStrip`. The plan is `test/fixtures/plan-seeded.json`,
 * the CLI's own output, which AgentKit's `PhoneTreeTests` and the iPhone's UI
 * tests read too.
 */
class OneTreeTest {
    private val billing = WorkspaceSummary(id = "ws-billing", name = "Billing", repository = "repo-1")

    private val plan: Plan = Plan.decode(Json.parseToJsonElement(repositoryFile("test/fixtures/plan-seeded.json")).jsonObject["plan"]!!.jsonObject)

    private fun row(id: String, key: String, status: TaskStatus = TaskStatus.IN_PROGRESS, worktree: String? = null) =
        TaskRow(id = id, key = key, title = "Title $key", status = status, statusSince = 0L, worktreeId = worktree)

    private fun board(vararg rows: TaskRow) =
        TaskBoard(TaskStatus.ORDER.map { s -> TaskBoardColumn(s, rows.filter { it.status == s }) })

    private val worktrees = listOf(
        // mac-vis's lane worktree, joined by its path.
        Worktree(id = "wt-mac-vis", repository = "repo-1", task = "mac-vis", branch = "mac-vis", worktree = "/repo/.claude/worktrees/mac-vis", workspace = "ws-billing", state = "ready",
            terminals = listOf(Terminal(id = "c1", preset = "claude", state = "running", activity = "working", role = "agent"), Terminal(id = "c2", preset = "zsh", state = "running"), Terminal(id = "c3", preset = "zsh", state = "running", paneMode = "changes"))),
        // bil-9's own worktree.
        Worktree(id = "wt-hooks", repository = "repo-1", task = "fc-3-webhooks", branch = "feat/webhooks", workspace = "ws-billing", state = "ready",
            terminals = listOf(Terminal(id = "s1", preset = "zsh", state = "running"))),
        // Loose: Billing's, and no card's.
        Worktree(id = "wt-spike", repository = "repo-1", task = "spike", branch = "spike", workspace = "ws-billing", state = "hidden"),
        // Another workspace's.
        Worktree(id = "wt-other", repository = "repo-1", task = "other", branch = "other", workspace = "ws-other", state = "ready"),
        // The checkout: a shell of the project's, the orchestrator, and a task's agent.
        Worktree(id = "wt-main", repository = "repo-1", task = "overnight", branch = "main", isMainCheckout = true, state = "ready",
            terminals = listOf(Terminal(id = "m1", preset = "zsh", state = "running"), Terminal(id = "m2", preset = "claude", state = "running", role = "orchestrator"), Terminal(id = "m3", preset = "claude", state = "running", taskId = "t9"))),
    )

    private val rows = board(row("t9", "bil-9", worktree = "wt-hooks"), row("t7", "bil-7", TaskStatus.NEEDS_DECISION), row("t5", "bil-5", TaskStatus.DONE))

    private fun tree(filter: OneTree.Filter = OneTree.Filter.OPEN, items: List<NeedsYouItem> = emptyList()) =
        OneTree.build(billing, rows, plan, worktrees, items, filter)

    @Test
    fun `the root is the themes in plan order, then No theme, and below them the checkout and loose worktrees`() {
        val tree = tree()
        assertEquals(listOf("Visual language", "Mac navigation", "Reliability", "Phone parity", "Plan layer", "No theme"), tree.work.map { it.title })
        assertEquals(listOf("Main checkout", "Loose worktrees"), tree.below.map { it.title })
        assertEquals("4/18", tree.work[0].detail)
    }

    @Test
    fun `a theme holds its open cards, then its done ones folded`() {
        val visual = tree().work[0]
        assertEquals("ov-216", visual.children.first().key)
        val fold = visual.children.last { it.kind == OneTree.Kind.DONE_FOLD }
        assertEquals("4 done", fold.title)
        assertTrue(visual.children.filter { it.kind == OneTree.Kind.TASK }.none { it.quiet })
    }

    @Test
    fun `a card's lane is its worktree, with the panes in it, not a changes pane, and its builder`() {
        val tree = tree()
        val lane = tree.all.first { it.target == OneTree.Target.Lane(plan.lanes.first { l -> l.name == "mac-vis" }.id) }
        assertEquals("Building", lane.detail)
        assertEquals(listOf("c1", "c2"), lane.children.filter { it.kind == OneTree.Kind.TERMINAL }.map { (it.target as OneTree.Target.Terminal).terminal })
        assertEquals("Agent", lane.children.first().detail)
        val builder = lane.children.single { it.kind == OneTree.Kind.SUBAGENT }
        assertEquals("Builder · Opus", builder.title)
        assertEquals(OneTree.Words.SUBAGENT_CAPTION, builder.caption)
        assertTrue(lane.also.startsWith("also ov-"))
    }

    @Test
    fun `a card's own worktree hangs under it, and only an unreached one is loose`() {
        val tree = tree()
        val bil9 = tree.all.first { it.target == OneTree.Target.Task("t9") }
        assertEquals(listOf(OneTree.Target.Worktree("wt-hooks")), bil9.children.map { it.target })
        val loose = tree.below[1]
        // Hidden, so in Loose worktrees' own closed Hidden group, as on the iPhone.
        assertEquals(listOf("Hidden"), loose.children.map { it.title })
        assertEquals(listOf("spike"), loose.children[0].children.map { it.title })
        assertFalse(tree.all.any { it.title == "other" })
    }

    @Test
    fun `the checkout lists the project's own shells, not the orchestrator or a task's agent`() {
        val main = tree().below[0]
        assertEquals(listOf(OneTree.Target.Terminal("wt-main", "m1")), main.children.map { it.target })
        assertEquals("1 shell", main.detail)
    }

    @Test
    fun `a theme's ask and a card's item put dots on them, and a dot rolls up to the row closed over it`() {
        val item = NeedsYouItem(id = "decision:1", kind = "decision", workspaceId = "ws-billing", task = TaskRef("t7"))
        val elsewhere = NeedsYouItem(id = "decision:2", kind = "decision", workspaceId = "ws-other", task = TaskRef("t9"))
        val tree = tree(items = listOf(item, elsewhere))
        assertTrue(tree.work[0].asks)  // Visual language asks the owner
        assertFalse(tree.work[1].showsDot)
        val noTheme = tree.work.last()
        assertFalse(noTheme.asks)
        assertTrue(noTheme.showsDot)
        assertFalse(tree.all.first { it.target == OneTree.Target.Task("t9") }.asks)
    }

    @Test
    fun `In review leaves the cards in review, and themes with none go`() {
        val tree = tree(OneTree.Filter.IN_REVIEW)
        assertFalse(tree.work.any { it.title == "No theme" })
        assertTrue(tree.work.flatMap { it.children }.all { it.kind == OneTree.Kind.TASK })
    }

    @Test
    fun `a row with children pushes, a leaf opens, a subagent opens nothing`() {
        val tree = tree()
        assertEquals(OneTree.Tap.Push(tree.work[0].id), OneTree.tap(tree.work[0]))
        val shell = tree.all.first { it.target == OneTree.Target.Terminal("wt-hooks", "s1") }
        assertEquals(OneTree.Tap.Open(shell.target!!), OneTree.tap(shell))
        val builder = tree.all.first { it.kind == OneTree.Kind.SUBAGENT }
        assertEquals(OneTree.Tap.None, OneTree.tap(builder))
        assertEquals("Theme page", OneTree.ownRow(tree.work[0]))
        assertNull(OneTree.ownRow(tree.work.last()))
        assertEquals(tree.work[0], tree.node(tree.work[0].id))
    }

    @Test
    fun `a level not found waits for the board and the plan, and is gone only once both came back without it`() {
        assertEquals(OneTree.LevelState.NODE, OneTree.level(true, OneTree.Read.PENDING, OneTree.Read.PENDING))
        assertEquals(OneTree.LevelState.LOADING, OneTree.level(false, OneTree.Read.READ, OneTree.Read.PENDING))
        assertEquals(OneTree.LevelState.LOADING, OneTree.level(false, OneTree.Read.PENDING, OneTree.Read.NOT_KEPT))
        assertEquals(OneTree.LevelState.FAILED, OneTree.level(false, OneTree.Read.READ, OneTree.Read.FAILED))
        assertEquals(OneTree.LevelState.GONE, OneTree.level(false, OneTree.Read.READ, OneTree.Read.READ))
        assertEquals(OneTree.LevelState.GONE, OneTree.level(false, OneTree.Read.READ, OneTree.Read.NOT_KEPT))
    }

    // ---- the strip ----

    @Test
    fun `the strip says what needs you, two Now lanes and how many more, and the next lane`() {
        val strip = PlanStrip.of(plan, needsYou = 2, orchestrator = Terminal(id = "o", state = "running", activity = "idle", role = "orchestrator"))
        assertEquals(listOf("2 need you", "Mac looks like one app Building", "A read-only Files\u2026 In review", "+4", "next: Plan view on iOS and\u2026"), strip.parts)
        assertEquals(PlanStrip.Orchestrator.IDLE, strip.orchestrator)
        assertTrue(strip.accessibilityLabel.startsWith("Orchestrator, idle. 2 need you"))
        assertTrue(PlanStrip.of(Plan(), 0, null).isEmpty)
        assertEquals("1 needs you", PlanStrip.of(Plan(), 1, null).text)
    }

    @Test
    fun `the orchestrator's state and line come from its pane`() {
        fun t(activity: String?, state: String = "running", extra: (Terminal) -> Terminal = { it }) =
            extra(Terminal(id = "o", state = state, activity = activity, role = "orchestrator"))
        assertEquals(PlanStrip.Orchestrator.STOPPED, PlanStrip.state(t("working", "lost")))
        assertEquals(PlanStrip.Orchestrator.NEEDS_YOU, PlanStrip.state(t("blocked")))
        assertEquals(PlanStrip.Orchestrator.FAILED, PlanStrip.state(t("done") { it.copy(turnFailed = true) }))
        val blocked = t("blocked") { it.copy(blockedQuestion = "Run the migration?", line = "Waiting") }
        assertEquals("Run the migration?", PlanStrip.line(blocked, PlanStrip.Orchestrator.NEEDS_YOU))
        val headline = t("working") { it.copy(line = "claude 4m", headline = "claude 4m", said = "Read the plan.") }
        assertEquals("Read the plan.", PlanStrip.line(headline, PlanStrip.Orchestrator.WORKING))
    }

    @Test
    fun `the count is this workspace's items and its themes' asks, the column's before the list is read`() {
        val mine = NeedsYouItem(id = "ask:1", kind = "ask", workspaceId = "ws-billing")
        val theirs = NeedsYouItem(id = "ask:2", kind = "ask", workspaceId = "ws-other")
        assertEquals(3, PlanStrip.needsYouCount(billing, rows, plan, RunnerNeedsYou(listOf(mine, theirs))))
        assertEquals(1 + 0 + 2, PlanStrip.needsYouCount(billing, rows, plan, null))
    }

    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}.")
    }
}
