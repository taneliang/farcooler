package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The one grouping rule this phone shares with the iPhone and the Mac, and the
 * board notice that decides which workspace's board reads again. AgentKit's
 * `WorkspaceGroupsTests.swift` holds the same cases.
 */
class WorkspaceGroupsTest {
    private val main = WorkspaceSummary(id = "m", name = "Main", taskPrefix = "ov", isMain = true, ordinal = 1)
    private val billing = WorkspaceSummary(id = "b", name = "Billing", taskPrefix = "bil", isMain = false, ordinal = 0)

    @Test
    fun mainComesFirstAndAWorkspaceWithNothingIsStillShown() {
        val g = WorkspaceGrouping.group("r", listOf(billing, main), listOf(WorktreeClaim("w1", "m")), emptyMap())
        assertEquals(listOf("m", "b"), g.workspaces.map { it.workspace.id })
        assertTrue(g.workspaces[1].worktrees.isEmpty())
    }

    /** After Main, the runner's ordinal decides, not the order listed. */
    @Test
    fun theRestFollowTheirOrdinal() {
        val tax = WorkspaceSummary(id = "t", name = "Tax", taskPrefix = "tax", ordinal = 2)
        val g = WorkspaceGrouping.group("r", listOf(tax, main, billing), emptyList(), emptyMap())
        assertEquals(listOf("m", "b", "t"), g.workspaces.map { it.id })
    }

    @Test
    fun unclaimedWorktreesAreTheirOwnGroup() {
        val g = WorkspaceGrouping.group(
            "r", listOf(main), listOf(WorktreeClaim("w1", "m"), WorktreeClaim("w2", null)), emptyMap(),
        )
        assertEquals(listOf("w2"), g.unclaimed)
        assertEquals(listOf("w1"), g.workspaces[0].worktrees)
    }

    @Test
    fun aWorktreeOwnedByAnUnknownWorkspaceIsUnclaimedRatherThanLost() {
        val g = WorkspaceGrouping.group("r", listOf(main), listOf(WorktreeClaim("w1", "gone")), emptyMap())
        assertEquals(listOf("w1"), g.unclaimed)
    }

    @Test
    fun anOrchestratorComesFromTheMapOrTheWorkspace() {
        val named = billing.copy(ordinal = 1, orchestrator = "t9")
        val g = WorkspaceGrouping.group("r", listOf(main, named), emptyList(), mapOf("m" to "t1"))
        assertEquals(listOf("t1", "t9"), g.workspaces.map { it.orchestrator })
    }

    /**
     * A runner without `workstreams` lists no workspaces and keeps the flat
     * layout: one implicit workspace per repository holding every worktree,
     * nothing unclaimed, and a board read without naming a workspace.
     */
    @Test
    fun aRunnerWithoutWorkspacesKeepsTodaysLayout() {
        val g = WorkspaceGrouping.group(
            "r", emptyList(), listOf(WorktreeClaim("w1", null), WorktreeClaim("w2", null)), emptyMap(),
        )
        assertEquals(1, g.workspaces.size)
        assertEquals(listOf("w1", "w2"), g.workspaces[0].worktrees)
        assertTrue(g.unclaimed.isEmpty())
        val only = g.workspaces[0].workspace
        assertTrue(only.isImplicit)
        assertEquals("r", only.id)
        assertNull(only.boardWorkspace)
        assertFalse(main.isImplicit)
        assertEquals("m", main.boardWorkspace)
    }

    // ---- which board reads again ----

    private fun board(id: String, repository: String = "r") =
        WorkspaceSummary(id = id, name = id, taskPrefix = id, repository = repository)

    /** Two boards in one repository: news naming Billing re-reads Billing only. */
    @Test
    fun aBoardReadsAgainForItsOwnNewsAndNotAnothers() {
        val news = BoardNotice(repository = "r", workspace = "b")
        assertTrue(news.touches(board("b")))
        assertFalse("Main re-read for Billing's change", news.touches(board("m")))
    }

