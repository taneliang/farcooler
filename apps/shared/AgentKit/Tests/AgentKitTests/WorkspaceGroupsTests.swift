import Foundation
import Testing

@testable import AgentKit

/// The one grouping rule the Mac and the phone share, and the board notice
/// that decides which workspace's board reads again. Android's
/// `WorkspaceGroupsTest.kt` holds the same cases.
struct WorkspaceGroupsTests {
    let main = WorkspaceSummary(id: "m", name: "Main", taskPrefix: "ov", isMain: true, ordinal: 1)
    let billing = WorkspaceSummary(id: "b", name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 0)

    @Test func mainComesFirstAndAWorkspaceWithNothingIsStillShown() {
        let g = WorkspaceGrouping.group(
            repository: "r", workspaces: [billing, main],
            worktrees: [("w1", "m")], orchestrators: [:])
        #expect(g.workspaces.map(\.workspace.id) == ["m", "b"])
        #expect(g.workspaces[1].worktrees.isEmpty)
    }

    /// After Main, the runner's ordinal decides, not the order listed.
    @Test func theRestFollowTheirOrdinal() {
        let tax = WorkspaceSummary(id: "t", name: "Tax", taskPrefix: "tax", isMain: false, ordinal: 2)
        let g = WorkspaceGrouping.group(
            repository: "r", workspaces: [tax, main, billing], worktrees: [], orchestrators: [:])
        #expect(g.workspaces.map(\.id) == ["m", "b", "t"])
    }

    @Test func unclaimedWorktreesAreTheirOwnGroup() {
        let g = WorkspaceGrouping.group(
            repository: "r", workspaces: [main],
            worktrees: [("w1", "m"), ("w2", nil)], orchestrators: [:])
        #expect(g.unclaimed == ["w2"])
        #expect(g.workspaces[0].worktrees == ["w1"])
    }

    @Test func aWorktreeOwnedByAnUnknownWorkspaceIsUnclaimedRatherThanLost() {
        let g = WorkspaceGrouping.group(
            repository: "r", workspaces: [main],
            worktrees: [("w1", "gone")], orchestrators: [:])
        #expect(g.unclaimed == ["w1"])
    }

    /// A workspace's orchestrator is the one the caller found running, or
    /// else the one the runner named on the workspace itself.
    @Test func anOrchestratorComesFromTheMapOrTheWorkspace() {
        let named = WorkspaceSummary(
            id: "b", name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1,
            orchestrator: "t9")
        let g = WorkspaceGrouping.group(
            repository: "r", workspaces: [main, named], worktrees: [], orchestrators: ["m": "t1"])
        #expect(g.workspaces.map(\.orchestrator) == ["t1", "t9"])
    }

    /// A runner without `workstreams` lists no workspaces, and gets today's
    /// layout: one implicit workspace per repository, holding every
    /// worktree, with nothing unclaimed. Its id is the repository's, and its
    /// board is read without naming a workspace.
    @Test func aRunnerWithoutWorkspacesKeepsTodaysLayout() {
        let g = WorkspaceGrouping.group(
            repository: "r", workspaces: [],
            worktrees: [("w1", nil), ("w2", nil)], orchestrators: [:])
        #expect(g.workspaces.count == 1)
        #expect(g.workspaces[0].worktrees == ["w1", "w2"])
        #expect(g.unclaimed.isEmpty)
        let only = g.workspaces[0].workspace
        #expect(only.isImplicit)
        #expect(only.id == "r")
        #expect(only.boardWorkspace == nil)
        #expect(!main.isImplicit)
        #expect(main.boardWorkspace == "m")
    }

    // ---- which board reads again ----

    private func board(_ id: String, _ repository: String = "r") -> WorkspaceSummary {
        WorkspaceSummary(
            id: id, name: id, taskPrefix: id, isMain: false, ordinal: 0, repository: repository)
    }

    /// Two boards in one repository: news naming Billing re-reads Billing only.
    @Test func aBoardReadsAgainForItsOwnNewsAndNotAnothers() {
        let news = BoardNotice(repository: "r", workspace: "b")
        #expect(news.touches(board("b")))
        #expect(!news.touches(board("m")), "Main re-read for Billing's change")
    }

    /// A move changes two boards, and both read again.
    @Test func aMoveReadsBothBoardsAgain() {
        let moved = BoardNotice(repository: "r", workspace: "m", fromWorkspace: "b")
        #expect(moved.touches(board("m")))
        #expect(moved.touches(board("b")))
        #expect(!moved.touches(board("t")))
    }

    /// A runner without `workstreams` names no workspace: every board in
    /// that repository reads again — its one implicit board — and no board
    /// in another repository does.
    @Test func newsFromARunnerWithoutWorkspacesReachesItsRepositorysBoard() {
        let news = BoardNotice(repository: "r", workspace: nil)
        #expect(news.touches(WorkspaceSummary.implicit(repository: "r")))
        #expect(!news.touches(WorkspaceSummary.implicit(repository: "q")))
    }

    /// The line the client core queues, read as the FFI writes it
    /// (`event_line` in `crates/client/src/ffi.rs`): null is absent, and a
    /// notice that is not board news is nothing.
    @Test func aNoticeIsReadFromTheCoresLine() throws {
        let line = #"""
        {"event":"task","repository":"r","workspace":"m","from_workspace":"b","actor":"user"}
        """#
        let object = try #require(
            try JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any])
        #expect(
            BoardNotice(notice: object)
                == BoardNotice(repository: "r", workspace: "m", fromWorkspace: "b", actor: "user"))

        let old = try #require(
            try JSONSerialization.jsonObject(
                with: Data(#"{"event":"task","repository":"r","workspace":null,"actor":"user"}"#.utf8))
                as? [String: Any])
        #expect(BoardNotice(notice: old)?.workspace == nil)
        #expect(BoardNotice(notice: ["event": "fleet"]) == nil)
    }

    // ---- the fleet, grouped ----

    /// The phone's fleet grouped: the orchestrator is found by its role, and
    /// a worktree is placed by its `workspace`.
    @Test func aPhonesFleetGroupsByRepositoryThenWorkspace() throws {
        let fleet = try FleetDecodeTests.decodeFleet(Self.twoWorkspaces)
        let groups = fleet.repositoryGroups()
        #expect(groups.map(\.repository) == ["r1"])
        let g = try #require(groups.first)
        #expect(g.workspaces.map(\.workspace.name) == ["Main", "Billing"])
        #expect(g.workspaces[0].worktrees == ["w1"])
        #expect(g.workspaces[0].orchestrator == "o1")
        #expect(g.workspaces[1].worktrees.isEmpty)
        #expect(g.unclaimed == ["w2"])
    }

    static let twoWorkspaces = """
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
    """
}
