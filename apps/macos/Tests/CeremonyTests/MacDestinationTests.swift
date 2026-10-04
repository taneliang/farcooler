import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// What the Mac holds, as the resolver reads it, and the selection a
/// destination opens (ov-182, ov-183): the adapter between a window's own
/// vocabulary and the shared `Destination`.
@MainActor
struct MacDestinationTests {
    private typealias Selection = ContentView.Selection

    private static let repo = "0198f2c0-0000-7000-8000-0000000000aa"
    private static let main = "0198f2c0-0000-7000-8000-0000000000cc"
    private static let billing = "0198f2c0-0000-7000-8000-0000000000dd"
    private static let task = "0198f2c0-0000-7000-8000-00000000b009"

    private static func terminal(
        _ id: String, preset: String = "zsh", taskId: String? = nil, role: String? = nil, workspace: String? = nil
    ) -> Terminal {
        var t = Terminal(id: id, short: id, title: id, preset: preset, state: "running", epoch: 0)
        t.taskId = taskId
        t.role = role
        t.workspace = workspace
        return t
    }

    private static func worktree(_ id: String, workspace: String?, host: String = "", terminals: [Terminal]) -> Worktree {
        Worktree(
            id: id, short: id, task: id, branch: "feat/\(id)", repository: "overnight", host: host,
            path: "/tmp/\(id)", state: "active", terminals: terminals, repositoryID: repo, workspace: workspace,
            openTasks: id == "lane"
                ? [NeedsYouTask(id: task, key: "bil-9", title: "Invoice PDF export", status: "in_progress")] : nil)
    }

    private static let workspaces = [
        WorkspaceSummary(id: main, name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0, repository: repo),
        WorkspaceSummary(
            id: billing, name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: repo,
            orchestrator: "conductor"),
    ]

    private static func fleet(workspaces: [WorkspaceSummary]? = workspaces) -> Fleet {
        var fleet = Fleet(
            runtimeHealthy: true, livePanes: 0,
            worktrees: [
                worktree(
                    "checkout", workspace: main,
                    terminals: [Self.terminal("conductor", preset: "claude", role: "orchestrator", workspace: billing)]),
                worktree(
                    "lane", workspace: billing,
                    terminals: [Self.terminal("agent", preset: "claude", taskId: task, role: "agent", workspace: billing)]),
                worktree("stray", workspace: nil, terminals: [Self.terminal("stray-shell")]),
            ],
            branchPrefix: nil)
        if let workspaces { fleet.runnerWorkspaces[""] = workspaces }
        return fleet
    }

    private static let runner = MacDestination.Runner(host: "", runnerId: "r-1", ready: true, loaded: true)

    // MARK: - The world

    @Test("A runner's workspaces are its list; its worktrees carry their owner and their panes")
    func theWorld() throws {
        let world = MacDestination.world(runners: [Self.runner], fleet: Self.fleet())
        let seat = try #require(world.seats.first)
        #expect(seat.host == "")
        #expect(seat.runnerId == "r-1")
        #expect(seat.workspaces?.map(\.id) == [Self.main, Self.billing])
        #expect(seat.workspaces?.map(\.orchestrator) == [false, true])
        let lane = try #require(seat.worktrees?.first { $0.id == "lane" })
        #expect(lane.workspace == Self.billing)
        #expect(lane.terminals.map(\.id) == ["agent"])
        #expect(seat.worktrees?.first { $0.id == "stray" }?.workspace == nil)
        #expect(seat.worktrees?.first { $0.id == "checkout" }?.terminals.first?.orchestrator == true)
    }

    @Test("A runner without workspaces has one implicit workspace per repository, whose id is the repository's")
    func implicitWorkspaces() throws {
        let world = MacDestination.world(runners: [Self.runner], fleet: Self.fleet(workspaces: nil))
        #expect(world.seats.first?.workspaces?.map(\.id) == [Self.repo])
        #expect(world.seats.first?.workspaces?.map(\.orchestrator) == [false])
    }

    @Test("Not read yet is not empty: a runner still coming up claims nothing is gone")
    func unreadIsUnknown() {
        let dialing = MacDestination.Runner(host: "", runnerId: nil, ready: false, loaded: false)
        let unloaded = MacDestination.Runner(host: "", runnerId: "r-1", ready: true, loaded: false)
        for runner in [dialing, unloaded] {
            let seat = MacDestination.world(runners: [runner], fleet: Self.fleet()).seats[0]
            #expect(seat.workspaces == nil)
            #expect(seat.worktrees == nil)
        }
    }

