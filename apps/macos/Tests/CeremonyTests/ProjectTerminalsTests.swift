import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The navigator's Terminals section (ov-178, from ov-190): a repository's
/// own terminals, the ones started by hand in its main checkout, beside the
/// task list and never in it.
@MainActor
struct ProjectTerminalsTests {
    private static let repo = "0198f2c0-0000-7000-8000-0000000000aa"
    private static let other = "0198f2c0-0000-7000-8000-0000000000bb"
    private static let main = WorkspaceSummary(
        id: "ws-main", name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0, repository: repo)
    private static let billing = WorkspaceSummary(
        id: "ws-bil", name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: repo,
        orchestrator: "conductor")

    private static func terminal(
        _ id: String, preset: String = "zsh", state: String = "running", role: String? = "shell",
        task: String? = nil, workspace: String? = nil, mode: String? = nil
    ) -> Terminal {
        var t = Terminal(id: id, short: id, title: id, preset: preset, state: state, epoch: 0)
        t.role = role
        t.taskId = task
        t.workspace = workspace
        t.paneMode = mode
        return t
    }

    private static func checkout(_ terminals: [Terminal], repository: String = repo, host: String = "") -> Worktree {
        var w = Worktree(
            id: "checkout-\(repository.suffix(2))", short: "checkout", task: "overnight", branch: "main",
            repository: "overnight", host: host, path: "/tmp/overnight", state: "active", terminals: terminals,
            repositoryID: repository, workspace: nil)
        w.is_main_checkout = true
        return w
    }

    private static func fleet(_ worktrees: [Worktree], workspaces: [WorkspaceSummary] = [main, billing]) -> Fleet {
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: worktrees, branchPrefix: nil)
        fleet.runnerWorkspaces[""] = workspaces
        return fleet
    }

    /// The project's are what's started by hand there. The orchestrator is
    /// its workspace's conversation, seated or stopped; a task's agent is on
    /// its task's row; a changes pane belongs to its diff. A lost shell is
    /// still the project's, and listed, so it can be opened.
    @Test("The section lists the checkout's own terminals, never an orchestrator, a task's agent or a changes pane")
    func theSectionListsOnlyTheProjectsTerminals() {
        var stopped = Self.terminal("old-conductor", preset: "claude", state: "exited", role: "orchestrator", workspace: "ws-main")
        stopped.title = "orchestrator"
        let checkout = Self.checkout([
            Self.terminal("conductor", preset: "claude", role: "orchestrator", workspace: "ws-bil"),
            stopped,
            Self.terminal("agent", preset: "codex", role: "agent", task: "t-1"),
            Self.terminal("diff", preset: "farcooler", role: nil, mode: "changes"),
            Self.terminal("proxy"),
            Self.terminal("tail", state: "lost"),
        ])
        let fleet = Self.fleet([checkout])
        #expect(ProjectTerminals.terminals(in: checkout, fleet: fleet).map(\.id) == ["proxy", "tail"])
    }

    /// Every workspace of the repository shares its main checkout, so each
    /// one's navigator finds it; another repository's, or another runner's,
    /// is not this one's. A runner without workspaces keys its one implicit
    /// workspace by the repository.
    @Test("Every workspace of the repository finds its checkout, and no other")
    func everyWorkspaceFindsItsCheckout() {
        let mine = Self.checkout([Self.terminal("proxy")])
        let theirs = Self.checkout([Self.terminal("db")], repository: Self.other)
        let remote = Self.checkout([Self.terminal("ssh")], host: "build-box")
        let fleet = Self.fleet([theirs, remote, mine])
        #expect(ProjectTerminals.checkout(for: Self.main, host: "", in: fleet)?.id == mine.id)
        #expect(ProjectTerminals.checkout(for: Self.billing, host: "", in: fleet)?.id == mine.id)
        #expect(ProjectTerminals.checkout(for: .implicit(repository: Self.repo), host: "", in: fleet)?.id == mine.id)
        #expect(ProjectTerminals.checkout(for: Self.main, host: "build-box", in: fleet)?.id == remote.id)
        // A plain worktree of the repository is never the checkout.
        var lane = Self.checkout([Self.terminal("dev")])
        lane.is_main_checkout = false
        #expect(ProjectTerminals.checkout(for: Self.main, host: "", in: Self.fleet([lane])) == nil)
    }

    /// Nothing starts one for you (ov-190): with none running, the section
    /// is there only to offer New Terminal, and not at all where the runner
    /// can't take one.
    @Test("The section appears only with project terminals, or to offer starting one")
    func theSectionAppearsOnlyWithTerminalsOrAnOffer() {
        #expect(!ProjectTerminals.none.isShown)
        #expect(!ProjectTerminals(checkout: Self.checkout([]), terminals: [], onNew: nil).isShown)
        #expect(ProjectTerminals(checkout: Self.checkout([]), terminals: [], onNew: {}).isShown)
        #expect(ProjectTerminals(checkout: Self.checkout([]), terminals: [Self.terminal("proxy")], onNew: nil).isShown)
    }

    /// The navigator's filter narrows them by name or command, as it does
    /// tasks and worktrees, and drops New Terminal, which would read as a
    /// hit.
    @Test("The filter narrows the section and drops New Terminal")
    func theFilterNarrowsTheSection() {
        var proxy = Self.terminal("proxy")
        proxy.title = "gcp proxy"
        let section = ProjectTerminals(
            checkout: Self.checkout([]), terminals: [proxy, Self.terminal("tail")], onNew: {})
        let narrowed = section.narrowed(by: "gcp")
        #expect(narrowed.terminals.map(\.id) == ["proxy"])
        #expect(narrowed.onNew == nil)
        #expect(!section.narrowed(by: "nothing like it").isShown)
        let unfiltered = section.narrowed(by: "  ")
        #expect(unfiltered.terminals.count == 2 && unfiltered.onNew != nil)
    }

    /// Its own collapsible section, remembered closed per board like the
    /// others, between Tasks and Worktrees.
    @Test("Terminals is a navigator section between Tasks and Worktrees")
    func terminalsIsANavigatorSection() {
        #expect(TaskBoardView.navigatorSections == ["tasks", "terminals", "worktrees"])
    }
}
