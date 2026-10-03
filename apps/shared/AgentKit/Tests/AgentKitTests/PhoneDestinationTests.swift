import Foundation
import Testing

@testable import AgentKit

/// How a notification, and a relaunch, reach a screen on the phone (ov-182,
/// ov-183): each kind's payload, through `Destination`, the resolver and
/// `PhoneDestination.link`, to the stack that opens. In AgentKit because the
/// iOS target has no unit tests, and every one of these looks fine in a
/// screenshot when it's wrong.
struct PhoneDestinationTests {
    static let runner = PhoneNavigationTests.runner
    static let billing = PhoneNavigationTests.place
    static let main = PhoneWorkspace(runner: runner, workspace: PhoneNavigationTests.main)

    static func row(_ id: String, _ key: String) -> TaskRow {
        TaskRow(id: id, key: key, title: key, status: .inProgress, statusSince: Date())
    }

    /// One runner that has answered: its fleet, its boards and its daemon read.
    static func ready(host: String = runner, runnerId: String? = "runner-a") -> PhoneDestination.Source {
        let workspaces = PhoneNavigationTests.fleet.workspaces ?? []
        return PhoneDestination.Source(
            host: host, runnerId: runnerId, ready: true, idle: false, fleet: PhoneNavigationTests.fleet,
            boardList: workspaces,
            boards: [
                PhoneNavigationTests.billing: TaskBoardModel(columns: [
                    TaskBoardColumn(status: .inProgress, rows: [row("t9", "bil-9")])
                ]),
                PhoneNavigationTests.main: TaskBoardModel(columns: []),
            ])
    }

    static func dialing(host: String = runner) -> PhoneDestination.Source {
        PhoneDestination.Source(host: host, runnerId: nil, ready: false, idle: false, fleet: nil)
    }

    /// Where `userInfo` opens on a runner that has answered, or nil when nothing does.
    static func opened(
        _ userInfo: [AnyHashable: Any], thread: String = "", sources: [PhoneDestination.Source] = [ready()]
    ) -> PhoneLink? {
        guard let destination = Destination(userInfo: userInfo, thread: thread) else { return nil }
        let world = PhoneDestination.world(sources, last: nil)
        let resolution = DestinationResolver.resolve(
            destination, arrival: .notification, in: world, elapsed: 1,
            deadline: DestinationResolver.Deadline.notificationPhone)
        guard case .open(let open, _) = resolution else { return nil }
        let fleet = sources.first(where: { $0.host == open.runner.host })?.fleet
        return PhoneDestination.link(for: open, fleet: fleet)
    }

    // MARK: - Each kind of notification opens its subject

    @Test("A task notice opens its task over its board")
    func aTaskNotice() {
        let link = Self.opened([
            "kind": "task", "task": "bil-9", "runner": "RUNNER-A", "event": "review", "noticeId": "t:runner-a:bil-9",
        ])
        #expect(link?.stack == [.workspace(Self.billing), .task(Self.billing, task: "t9")])
    }

    @Test("A decision push, with an empty terminal beside it, opens its task")
    func aDecisionPush() {
        // The relay sends terminal "" beside a task.
        let link = Self.opened(["kind": "decision", "task": "bil-9", "terminal": "", "runner": "runner-a"])
        #expect(link?.stack == [.workspace(Self.billing), .task(Self.billing, task: "t9")])
    }