    @Test("Each runner has only its own worktrees")
    func worktreesByRunner() {
        var fleet = Self.fleet()
        fleet.worktrees.append(Self.worktree("far", workspace: nil, host: "studio", terminals: []))
        let studio = MacDestination.Runner(host: "studio", runnerId: "r-2", ready: true, loaded: true)
        let world = MacDestination.world(runners: [Self.runner, studio], fleet: fleet)
        #expect(world.seats[0].worktrees?.map(\.id).contains("far") == false)
        #expect(world.seats[1].worktrees?.map(\.id) == ["far"])
    }

    // MARK: - A selection, kept and opened

    /// **Every selection a window can be in comes back as itself**, through
    /// the destination it's kept as.
    @Test("A selection is kept as a destination and opens the same selection")
    func everySelectionRoundTrips() {
        let fleet = Self.fleet()
        for selection: Selection in [
            .needsYou,
            .workspace(host: "", workspace: Self.billing, focus: nil),
            .workspace(host: "", workspace: Self.billing, focus: .task(Self.task)),
            .workspace(host: "", workspace: Self.billing, focus: .history(.done)),
            .workspace(host: "", workspace: Self.billing, focus: .worktree("lane", terminal: nil)),
            .looseWorktree(host: "", worktree: "stray", terminal: nil),
        ] {
            let kept = MacDestination.destination(selection)
            #expect(kept.flatMap { Destination(encoded: $0.encoded) } == kept, "\(selection) doesn't survive its encoding")
            #expect(kept.flatMap { MacDestination.selection(for: $0, in: fleet) } == selection, "\(selection)")
        }
        #expect(MacDestination.destination(nil) == nil)
    }

    @Test("A kept place is a selection again with no fleet to ask, panes and all")
    func placeNeedsNoFleet() {
        for selection: Selection in [
            .needsYou,
            .workspace(host: "", workspace: Self.billing, focus: nil),
            .workspace(host: "studio", workspace: Self.billing, focus: .task(Self.task)),
            .workspace(host: "", workspace: Self.billing, focus: .history(.cancelled)),
            .workspace(host: "", workspace: Self.billing, focus: .worktree("lane", terminal: nil)),
            .workspace(host: "", workspace: Self.billing, focus: .worktree("lane", terminal: "agent")),
            .looseWorktree(host: "", worktree: "stray", terminal: nil),
            .looseWorktree(host: "studio", worktree: "stray", terminal: "stray-shell"),
        ] {
            let kept = MacDestination.destination(selection)
            #expect(kept.flatMap { MacDestination.place($0) } == selection, "\(selection)")
        }
        #expect(MacDestination.place(Destination(runner: .init(host: ""), place: .terminal("t"))) == nil)
        #expect(MacDestination.place(Destination(runner: .init(host: ""), place: .task(workspace: nil, task: .init(key: "bil-1")))) == nil)
    }

