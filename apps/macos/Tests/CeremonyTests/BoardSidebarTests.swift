import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The board as a row in the sidebar, and the way from a card to its agent.
///
/// The rule for which pane works a task is AgentKit's and is tested there.
/// What is this app's, and is here, is the half no view can be asked about:
/// that `taskId` survives the decode, what "runs an agent" means for this
/// model (a changes pane's process is `farcooler`, which `hasDetectedAgent`
/// alone would take for one), that a pill leads to the workspace the pane is
/// in, and that a board re-reads only when a runner says THAT board moved —
/// every read is a CLI process, and every repository's row now holds one open.
@MainActor
struct BoardSidebarTests {
    private static let task = "0198f2c0-0000-7000-8000-00000000b001"

    private static func terminal(
        _ id: String, preset: String = "claude", state: String = "running",
        taskId: String? = task, paneMode: String? = nil
    ) -> Terminal {
        var t = Terminal(id: id, short: id, title: "", preset: preset, state: state, epoch: 0)
        t.taskId = taskId
        t.paneMode = paneMode
        return t
    }

    private static func workspace(_ id: String, _ terminals: [Terminal]) -> Workspace {
        Workspace(
            id: id, short: id, task: id, branch: "feat/\(id)", repository: "overnight",
            host: "", worktree: "/tmp/\(id)", state: "active", terminals: terminals)
    }

    private static func row(status: TaskStatus = .inProgress) -> TaskRow {
        TaskRow(id: task, key: "-19", title: "A task", status: status, statusSince: .now)
    }

