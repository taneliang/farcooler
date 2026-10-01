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
        // A saved terminal that has since closed keeps its worktree.
        #expect(
            WorkspaceSelection.mapping(old: .terminal(host: "", worktree: "scratch", terminal: "closed"), in: fleet)
                == .workspace(host: "", workspace: Self.billing, focus: .worktree("scratch", terminal: nil)))
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

    // MARK: - Where the window reopens

    private static func scratchDefaults() -> UserDefaults {
        let name = "fc-selection-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    /// `fleet.lastTerminal` is read once its runner has been read, mapped by
    /// the table, written as `workspace.lastSelection`, and removed; a second
    /// launch finds nothing to migrate and leaves the new key alone. A key
    /// naming a terminal that's gone is removed and leaves nothing.
    @Test("The old last-terminal key migrates once and is removed")
    func theOldLastTerminalKeyMigratesOnceAndIsRemoved() {
        let defaults = Self.scratchDefaults()
        defaults.set("/lane/agent", forKey: SelectionMemory.legacyKey)
        #expect(!SelectionMemory.migrate(defaults, fleet: Self.fleet(), ready: { _ in false }))
        #expect(defaults.string(forKey: SelectionMemory.legacyKey) == "/lane/agent", "migrated before its runner was read")

        #expect(SelectionMemory.migrate(defaults, fleet: Self.fleet(), ready: { _ in true }))
        #expect(defaults.string(forKey: SelectionMemory.legacyKey) == nil)
        let saved = defaults.string(forKey: SelectionMemory.key)
        #expect(saved.flatMap(SelectionMemory.decode) == .workspace(host: "", workspace: Self.billing, focus: .task(Self.task)))

        // Once: the new key isn't overwritten by an old one written again.
        defaults.set("/checkout/conductor", forKey: SelectionMemory.legacyKey)
        SelectionMemory.migrate(defaults, fleet: Self.fleet(), ready: { _ in true })
        #expect(defaults.string(forKey: SelectionMemory.key) == saved)
        #expect(defaults.string(forKey: SelectionMemory.legacyKey) == nil)

        let gone = Self.scratchDefaults()
        gone.set("/gone/t", forKey: SelectionMemory.legacyKey)
        #expect(SelectionMemory.migrate(gone, fleet: Self.fleet(), ready: { _ in true }))
        #expect(gone.string(forKey: SelectionMemory.legacyKey) == nil)
        #expect(gone.string(forKey: SelectionMemory.key) == nil)

        // Every shape round-trips.
        for selection: Selection in [
            .workspace(host: "", workspace: Self.billing, focus: nil),
            .workspace(host: "me@box", workspace: Self.billing, focus: .task(Self.task)),
            .workspace(host: "", workspace: Self.main, focus: .worktree("checkout", terminal: "shell")),
            .workspace(host: "", workspace: Self.main, focus: .worktree("checkout", terminal: nil)),
            .looseWorktree(host: "", worktree: "stray", terminal: "stray-shell"),
            .looseWorktree(host: "", worktree: "stray", terminal: nil),
        ] {
            #expect(SelectionMemory.encode(selection).flatMap(SelectionMemory.decode) == selection)
        }
        #expect(SelectionMemory.encode(.needsYou) == nil)
    }

    /// Needs You when anything is waiting, as the iPhone opens (ruling 4).
    /// Otherwise, once every runner has said, the last workspace selection
    /// while it's still there, else the first workspace; and nothing is
    /// decided while a runner hasn't said yet, so the window doesn't open on
    /// a workspace a moment before a decision arrives.
    @Test("Launch opens Needs You when it has items, else the last workspace")
    func launchOpensNeedsYouWhenItHasItemsElseTheLastWorkspace() {
        let fleet = Self.fleet()
        let last = Selection.workspace(host: "", workspace: Self.billing, focus: .worktree("scratch", terminal: nil))
        #expect(SelectionMemory.launch(needsYou: 2, settled: false, last: last, in: fleet) == .some(.needsYou))
        #expect(SelectionMemory.launch(needsYou: 0, settled: false, last: last, in: fleet) == nil)
        #expect(SelectionMemory.launch(needsYou: 0, settled: true, last: last, in: fleet) == .some(last))
        // Its opened worktree gone: the workspace, with the column closed.
        let goneLane = Selection.workspace(host: "", workspace: Self.billing, focus: .worktree("gone", terminal: nil))
        #expect(
            SelectionMemory.launch(needsYou: 0, settled: true, last: goneLane, in: fleet)
                == .some(.workspace(host: "", workspace: Self.billing, focus: nil)))
        // Its workspace gone, or nothing saved: the first workspace listed.
        let gone = Selection.workspace(host: "", workspace: "0198f2c0-0000-7000-8000-0000000000ee", focus: nil)
        #expect(
            SelectionMemory.launch(needsYou: 0, settled: true, last: gone, in: fleet)
                == .some(.workspace(host: "", workspace: Self.main, focus: nil)))
        #expect(
            SelectionMemory.launch(needsYou: 0, settled: true, last: nil, in: fleet)
                == .some(.workspace(host: "", workspace: Self.main, focus: nil)))
        // And a runner still making its first connection holds it open.
        #expect(!FleetStore.settled([(.connected, true), (.connecting, false)]))
        #expect(FleetStore.settled([(.connected, true), (.unreachable(reason: "x"), false)]))
        #expect(!FleetStore.settled([(.connected, false)]))
    }

    /// A visit to a workspace ends when it's left, not when a task or a
    /// worktree in it is drilled into or come back up from (ov-79, for
    /// ov-80's "Since you were last here").
    @Test("Drilling in and out stays in a workspace; anything else leaves it")
    func drillingInAndOutStaysInAWorkspace() {
        let top = ContentView.Selection.workspace(host: "", workspace: "ws", focus: nil)
        let task = ContentView.Selection.workspace(host: "", workspace: "ws", focus: .task("t"))
        let lane = ContentView.Selection.workspace(host: "", workspace: "ws", focus: .worktree("w", terminal: nil))
        let other = ContentView.Selection.workspace(host: "", workspace: "billing", focus: nil)
        let elsewhere = ContentView.Selection.workspace(host: "mini", workspace: "ws", focus: nil)
        let stays = [(top, task), (task, lane), (lane, top), (task, top)].map { WorkspaceSelection.leaves($0, for: $1) }
        let goes = [other, elsewhere, .needsYou, .looseWorktree(host: "", worktree: "w", terminal: nil), nil]
            .map { WorkspaceSelection.leaves(task, for: $0) }
        #expect(stays == [false, false, false, false])
        #expect(goes == [true, true, true, true, true])
        #expect(!WorkspaceSelection.leaves(.needsYou, for: top))
    }
}