    @Test("A pane the keyboard was in comes back, and opens where going to it always has")
    func aPaneComesBack() throws {
        let fleet = Self.fleet()
        let kept = try #require(
            MacDestination.destination(
                .workspace(host: "", workspace: Self.billing, focus: .worktree("lane", terminal: nil)), pane: "agent"))
        #expect(kept.pane == "agent")
        // An agent's pane is its task, as the palette's Go To lands it.
        #expect(
            MacDestination.selection(for: kept, in: fleet)
                == .workspace(host: "", workspace: Self.billing, focus: .task(Self.task)))
        #expect(MacDestination.pane(of: kept) == PaneRef(host: "", worktree: "lane", terminal: "agent"))
        let stray = try #require(
            MacDestination.destination(.looseWorktree(host: "", worktree: "stray", terminal: "stray-shell")))
        #expect(
            MacDestination.selection(for: stray, in: fleet)
                == .looseWorktree(host: "", worktree: "stray", terminal: "stray-shell"))
    }

    @Test("A task keeps its tab and its chosen agent")
    func aTaskKeepsItsTab() throws {
        let kept = try #require(
            MacDestination.destination(
                .workspace(host: "studio", workspace: Self.billing, focus: .task(Self.task)), tab: .changes,
                agent: "agent"))
        #expect(kept.tab == .changes)
        #expect(kept.agent == "agent")
        #expect(Destination.Tab.allCases.map(\.rawValue) == TaskTab.allCases.map(\.rawValue), "the tabs' names drifted")
    }

    @Test("An orchestrator's pane opens its workspace; a pane the fleet doesn't have opens nothing")
    func orchestratorAndGone() {
        let fleet = Self.fleet()
        let orchestrator = Destination(runner: .init(host: ""), place: .orchestrator(workspace: Self.billing))
        #expect(
            MacDestination.selection(for: orchestrator, in: fleet)
                == .workspace(host: "", workspace: Self.billing, focus: nil))
        let gone = Destination(runner: .init(host: ""), place: .worktree("gone", workspace: nil))
        #expect(MacDestination.selection(for: gone, in: fleet) == nil)
        #expect(MacDestination.selection(for: Destination(place: .terminal("x")), in: fleet) == nil)
    }

    @Test("A click on a task opens it as the navigator does; a relaunch only selects it, with its tab and agent")
    func landings() {
        let fleet = Self.fleet()
        let task = Destination(
            runner: .init(host: ""), place: .task(workspace: Self.billing, task: .init(id: Self.task)),
            tab: .changes, agent: "agent")
        let click = MacDestination.landing(task, click: true, in: fleet)
        #expect(click.openTask == .init(id: Self.task, host: "", workspace: Self.billing))
        #expect(click.selection == nil)
        #expect(click.taskID == Self.task)
        #expect(click.tab == .changes)
        #expect(click.agent == "agent")
        let relaunch = MacDestination.landing(task, click: false, in: fleet)
        #expect(relaunch.openTask == nil)
        #expect(relaunch.selection == .workspace(host: "", workspace: Self.billing, focus: .task(Self.task)))
        #expect(relaunch.taskID == Self.task)

        // A pane opens where going to it always has, with the keyboard in it.
        let pane = Destination(runner: .init(host: ""), place: .worktree("lane", workspace: Self.billing), pane: "agent")
        let landed = MacDestination.landing(pane, click: true, in: fleet)
        #expect(landed.selection == .workspace(host: "", workspace: Self.billing, focus: .task(Self.task)))
        #expect(landed.pane == PaneRef(host: "", worktree: "lane", terminal: "agent"))
        #expect(landed.taskID == Self.task)

        // What the fleet doesn't have opens nothing, and keeps no tab.
        let gone = Destination(runner: .init(host: ""), place: .worktree("gone", workspace: nil), tab: .agent)
        #expect(MacDestination.landing(gone, click: true, in: fleet) == MacDestination.Landing())
    }

    // MARK: - Where the window goes back to

    @Test("Where a window goes back to is what's kept, else what an earlier build kept")
    func whatIsKept() {
        let kept = Destination(runner: .init(host: ""), place: .workspace(Self.billing))
        let legacy = SelectionMemory.encode(.workspace(host: "", workspace: Self.main, focus: .task("t")))!
        #expect(SelectionMemory.kept(destination: kept.encoded, legacy: legacy) == kept)
        #expect(SelectionMemory.kept(destination: "", legacy: legacy)?.place == .task(workspace: Self.main, task: .init(id: "t")))
        #expect(SelectionMemory.kept(destination: "garbage", legacy: "")  == nil)
        #expect(SelectionMemory.kept(destination: "", legacy: "") == nil)
    }

    @Test("With nowhere to go back to, a window opens Needs You while anything waits, else the first workspace")
    func launchWithNowhereKept() {
        let fleet = Self.fleet()
        #expect(SelectionMemory.launch(needsYou: 2, settled: false, in: fleet) == .some(.needsYou))
        #expect(SelectionMemory.launch(needsYou: 0, settled: false, in: fleet) == nil)
        #expect(
            SelectionMemory.launch(needsYou: 0, settled: true, in: fleet)
                == .some(.workspace(host: "", workspace: Self.main, focus: nil)))
    }

    // MARK: - A banner

    @Test("A failed command's and an agent's banner both carry their pane")
    func bannersCarryTheirPane() {
        let terminal = Self.terminal("agent")
        let failed = Notifier.failedExit(terminal: terminal, place: "lane", host: "studio", runnerId: "R-1")
        let own = Notifier.ownBanner(
            terminal: terminal, words: (title: "t", body: "b"), activity: .done, host: "studio", runnerId: "R-1")
        for request in [failed, own] {
            let destination = Destination(userInfo: request.content.userInfo, thread: request.content.threadIdentifier)
            #expect(destination?.place == .terminal("agent"))
            #expect(destination?.runner == .init(host: "studio", id: "r-1"))
        }
    }
}