    /// The CLI sends `taskId` on both projections, and the model keeps it.
    /// A key the synthesized decoder did not know would be dropped without a
    /// word, and every card would say "No Agent" with an agent on it.
    @Test func aTerminalKeepsTheTaskItWasOpenedFor() throws {
        let json = Data(
            #"{"id":"t1","short":"t1","title":"","preset":"claude","state":"running","epoch":0,"taskId":"\#(Self.task)"}"#
                .utf8)
        let decoded = try JSONDecoder().decode(Terminal.self, from: json)
        #expect(decoded.taskId == Self.task)
        #expect(decoded.boardTaskID == Self.task)

        // And an older CLI that sends no key at all still decodes.
        let older = Data(
            #"{"id":"t1","short":"t1","title":"","preset":"claude","state":"running","epoch":0}"#.utf8)
        #expect(try JSONDecoder().decode(Terminal.self, from: older).taskId == nil)
    }

    /// An agent, and only an agent: a shell the dispatched agent exited back
    /// to is not one, and neither is a changes pane, whose process is
    /// `farcooler` and reads as an agent to `hasDetectedAgent`.
    @Test func onlyAnAgentPaneRunsAnAgent() {
        #expect(Self.terminal("a", preset: "claude").runsAgent)
        #expect(Self.terminal("b", preset: "codex").runsAgent)
        #expect(!Self.terminal("c", preset: "zsh").runsAgent)
        #expect(!Self.terminal("d", preset: "shell").runsAgent)
        #expect(!Self.terminal("e", preset: "farcooler", paneMode: "changes").runsAgent)
    }

    /// The pill leads to the pane AND the workspace it is in, so the window
    /// can select it by host and id without searching the fleet again.
    @Test func aCardsAgentsAreItsLivePanesWithTheirWorkspaces() {
        let agent = Self.terminal("agent")
        let second = Self.terminal("second", state: "starting")
        let fleet = [
            Self.workspace("lane-a", [agent, Self.terminal("shell", preset: "zsh")]),
            Self.workspace(
                "lane-b",
                [
                    Self.terminal("diff", preset: "farcooler", paneMode: "changes"),
                    Self.terminal("gone", state: "exited"),
                    Self.terminal("other", taskId: "0198f2c0-0000-7000-8000-00000000b002"),
                    second,
                ]),
        ]
        let agents = BoardAgents(workspaces: fleet, runnerRecordsTasks: true)
        let live = agents.live(for: Self.row())
        #expect(live.map(\.terminal.id) == ["agent", "second"])
        #expect(live.map(\.workspace.id) == ["lane-a", "lane-b"])
        #expect(live.first?.title == "claude in lane-a")
        #expect(agents.presence(for: Self.row()) == .agents(2))
        #expect(agents.tasksWithAgents(on: TaskBoardModel(columns: [
            TaskBoardColumn(status: .inProgress, rows: [Self.row()])
        ])) == 1)
    }

    /// A runner without `terminal_task` gets no link, whatever its panes
    /// happen to say, and no "No Agent" either.
    @Test func aRunnerWithoutTerminalTaskGetsNoLink() {
        let agents = BoardAgents(
            workspaces: [Self.workspace("lane", [Self.terminal("agent")])],
            runnerRecordsTasks: false)
        #expect(agents.live(for: Self.row()).isEmpty)
        #expect(agents.presence(for: Self.row()) == .unsaid)
        #expect(agents.tasksWithAgents(on: TaskBoardModel(columns: [
            TaskBoardColumn(status: .inProgress, rows: [Self.row()])
        ])) == 0)
    }

    /// A board selection is a repository's, and nothing about a terminal
    /// closing moves it.
    @Test func aBoardSelectionIsLeftWhereItIs() {
        let board = ContentView.Selection.board(host: "", repository: "r1")
        #expect(ContentView.healed(board, in: [Self.workspace("lane", [])]) == board)
        #expect(ContentView.healed(board, in: []) == board)
    }

    // MARK: - Reading only the board that moved

    private static let repoA = "0198f2c0-0000-7000-8000-0000000000aa"
    private static let repoB = "0198f2c0-0000-7000-8000-0000000000bb"

    /// Every `task list` the stub was asked for, by repository.
    @MainActor
    final class Reads {
        var calls: [String] = []
        func count(_ repository: String) -> Int { calls.filter { $0 == repository }.count }
    }

    private func client(_ reads: Reads) -> DaemonClient {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            if args.starts(with: ["task", "list"]), let at = args.firstIndex(of: "--repo") {
                reads.calls.append(args[at + 1])
                // A beat, so a second caller arrives while this read is in
                // flight — which is when two views asking at once would each
                // have launched one.
                try? await Task.sleep(for: .milliseconds(20))
                return (Data(#"{"tasks":[]}"#.utf8), nil)
            }
            return (Data(), nil)
        }
        return client
    }

    private static func repository(_ id: String) -> Repository {
        Repository(id: id, short: String(id.suffix(8)), displayName: id, remote: "", repositoryRootId: "")
    }

    /// A `task` event about one repository re-reads that repository's board
    /// and no other. It used to be one counter per runner, so a busy agent on
    /// one repository re-read every board the window held.
    @Test func aBoardEventReReadsOnlyTheBoardItNames() async {
        let reads = Reads()
        let client = client(reads)
        let a = TaskBoardStore(client: client, repository: Self.repository(Self.repoA))
        let b = TaskBoardStore(client: client, repository: Self.repository(Self.repoB))
        await a.readIfNeverRead()
        await b.readIfNeverRead()
        #expect(reads.count(Self.repoA) == 1)
        #expect(reads.count(Self.repoB) == 1)

        client.boardMoved(TaskEvent(repository: Self.repoA, actor: "user"))
        await a.reloadIfMoved()
        await b.reloadIfMoved()
        #expect(reads.count(Self.repoA) == 2, "the board that moved was read again")
        #expect(reads.count(Self.repoB) == 1, "the board that didn’t move was read again")

        // And once acted on, the same event is not read twice.
        await a.reloadIfMoved()
        #expect(reads.count(Self.repoA) == 2)
    }

    /// The sidebar row and the board both ask for the first read when the row
    /// is selected. One read, not two.
    @Test func twoViewsAskingForTheFirstReadLaunchOne() async {
        let reads = Reads()
        let store = TaskBoardStore(client: client(reads), repository: Self.repository(Self.repoA))
        async let first: Void = store.readIfNeverRead()
        async let second: Void = store.readIfNeverRead()
        _ = await (first, second)
        #expect(reads.count(Self.repoA) == 1)
        #expect(store.hasRead)
    }
}