    @Test
    fun aMoveReadsBothBoardsAgain() {
        val moved = BoardNotice(repository = "r", workspace = "m", fromWorkspace = "b")
        assertTrue(moved.touches(board("m")))
        assertTrue(moved.touches(board("b")))
        assertFalse(moved.touches(board("t")))
    }

    @Test
    fun newsFromARunnerWithoutWorkspacesReachesItsRepositorysBoard() {
        val news = BoardNotice(repository = "r", workspace = null)
        assertTrue(news.touches(WorkspaceSummary.implicit("r")))
        assertFalse(news.touches(WorkspaceSummary.implicit("q")))
    }

    /**
     * An implicit board is the whole repository's, so news naming a workspace
     * in that repository reaches it too: a runner upgraded under a connected
     * app, or a board a route saved before workspaces restored.
     */
    @Test
    fun anImplicitBoardReadsAgainForNewsThatNamesAWorkspace() {
        val news = BoardNotice(repository = "r", workspace = "m")
        assertTrue(news.touches(WorkspaceSummary.implicit("r")))
        assertFalse(news.touches(WorkspaceSummary.implicit("q")))
    }

    /** The line the client core queues, as `event_line` in `crates/client/src/ffi.rs` writes it. */
    @Test
    fun aNoticeIsReadFromTheCoresLine() {
        fun line(text: String) = Json.parseToJsonElement(text).jsonObject
        assertEquals(
            BoardNotice("r", "m", "b", "user"),
            BoardNotice.of(
                line("""{"event":"task","repository":"r","workspace":"m","from_workspace":"b","actor":"user"}"""),
            ),
        )
        assertNull(
            BoardNotice.of(line("""{"event":"task","repository":"r","workspace":null,"actor":"user"}"""))!!.workspace,
        )
        assertNull(BoardNotice.of(line("""{"event":"fleet"}""")))
    }

    // ---- the fleet, grouped ----

    @Test
    fun aPhonesFleetGroupsByRepositoryThenWorkspace() {
        val fleet = json.decodeFromString(Fleet.serializer(), twoWorkspaces)
        val groups = WorkspaceGrouping.groups(fleet)
        assertEquals(listOf("r1"), groups.map { it.repository })
        val g = groups.single()
        assertEquals(listOf("Main", "Billing"), g.workspaces.map { it.workspace.name })
        assertEquals(listOf("w1"), g.workspaces[0].worktrees)
        assertEquals("o1", g.workspaces[0].orchestrator)
        assertTrue(g.workspaces[1].worktrees.isEmpty())
        assertEquals(listOf("w2"), g.unclaimed)
    }

    /** The same configuration `Connection` decodes a fleet with. */
    private val json = Json { ignoreUnknownKeys = true }

    private val twoWorkspaces = """
        {
          "runtime_healthy": true, "live_panes": 1,
          "workspaces": [
            {"id": "m1", "repository": "r1", "name": "Main", "task_prefix": "ov",
             "is_main": true, "ordinal": 0, "orchestrator": null},
            {"id": "b1", "repository": "r1", "name": "Billing", "task_prefix": "bil",
             "is_main": false, "ordinal": 1, "orchestrator": null}
          ],
          "worktrees": [
            {"id": "w1", "short": "w1", "repository": "r1", "task": "t", "branch": "b",
             "state": "active", "workspace": "m1", "claim_source": "migration",
             "foreign_writers": [],
             "terminals": [
               {"id": "o1", "short": "o1", "title": "", "preset": "claude", "state": "running",
                "epoch": 1, "workspace": "m1", "role": "orchestrator"}
             ]},
            {"id": "w2", "short": "w2", "repository": "r1", "task": "u", "branch": "c",
             "state": "active", "workspace": null, "terminals": []}
          ]
        }
    """.trimIndent()
}