    @Test("An agent's push opens its pane, over its task and workspace")
    func anAgentPush() {
        let link = Self.opened(["terminal": "agent", "status": "blocked", "runner": "runner-a"])
        #expect(
            link?.stack == [
                .workspace(Self.billing), .task(Self.billing, task: "t9"),
                .worktree(runner: Self.runner, worktree: "webhooks", landing: .terminal("agent")),
            ])
    }

    @Test("A banner this app posted, filed under its terminal, opens that pane")
    func aLocalBanner() {
        let link = Self.opened(["target": Self.runner], thread: "shell")
        // The worktree has one open task, so a shell in it is that task's (`TaskLink`).
        #expect(
            link?.stack == [
                .workspace(Self.billing), .task(Self.billing, task: "t9"),
                .worktree(runner: Self.runner, worktree: "webhooks", landing: .terminal("shell")),
            ])
    }

    @Test("An orchestrator's pane opens its workspace on the Orchestrator segment")
    func anOrchestrator() {
        let link = Self.opened(["terminal": "orch"])
        #expect(link?.stack == [.workspace(Self.main)])
        #expect(link?.segment == .orchestrator)
    }

    @Test("A loose worktree's pane opens the worktree alone")
    func aLoosePane() {
        let link = Self.opened(["terminal": "loose"])
        #expect(link?.stack == [.worktree(runner: Self.runner, worktree: "scratch", landing: .terminal("loose"))])
    }

    @Test("A Live Activity's link opens its pane")
    func aLiveActivityLink() throws {
        let url = try #require(URL(string: "farcooler://terminal/agent?runner=runner-a"))
        let destination = try #require(Destination(url: url))
        let world = PhoneDestination.world([Self.ready()], last: nil)
        guard case .open(let open, _) = DestinationResolver.resolve(
            destination, arrival: .notification, in: world, elapsed: 1, deadline: 60)
        else { Issue.record("not opened"); return }
        #expect(PhoneDestination.link(for: open, fleet: PhoneNavigationTests.fleet).stack.last
            == .worktree(runner: Self.runner, worktree: "webhooks", landing: .terminal("agent")))
    }

    @Test("A Needs You destination is the front door")
    func needsYou() {
        #expect(Self.opened(["destination": Destination.needsYou.encoded])?.stack == [])
    }

    @Test("A pane that's gone, a task that's gone and another runner's task open nothing")
    func nothingToOpen() {
        #expect(Self.opened(["terminal": "gone"]) == nil)
        #expect(Self.opened(["kind": "task", "task": "bil-404", "runner": "runner-a"]) == nil)
        #expect(Self.opened(["kind": "task", "task": "bil-9", "runner": "runner-b"]) == nil)
    }

    // MARK: - Cold launch

    @Test("A tap before its runner is up is held, then opened")
    func aColdLaunchTap() throws {
        let destination = try #require(Destination(userInfo: ["terminal": "agent", "runner": "runner-a"], thread: ""))
        let cold = PhoneDestination.world([Self.dialing()], last: nil)
        #expect(
            DestinationResolver.resolve(destination, arrival: .notification, in: cold, elapsed: 5, deadline: 60) == .wait)
        let warm = PhoneDestination.world([Self.ready()], last: nil)
        guard case .open = DestinationResolver.resolve(destination, arrival: .notification, in: warm, elapsed: 20, deadline: 60)
        else { Issue.record("not opened"); return }
        // Past the deadline it never opens, even when what it names has turned up.
        #expect(
            DestinationResolver.resolve(destination, arrival: .notification, in: warm, elapsed: 61, deadline: 60)
                == .stay(.notFound))
    }

    @Test("A runner that's paired and not connected is told to connect")
    func aRunnerNothingDials() throws {
        let idle = PhoneDestination.Source(host: "RUNNER-B", runnerId: nil, ready: false, idle: true, fleet: nil)
        let destination = Destination(runner: .init(host: "RUNNER-B"), place: .terminal("x"))
        let world = PhoneDestination.world([Self.ready(), idle], last: nil)
        #expect(
            DestinationResolver.resolve(destination, arrival: .notification, in: world, elapsed: 1, deadline: 60)
                == .connect(host: "RUNNER-B"))
    }

    // MARK: - Relaunch

    static func restored(_ saved: [PhoneRoute], sources: [PhoneDestination.Source] = [ready()], elapsed: TimeInterval = 1) -> PhoneLink? {
        guard let kept = Destination(phoneStack: saved) else { return nil }
        let world = PhoneDestination.world(sources, last: nil)
        guard case .open(let open, _) = DestinationResolver.resolve(
            kept, arrival: .restore, in: world, elapsed: elapsed, deadline: DestinationResolver.Deadline.restore)
        else { return nil }
        return PhoneDestination.link(for: open, fleet: sources.first?.fleet)
    }

    @Test("A saved task reopens over its workspace, whatever Needs You holds")
    func aSavedTask() {
        let saved: [PhoneRoute] = [.workspace(Self.billing), .task(Self.billing, task: "t9")]
        #expect(Self.restored(saved)?.stack == saved)
    }

    @Test("A saved worktree reopens on its pane, over its workspace")
    func aSavedWorktree() {
        let saved: [PhoneRoute] = [
            .workspace(Self.billing), .worktree(runner: Self.runner, worktree: "webhooks", landing: .terminal("shell")),
        ]
        #expect(Self.restored(saved)?.stack.last == saved.last)
    }

    @Test("A saved task that's gone falls back to its workspace, not to Needs You")
    func aGoneTask() {
        let saved: [PhoneRoute] = [.workspace(Self.billing), .task(Self.billing, task: "t-gone")]
        #expect(Self.restored(saved)?.stack == [.workspace(Self.billing)])
    }

    @Test("A saved worktree that's gone falls back to its workspace")
    func aGoneWorktree() {
        let saved: [PhoneRoute] = [
            .workspace(Self.billing), .worktree(runner: Self.runner, worktree: "gone", landing: .resume),
        ]
        #expect(Self.restored(saved)?.stack == [.workspace(Self.billing)])
    }

    @Test("A saved workspace that's gone falls back to the runner's first")
    func aGoneWorkspace() {
        let saved: [PhoneRoute] = [.workspace(PhoneWorkspace(runner: Self.runner, workspace: "ws-gone"))]
        #expect(Self.restored(saved)?.stack == [.workspace(Self.main)])
    }

    @Test("A relaunch is held for its runner, then falls back to Needs You")
    func aSlowRunner() throws {
        let kept = try #require(Destination(phoneStack: [.workspace(Self.billing)]))
        let world = PhoneDestination.world([Self.dialing()], last: nil)
        #expect(DestinationResolver.resolve(kept, arrival: .restore, in: world, elapsed: 3, deadline: 10) == .wait)
        #expect(
            DestinationResolver.resolve(kept, arrival: .restore, in: world, elapsed: 10, deadline: 10)
                == .open(Destination.needsYou, fellBack: true))
    }

    @Test("A relaunch yields to somebody who moved first")
    func movedFirst() throws {
        let kept = try #require(Destination(phoneStack: [.workspace(Self.billing)]))
        let world = PhoneDestination.world([Self.ready()], last: nil)
        #expect(
            DestinationResolver.resolve(kept, arrival: .restore, in: world, elapsed: 1, deadline: 10, interrupted: true)
                == .stay(nil))
    }

    // MARK: - The world

    @Test("Boards and fleets not read yet are unknown, not empty")
    func unreadIsUnknown() {
        let world = PhoneDestination.world([Self.dialing()], last: nil)
        #expect(world.seats[0].workspaces == nil)
        #expect(world.seats[0].worktrees == nil)
        var noBoards = Self.ready()
        noBoards.boardList = []
        #expect(PhoneDestination.world([noBoards], last: nil).seats[0].workspaces == nil)
    }

    @Test("A worktree is its owner's, else its orchestrator's workspace's")
    func worktreeOwner() {
        let world = PhoneDestination.world([Self.ready()], last: nil)
        let worktrees = world.seats[0].worktrees ?? []
        #expect(worktrees.first(where: { $0.id == "webhooks" })?.workspace == PhoneNavigationTests.billing)
        #expect(worktrees.first(where: { $0.id == "scratch" })?.workspace == nil)
    }
}
