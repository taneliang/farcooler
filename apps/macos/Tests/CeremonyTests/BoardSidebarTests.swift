import AgentKit
import AppKit
import Foundation
import SwiftUI
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
        /// Reads running right now, and the most there ever were at once.
        var inFlight = 0
        var mostAtOnce = 0
        var delay: Duration = .milliseconds(20)
        func count(_ repository: String) -> Int { calls.filter { $0 == repository }.count }
    }

    /// A client whose runner answers `task list` with a board of as many
    /// tasks as it has been asked so far — so which read drew the board is
    /// readable off the board — and `workspace list` with an empty fleet.
    private func client(_ reads: Reads) -> DaemonClient {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            let words = args.filter { $0 != "--json" }
            if words.starts(with: ["task", "list"]), let at = words.firstIndex(of: "--repo") {
                reads.calls.append(words[at + 1])
                let nth = reads.calls.count
                reads.inFlight += 1
                reads.mostAtOnce = max(reads.mostAtOnce, reads.inFlight)
                // A beat, so a second caller arrives while this read is in
                // flight — which is when two views asking at once would each
                // have launched one.
                try? await Task.sleep(for: reads.delay)
                reads.inFlight -= 1
                let tasks = (0..<nth).map { i in
                    #"{"id":"t\#(i)","key":"-\#(i)","title":"T","status":"todo"}"#
                }
                return (Data(#"{"tasks":[\#(tasks.joined(separator: ","))]}"#.utf8), nil)
            }
            if words.starts(with: ["workspace", "list"]) {
                return (Data(#"{"runtime_healthy":true,"live_panes":0,"workspaces":[]}"#.utf8), nil)
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

    // MARK: - One read at a time

    /// A burst of events while a read is running costs that read plus one
    /// after it — never one per event — and never two at once, so an older
    /// board can't land over a newer one. `.task(id:)` cancelling a view's
    /// task doesn't stop a `task list` already launched, which is how five
    /// writes in a second used to be five processes racing.
    @Test func aBurstOfEventsIsOneReadRunningAndOneAfter() async {
        let reads = Reads()
        reads.delay = .milliseconds(80)
        let client = client(reads)
        let store = TaskBoardStore(client: client, repository: Self.repository(Self.repoA))

        async let first: Void = store.readIfNeverRead()
        // Until the first read is actually in flight — not a guessed sleep.
        for _ in 0..<500 where reads.inFlight == 0 {
            try? await Task.sleep(for: .milliseconds(1))
        }
        #expect(reads.inFlight == 1)
        for _ in 0..<3 {
            client.boardMoved(TaskEvent(repository: Self.repoA, actor: "agent:x"))
            await store.reloadIfMoved()
        }
        await first

        #expect(reads.count(Self.repoA) == 2, "one read running and one after it")
        #expect(reads.mostAtOnce == 1, "two reads of one board ran at once")
        #expect(store.board.rows.count == 2, "the board is the last read's")
        #expect(!store.reading)
    }

    // MARK: - Can't say

    private static func build(_ capabilities: Set<String>) -> DaemonBuild {
        DaemonBuild(version: "0.1.0", matches: true, platform: "macos", capabilities: capabilities)
    }

    /// A runner that isn't connected right now has workspaces that are the
    /// last ones read before the link went. The agents in them may have
    /// exited since, so the board says nothing about agents there: no pill,
    /// no "No Agent", no count. `.reconnecting` included — a dead runner
    /// spends most of an outage there between attempts.
    @Test func aRunnerThatIsntConnectedSaysNothingAboutAgents() {
        let fleet = [Self.workspace("lane", [Self.terminal("agent")])]
        let board = TaskBoardModel(columns: [
            TaskBoardColumn(status: .inProgress, rows: [Self.row()])
        ])
        let recording = Self.build(["tasks", "terminal_task"])
        for state: HostState in [
            .unreachable(reason: "gone"), .notInstalled, .connecting, .reconnecting(attempt: 1),
        ] {
            let agents = BoardAgents.on(fleet, state: state, build: recording)
            #expect(agents.live(for: Self.row()).isEmpty, "\(state)")
            #expect(agents.presence(for: Self.row()) == .unsaid, "\(state)")
            #expect(agents.tasksWithAgents(on: board) == 0, "\(state)")
            // And no "No Agent" for a card with nobody on it, either.
            #expect(
                BoardAgents.on([], state: state, build: recording).presence(for: Self.row())
                    == .unsaid, "\(state)")
        }
        // Connected: it says.
        let connected = BoardAgents.on(fleet, state: .connected, build: recording)
        #expect(connected.presence(for: Self.row()) == .agents(1))
        #expect(connected.tasksWithAgents(on: board) == 1)
        // And a runner that doesn't record tasks never does.
        #expect(
            BoardAgents.on(fleet, state: .connected, build: Self.build(["tasks"]))
                .presence(for: Self.row()) == .unsaid)
        #expect(BoardAgents.on(fleet, state: .connected, build: nil).presence(for: Self.row()) == .unsaid)
    }

    // MARK: - Reconnecting

    /// Writes made while the event stream was down send no event anybody
    /// hears, so a reconnection re-reads the board.
    @Test func aReconnectionReReadsTheBoard() async {
        let reads = Reads()
        let client = client(reads)
        let store = TaskBoardStore(client: client, repository: Self.repository(Self.repoA))
        await store.readIfNeverRead()
        #expect(reads.count(Self.repoA) == 1)

        await client.refresh()  // .connecting → .connected: the link comes up
        #expect(client.state == .connected)
        await store.reloadIfMoved()
        #expect(reads.count(Self.repoA) == 2, "the link came up and the board wasn’t re-read")

        // A refresh on a link that was already up is not a reconnection.
        await client.refresh()
        await store.reloadIfMoved()
        #expect(reads.count(Self.repoA) == 2)
        client.stopEvents()
    }

    // MARK: - A replacement store is read

    @MainActor
    final class Holder: ObservableObject {
        @Published var client: DaemonClient
        @Published var store: TaskBoardStore
        init(_ client: DaemonClient, _ store: TaskBoardStore) {
            self.client = client
            self.store = store
        }
    }

    private struct RowHost: View {
        @ObservedObject var holder: Holder
        var body: some View {
            BoardRow(
                store: holder.store, client: holder.client, agents: .none, isSelected: false,
                onSelect: {})
        }
    }

    private struct BoardHost: View {
        @ObservedObject var holder: Holder
        var body: some View {
            TaskBoardView(store: holder.store, client: holder.client, agents: .none, onGoTo: { _ in })
        }
    }

    /// A runner removed and added back gets a new client, and the window hands
    /// the same row, and the same board, a new store in the same place. The
    /// first read is keyed on the store, so that store is read — rather than
    /// the row losing its counts and the board sitting on empty columns until
    /// the next event.
    @Test(arguments: ["row", "board"])
    func aReplacementStoreIsRead(_ which: String) async {
        let before = Reads()
        let after = Reads()
        let oldClient = client(before)
        let holder = Holder(
            oldClient, TaskBoardStore(client: oldClient, repository: Self.repository(Self.repoA)))
        let view: AnyView = which == "row"
            ? AnyView(RowHost(holder: holder)) : AnyView(BoardHost(holder: holder))
        let host = NSHostingView(rootView: view.frame(width: 600, height: 300))
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 600, height: 300),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        for _ in 0..<100 where before.count(Self.repoA) == 0 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(10))
        }
        #expect(before.count(Self.repoA) == 1, "the first store was read")

        let newClient = client(after)
        holder.client = newClient
        holder.store = TaskBoardStore(client: newClient, repository: Self.repository(Self.repoA))
        // Until the read has LANDED, not just started: the stub counts a
        // read when it is asked, and answers a beat later.
        for _ in 0..<100 where !holder.store.hasRead {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(10))
        }
        #expect(after.count(Self.repoA) == 1, "the replacement store was never read")
        #expect(holder.store.hasRead)
        window.close()
    }

    // MARK: - Going to an agent

    /// Chosen from a menu that was open while the fleet moved: the pane when
    /// it's still there, its workspace when only the pane has gone, and
    /// nothing — stay on the board — when both have.
    @Test func goingToAnAgentLandsWhereTheFleetIsNow() {
        let agent = Self.terminal("agent")
        let pane = BoardPane(terminal: agent, workspace: Self.workspace("lane", [agent]))
        #expect(
            BoardPane.landing(for: pane, in: [Self.workspace("lane", [agent])])
                == .terminal(host: "", workspace: "lane", terminal: "agent"))
        #expect(
            BoardPane.landing(for: pane, in: [Self.workspace("lane", [])])
                == .workspace(host: "", id: "lane"))
        #expect(BoardPane.landing(for: pane, in: [Self.workspace("other", [])]) == nil)
        // Never another runner's workspace that happens to share the id.
        var elsewhere = Self.workspace("lane", [agent])
        elsewhere.host = "remote"
        #expect(BoardPane.landing(for: pane, in: [elsewhere]) == nil)
    }

    /// Two agents in one workspace are two menu items you can tell apart.
    @Test func twoAgentsInOneWorkspaceHaveDifferentMenuItems() {
        let a = Self.terminal("a1")
        let b = Self.terminal("b2")
        let lane = Self.workspace("lane", [a, b])
        let panes = [BoardPane(terminal: a, workspace: lane), BoardPane(terminal: b, workspace: lane)]
        let titles = BoardPane.titles(panes)
        #expect(titles == ["claude 1 in lane", "claude 2 in lane"])

        // Titled identically by the agent: the short ids tell them apart.
        var c = Self.terminal("c3")
        var d = Self.terminal("d4")
        c.title = "Fix it"
        d.title = "Fix it"
        let same = Self.workspace("same", [c, d])
        let named = BoardPane.titles([
            BoardPane(terminal: c, workspace: same), BoardPane(terminal: d, workspace: same),
        ])
        #expect(named == ["Fix it in same (c3)", "Fix it in same (d4)"])
        #expect(Set(named).count == 2)
    }
}
