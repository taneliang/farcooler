import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Where every old selection lands now that a workspace is a place (spec
/// §4.2's table), which is also where "go to this terminal" lands from the
/// palette, the attention cycle and a board's Go to Agent.
@MainActor
struct WorkspaceSelectionTests {
    private typealias Selection = ContentView.Selection

    private static let repo = "0198f2c0-0000-7000-8000-0000000000aa"
    private static let main = "0198f2c0-0000-7000-8000-0000000000cc"
    private static let billing = "0198f2c0-0000-7000-8000-0000000000dd"
    private static let task = "0198f2c0-0000-7000-8000-00000000b009"

    private static func terminal(
        _ id: String, preset: String = "zsh", taskId: String? = nil, role: String? = nil,
        workspace: String? = nil
    ) -> Terminal {
        var t = Terminal(id: id, short: id, title: id, preset: preset, state: "running", epoch: 0)
        t.taskId = taskId
        t.role = role
        t.workspace = workspace
        return t
    }

    private static func worktree(
        _ id: String, workspace: String?, repository: String? = repo, terminals: [Terminal],
        openTasks: [NeedsYouTask]? = nil
    ) -> Worktree {
        Worktree(
            id: id, short: id, task: id, branch: "feat/\(id)", repository: "overnight", host: "",
            path: "/tmp/\(id)", state: "active", terminals: terminals, repositoryID: repository,
            workspace: workspace, openTasks: openTasks)
    }

    private static let workspaces = [
        WorkspaceSummary(id: main, name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0, repository: repo),
        WorkspaceSummary(
            id: billing, name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: repo,
            orchestrator: "conductor"),
    ]

    /// This Mac's fleet: the main checkout with Billing's orchestrator and a
    /// shell, Billing's lane with a dispatched agent, a claimed worktree with
    /// a shell and no task, and an unclaimed one.
    private static func fleet(workspaces: [WorkspaceSummary]? = workspaces) -> Fleet {
        var fleet = Fleet(
            runtimeHealthy: true, livePanes: 0,
            worktrees: [
                worktree(
                    "checkout", workspace: main,
                    terminals: [
                        terminal("conductor", preset: "claude", role: "orchestrator", workspace: billing),
                        terminal("shell"),
                    ]),
                worktree(
                    "lane", workspace: billing,
                    terminals: [
                        terminal("agent", preset: "claude", taskId: task, role: "agent", workspace: billing)
                    ],
                    openTasks: [NeedsYouTask(id: task, key: "bil-9", title: "Invoice PDF export", status: "in_progress")]),
                worktree("scratch", workspace: billing, terminals: [terminal("scratch-shell")]),
                worktree("stray", workspace: nil, terminals: [terminal("stray-shell")]),
            ],
            branchPrefix: nil)
        if let workspaces { fleet.runnerWorkspaces[""] = workspaces }
        return fleet
    }

    @Test("A board selection becomes its workspace")
    func aBoardSelectionBecomesItsWorkspace() {
        #expect(
            WorkspaceSelection.mapping(old: .board(host: "", workspace: Self.billing), in: Self.fleet())
                == .workspace(host: "", workspace: Self.billing, focus: nil))
    }

    /// Its conversation column is where it's drawn: the workspace it leads,
    /// not Main, whose checkout it runs in.
    @Test("An orchestrator's terminal becomes its workspace with no focus")
    func anOrchestratorsTerminalBecomesItsWorkspaceWithNoFocus() {
        #expect(
            WorkspaceSelection.mapping(
                old: .terminal(host: "", worktree: "checkout", terminal: "conductor"), in: Self.fleet())
                == .workspace(host: "", workspace: Self.billing, focus: nil))
    }

    /// Its task's column, which holds it: by its own `taskId`, and, for a
    /// shell in a worktree with one open task, by `TaskLink`'s fallback.
    @Test("A dispatched agent's terminal becomes its task")
    func aDispatchedAgentsTerminalBecomesItsTask() {
        let fleet = Self.fleet()
        #expect(
            WorkspaceSelection.mapping(old: .terminal(host: "", worktree: "lane", terminal: "agent"), in: fleet)
                == .workspace(host: "", workspace: Self.billing, focus: .task(Self.task)))
        #expect(
            WorkspaceSelection.landing(
                on: PaneRef(host: "", worktree: "lane", terminal: "agent"), in: fleet)
                == .workspace(host: "", workspace: Self.billing, focus: .task(Self.task)))
    }

    @Test("A shell in a claimed worktree becomes that worktree under its owner")
    func aShellInAClaimedWorktreeBecomesThatWorktreeUnderItsOwner() {
        let fleet = Self.fleet()
        #expect(
            WorkspaceSelection.mapping(
                old: .terminal(host: "", worktree: "scratch", terminal: "scratch-shell"), in: fleet)
                == .workspace(host: "", workspace: Self.billing, focus: .worktree("scratch", terminal: "scratch-shell")))
        #expect(
            WorkspaceSelection.mapping(old: .terminal(host: "", worktree: "checkout", terminal: "shell"), in: fleet)
                == .workspace(host: "", workspace: Self.main, focus: .worktree("checkout", terminal: "shell")))
        #expect(
            WorkspaceSelection.mapping(old: .worktree(host: "", id: "scratch"), in: fleet)
                == .workspace(host: "", workspace: Self.billing, focus: .worktree("scratch", terminal: nil)))
        #expect(
            WorkspaceSelection.mapping(old: .terminal(host: "", worktree: "gone", terminal: "t"), in: fleet) == nil)
    }

    /// And one whose owner the runner no longer lists, likewise.
    @Test("A terminal in an unclaimed worktree becomes a loose worktree")
    func aTerminalInAnUnclaimedWorktreeBecomesALooseWorktree() {
        #expect(
            WorkspaceSelection.mapping(
                old: .terminal(host: "", worktree: "stray", terminal: "stray-shell"), in: Self.fleet())
                == .looseWorktree(host: "", worktree: "stray", terminal: "stray-shell"))
        let withoutBilling = Self.fleet(workspaces: [Self.workspaces[0]])
        #expect(
            WorkspaceSelection.mapping(old: .worktree(host: "", id: "scratch"), in: withoutBilling)
                == .looseWorktree(host: "", worktree: "scratch", terminal: nil))
    }

    /// A runner without `workstreams` has one implicit workspace per
    /// repository, whose id is the repository's; only a CLI too old to say
    /// which repository leaves a worktree loose.
    @Test("A runner without workstreams maps to its repository's implicit workspace")
    func aRunnerWithoutWorkstreamsMapsToItsRepositorysImplicitWorkspace() {
        let older = Self.fleet(workspaces: nil)
        #expect(
            WorkspaceSelection.mapping(old: .terminal(host: "", worktree: "stray", terminal: "stray-shell"), in: older)
                == .workspace(host: "", workspace: Self.repo, focus: .worktree("stray", terminal: "stray-shell")))
        #expect(
            WorkspaceSelection.mapping(old: .terminal(host: "", worktree: "lane", terminal: "agent"), in: older)
                == .workspace(host: "", workspace: Self.repo, focus: .task(Self.task)))
        var nameless = older
        nameless.worktrees[3].repositoryID = nil
        #expect(
            WorkspaceSelection.mapping(old: .worktree(host: "", id: "stray"), in: nameless)
                == .looseWorktree(host: "", worktree: "stray", terminal: nil))
    }
}
