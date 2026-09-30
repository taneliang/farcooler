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
/// alone would take for one), that a pill leads to the worktree the pane is
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

    private static func worktree(_ id: String, _ terminals: [Terminal]) -> Worktree {
        Worktree(
            id: id, short: id, task: id, branch: "feat/\(id)", repository: "overnight",
            host: "", path: "/tmp/\(id)", state: "active", terminals: terminals)
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

    /// The pill leads to the pane AND the worktree it is in, so the window
    /// can select it by host and id without searching the fleet again.
    @Test func aCardsAgentsAreItsLivePanesWithTheirWorktrees() {
        let agent = Self.terminal("agent")
        let second = Self.terminal("second", state: "starting")
        let fleet = [
            Self.worktree("lane-a", [agent, Self.terminal("shell", preset: "zsh")]),
            Self.worktree(
                "lane-b",
                [
                    Self.terminal("diff", preset: "farcooler", paneMode: "changes"),
                    Self.terminal("gone", state: "exited"),
                    Self.terminal("other", taskId: "0198f2c0-0000-7000-8000-00000000b002"),
                    second,
                ]),
        ]
        let agents = BoardAgents(worktrees: fleet, runnerRecordsTasks: true)
        let live = agents.live(for: Self.row())
        #expect(live.map(\.terminal.id) == ["agent", "second"])
        #expect(live.map(\.worktree.id) == ["lane-a", "lane-b"])
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
            worktrees: [Self.worktree("lane", [Self.terminal("agent")])],
            runnerRecordsTasks: false)
        #expect(agents.live(for: Self.row()).isEmpty)
        #expect(agents.presence(for: Self.row()) == .unsaid)
        #expect(agents.tasksWithAgents(on: TaskBoardModel(columns: [
            TaskBoardColumn(status: .inProgress, rows: [Self.row()])
        ])) == 0)
    }

    /// A board selection is a workspace's, and nothing about a terminal
    /// closing moves it.
    @Test func aBoardSelectionIsLeftWhereItIs() {
        let board = ContentView.Selection.workspace(host: "", workspace: "w1", focus: nil)
        #expect(ContentView.healed(board, in: [Self.worktree("lane", [])]) == board)
        #expect(ContentView.healed(board, in: []) == board)
    }

    // MARK: - Reading only the board that moved

    private static let repoA = "0198f2c0-0000-7000-8000-0000000000aa"
    private static let repoB = "0198f2c0-0000-7000-8000-0000000000bb"
    /// Two workspaces in ONE repository, `repoA`: the case that matters now
    /// that a repository holds several boards.
    private static let main = "0198f2c0-0000-7000-8000-0000000000cc"
    private static let billing = "0198f2c0-0000-7000-8000-0000000000dd"
    /// `repoB`'s Main.
    private static let otherMain = "0198f2c0-0000-7000-8000-0000000000ee"

    /// Every `task list` the stub was asked for, by the board it named: the
    /// `--workspace` when there is one, and the `--repo` when there is not
    /// (a runner without workspaces, whose one board is the repository's).
    @MainActor
    final class Reads {
        var calls: [String] = []
        /// Reads running right now, and the most there ever were at once.
        var inFlight = 0
        var mostAtOnce = 0
        var delay: Duration = .milliseconds(20)
        /// The repositories `repo list` answers with, or nil to answer with
        /// nothing it can decode.
        var listed: [String]?
        /// The workspaces `worktree list` names, all in `repoA`, or nil for a
        /// runner without workspaces (the key absent, as the CLI sends it).
        var workspaces: [String]?
        func count(_ repository: String) -> Int { calls.filter { $0 == repository }.count }
    }

    /// A client whose runner answers `task list` with a board of as many
    /// tasks as it has been asked so far — so which read drew the board is
    /// readable off the board — and `worktree list` with an empty fleet.
    private func client(_ reads: Reads) -> DaemonClient {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            let words = args.filter { $0 != "--json" }
            if words.starts(with: ["task", "list"]),
                let at = words.firstIndex(of: "--workspace") ?? words.firstIndex(of: "--repo")
            {
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
            if words.starts(with: ["repo", "list"]), let listed = reads.listed {
                let rows = listed.map { id in
                    #"{"id":"\#(id)","short":"\#(id.suffix(8))","displayName":"\#(id)","remote":"","repositoryRootId":""}"#
                }
                return (Data(#"{"repositories":[\#(rows.joined(separator: ","))]}"#.utf8), nil)
            }
            if words.starts(with: ["worktree", "list"]) {
                let listed = reads.workspaces.map { ids in
                    let rows = ids.map { #"{"id":"\#($0)","repository":"\#(Self.repoA)","name":"W"}"# }
                    return #","workspaces":[\#(rows.joined(separator: ","))]"#
                } ?? ""
                return (Data(#"{"runtime_healthy":true,"live_panes":0,"worktrees":[]\#(listed)}"#.utf8), nil)
            }
            return (Data(), nil)
        }
        return client
    }

    /// A workspace in `repoA`, as the runner lists it.
    private static func summary(
        _ id: String, _ name: String, isMain: Bool = false, ordinal: Int = 1,
        repository: String = repoA, orchestrator: String? = nil
    ) -> WorkspaceSummary {
        WorkspaceSummary(
            id: id, name: name, taskPrefix: String(name.prefix(3)).lowercased(), isMain: isMain,
            ordinal: isMain ? 0 : ordinal, repository: repository, orchestrator: orchestrator)
    }

    /// The one board a runner without workspaces has in a repository.
    private static func implicit(_ repository: String) -> WorkspaceSummary {
        WorkspaceSummary.implicit(repository: repository)
    }

    /// Two boards in ONE repository: an event naming Billing re-reads Billing
    /// only. A counter per repository would re-read Main for every write to
    /// Billing, and each read is a CLI process.
    @Test func aBoardEventReReadsOnlyTheWorkspaceItNames() async {
        let reads = Reads()
        let client = client(reads)
        let m = TaskBoardStore(client: client, workspace: Self.summary(Self.main, "Main", isMain: true))
        let b = TaskBoardStore(client: client, workspace: Self.summary(Self.billing, "Billing"))
        await m.readIfNeverRead()
        await b.readIfNeverRead()
        #expect(reads.count(Self.main) == 1)
        #expect(reads.count(Self.billing) == 1)

        client.boardMoved(TaskEvent(repository: Self.repoA, workspace: Self.billing, actor: "user"))
        await m.reloadIfMoved()
        await b.reloadIfMoved()
        #expect(reads.count(Self.billing) == 2, "the board that moved was read again")
        #expect(reads.count(Self.main) == 1, "Main’s board was read again for Billing’s change")

        // And once acted on, the same event is not read twice.
        await b.reloadIfMoved()
        #expect(reads.count(Self.billing) == 2)
    }

    /// A move names the board the task left as well as the one it is on, and
    /// both re-read: the card has to disappear from one and appear on the
    /// other. A third board in the repository is not read.
    @Test func aMoveReReadsTheBoardItLeftAndTheBoardItJoined() async {
        let reads = Reads()
        let client = client(reads)
        let ops = "0198f2c0-0000-7000-8000-0000000000ee"
        let m = TaskBoardStore(client: client, workspace: Self.summary(Self.main, "Main", isMain: true))
        let b = TaskBoardStore(client: client, workspace: Self.summary(Self.billing, "Billing"))
        let o = TaskBoardStore(client: client, workspace: Self.summary(ops, "Ops", ordinal: 2))
        for store in [m, b, o] { await store.readIfNeverRead() }

        client.boardMoved(
            TaskEvent(repository: Self.repoA, workspace: Self.billing, fromWorkspace: Self.main, actor: "user"))
        for store in [m, b, o] { await store.reloadIfMoved() }
        #expect(reads.count(Self.billing) == 2, "the board it joined wasn’t read again")
        #expect(reads.count(Self.main) == 2, "the board it left wasn’t read again")
        #expect(reads.count(ops) == 1, "a board the move didn’t touch was read again")
    }

    /// A runner without workspaces names no workspace, and its one board per
    /// repository is the whole repository's: the notice reaches that board,
    /// and not another repository's.
    @Test func aNoticeNamingNoWorkspaceReachesItsRepositorysBoardsOnly() async {
        let reads = Reads()
        let client = client(reads)
        let a = TaskBoardStore(client: client, workspace: Self.implicit(Self.repoA))
        let b = TaskBoardStore(client: client, workspace: Self.implicit(Self.repoB))
        await a.readIfNeverRead()
        await b.readIfNeverRead()
        #expect(reads.count(Self.repoA) == 1, "an implicit board reads the whole repository")

        client.boardMoved(TaskEvent(repository: Self.repoA, actor: "user"))
        await a.reloadIfMoved()
        await b.reloadIfMoved()
        #expect(reads.count(Self.repoA) == 2)
        #expect(reads.count(Self.repoB) == 1)
    }

    /// The re-read rule is AgentKit's `BoardNotice.touches`, which the phones
    /// follow too. This app counts the news it hears and asks `touches` which
    /// of it a board cares about, and the two are checked against each other
    /// here: every board a notice touches moves, and no other.
    @Test func whichBoardsMoveIsTheSharedNoticeRule() {
        let boards = [
            Self.summary(Self.main, "Main", isMain: true), Self.summary(Self.billing, "Billing"),
            Self.summary("0198f2c0-0000-7000-8000-0000000000ee", "Ops", repository: Self.repoB),
            Self.implicit(Self.repoA), Self.implicit(Self.repoB),
        ]
        let events = [
            TaskEvent(repository: Self.repoA, workspace: Self.billing, actor: "user"),
            TaskEvent(repository: Self.repoA, workspace: Self.billing, fromWorkspace: Self.main, actor: nil),
            TaskEvent(repository: Self.repoA, actor: "manager"),
            TaskEvent(repository: Self.repoB, actor: nil),
        ]
        for event in events {
            let client = DaemonClient(target: "", notifications: NotificationCenter())
            let before = boards.map { client.boardGeneration(for: $0) }
            client.boardMoved(event)
            for (board, was) in zip(boards, before) {
                #expect(
                    (client.boardGeneration(for: board) != was) == event.notice.touches(board),
                    "\(board.name) in \(board.repository ?? "-") for \(event)")
            }
        }
    }

    /// The line the CLI's `events` actually prints for a move — `kind`, not
    /// the FFI's `event`, which is the key AgentKit's `BoardNotice(notice:)`
    /// reads — decoded by the stream and handed to the client, re-reads both
    /// boards. Copied from `task_event_json` in `crates/cli/src/main.rs`.
    @Test func aRealCLIEventLineReReadsTheBoard() async throws {
        let line = Data(
            #"{"kind":"task","task":"0198f2c0-0000-7000-8000-00000000b001","short":"0000b001","repository":"\#(Self.repoA)","actor":"user","workspace":"\#(Self.billing)","from_workspace":"\#(Self.main)"}"#
                .utf8)
        var heard: [TaskEvent] = []
        EventStream.dispatch(line, decoder: JSONDecoder(), onTask: { heard.append($0) })
        let event = try #require(heard.first, "the stream dropped the CLI’s board line")
        #expect(event.notice == BoardNotice(
            repository: Self.repoA, workspace: Self.billing, fromWorkspace: Self.main, actor: "user"))

        let reads = Reads()
        let client = client(reads)
        let m = TaskBoardStore(client: client, workspace: Self.summary(Self.main, "Main", isMain: true))
        let b = TaskBoardStore(client: client, workspace: Self.summary(Self.billing, "Billing"))
        await m.readIfNeverRead()
        await b.readIfNeverRead()
        client.boardMoved(event)
        await m.reloadIfMoved()
        await b.reloadIfMoved()
        #expect(reads.count(Self.billing) == 2)
        #expect(reads.count(Self.main) == 2)

        // And the line from a runner without workspaces: both keys null.
        let older = Data(
            #"{"kind":"task","task":"t","short":"t","repository":"\#(Self.repoA)","actor":"user","workspace":null,"from_workspace":null}"#
                .utf8)
        EventStream.dispatch(older, decoder: JSONDecoder(), onTask: { heard.append($0) })
        #expect(heard.count == 2)
        #expect(heard.last?.notice == BoardNotice(repository: Self.repoA, workspace: nil, actor: "user"))
    }

    /// A board reads its own workspace, in its own repository — the CLI
    /// resolves `--workspace` within `--repo` — and a runner without
    /// workspaces is asked for the whole repository with no `--workspace`
    /// at all, which it would refuse.
    @Test func aBoardReadNamesItsWorkspaceOnlyWhenItHasOne() async {
        var lines: [[String]] = []
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            lines.append(args)
            return (Data(#"{"tasks":[]}"#.utf8), nil)
        }
        await TaskBoardStore(client: client, workspace: Self.summary(Self.billing, "Billing")).reload()
        await TaskBoardStore(client: client, workspace: Self.implicit(Self.repoB)).reload()
        #expect(lines == [
            ["task", "list", "--repo", Self.repoA, "--workspace", Self.billing, "--json"],
            ["task", "list", "--repo", Self.repoB, "--json"],
        ])
    }

    /// The sidebar row and the board both ask for the first read when the row
    /// is selected. One read, not two.
    @Test func twoViewsAskingForTheFirstReadLaunchOne() async {
        let reads = Reads()
        let store = TaskBoardStore(client: client(reads), workspace: Self.summary(Self.main, "Main", isMain: true))
        async let first: Void = store.readIfNeverRead()
        async let second: Void = store.readIfNeverRead()
        _ = await (first, second)
        #expect(reads.count(Self.main) == 1)
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
        let store = TaskBoardStore(client: client, workspace: Self.summary(Self.main, "Main", isMain: true))

        async let first: Void = store.readIfNeverRead()
        // Until the first read is actually in flight — not a guessed sleep.
        for _ in 0..<500 where reads.inFlight == 0 {
            try? await Task.sleep(for: .milliseconds(1))
        }
        #expect(reads.inFlight == 1)
        for _ in 0..<3 {
            client.boardMoved(TaskEvent(repository: Self.repoA, workspace: Self.main, actor: "agent:x"))
            await store.reloadIfMoved()
        }
        await first

        #expect(reads.count(Self.main) == 2, "one read running and one after it")
        #expect(reads.mostAtOnce == 1, "two reads of one board ran at once")
        #expect(store.board.rows.count == 2, "the board is the last read's")
        #expect(!store.reading)
    }

    // MARK: - Can't say

    private static func build(_ capabilities: Set<String>) -> DaemonBuild {
        DaemonBuild(version: "0.1.0", matches: true, platform: "macos", capabilities: capabilities)
    }

    /// A runner that isn't connected right now has worktrees that are the
    /// last ones read before the link went. The agents in them may have
    /// exited since, so the board says nothing about agents there: no pill,
    /// no "No Agent", no count. `.reconnecting` included — a dead runner
    /// spends most of an outage there between attempts.
    @Test func aRunnerThatIsntConnectedSaysNothingAboutAgents() {
        let fleet = [Self.worktree("lane", [Self.terminal("agent")])]
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
        let store = TaskBoardStore(client: client, workspace: Self.summary(Self.main, "Main", isMain: true))
        await store.readIfNeverRead()
        #expect(reads.count(Self.main) == 1)

        await client.refresh()  // .connecting → .connected: the link comes up
        #expect(client.state == .connected)
        await store.reloadIfMoved()
        #expect(reads.count(Self.main) == 2, "the link came up and the board wasn’t re-read")

        // A refresh on a link that was already up is not a reconnection.
        await client.refresh()
        await store.reloadIfMoved()
        #expect(reads.count(Self.main) == 2)
        client.stopEvents()
    }

    /// **A stream that fell behind re-reads everything it feeds** (ov-49).
    /// The runner drops events a connection can't keep up with and says so
    /// with one `events_missed` line; that line used to fall into the
    /// stream's `default`, so a Mac that fell behind went on showing boards,
    /// panes and layouts nobody would ever tell it had moved. Now it is the
    /// phones' `resync`: the fleet, every board, and what a reconnection
    /// seeds — without replacing the link, which would restart every pane.
    @Test func aStreamThatFellBehindReReadsEverythingItFeeds() async {
        // The line `event_json` in crates/cli/src/main.rs prints.
        var missed = 0
        EventStream.dispatch(
            Data(#"{"kind":"events_missed"}"#.utf8), decoder: JSONDecoder(), onMissed: { missed += 1 })
        #expect(missed == 1, "the stream dropped the line that says lines were dropped")

        let reads = Reads()
        let client = client(reads)
        var asked: [[String]] = []
        let runner = client.commandRunnerForTesting
        client.commandRunnerForTesting = { args in
            asked.append(args)
            return await runner!(args)
        }
        var seeded = 0
        client.onReconnect = { seeded += 1 }
        await client.refresh()  // the link comes up
        let store = TaskBoardStore(client: client, workspace: Self.summary(Self.main, "Main", isMain: true))
        await store.readIfNeverRead()
        let link = client.linkGeneration
        asked = []
        seeded = 0

        await client.eventsMissed()
        #expect(asked.contains(["worktree", "list", "--json"]), "the fleet wasn’t re-read")
        #expect(seeded == 1, "layouts weren’t re-read")
        await store.reloadIfMoved()
        #expect(reads.count(Self.main) == 2, "the board wasn’t re-read")
        #expect(client.linkGeneration == link, "the link was replaced, so every pane would reattach")
        client.stopEvents()
    }

    /// **A burst of `events_missed` is one pass running and one after**
    /// (ov-49 review). Each pass is a fleet read and a seed; five lines
    /// used to be five of each, all at once.
    @Test func aBurstOfMissedNewsIsOnePassRunningAndOneAfter() async {
        let client = client(Reads())
        let runner = client.commandRunnerForTesting
        var fleetReads = 0
        client.commandRunnerForTesting = { args in
            if args.starts(with: ["worktree", "list"]) {
                fleetReads += 1
                // A beat, so the rest of the burst lands while this is out.
                try? await Task.sleep(for: .milliseconds(50))
            }
            return await runner!(args)
        }
        var seeded = 0
        client.onReconnect = { seeded += 1 }
        await client.refresh()  // the link comes up
        fleetReads = 0
        seeded = 0

        let burst = (0..<5).map { _ in Task { await client.eventsMissed() } }
        for pass in burst { await pass.value }
        #expect(fleetReads == 2, "\(fleetReads) fleet reads for one burst")
        #expect(seeded == 2, "\(seeded) seeds for one burst")
        client.stopEvents()
    }

    /// **Missed news on a link that wasn't up is re-read once** (ov-49
    /// review). The fleet read that brings the link up is a reconnection,
    /// which re-reads everything itself; `eventsMissed()` re-reading after
    /// it was a second seed and a second read of every board.
    @Test func missedNewsOnALinkThatWasntUpIsReReadOnce() async {
        let client = client(Reads())
        var seeded = 0
        client.onReconnect = { seeded += 1 }
        let board = Self.summary(Self.main, "Main", isMain: true)
        let before = client.boardGeneration(for: board)
        #expect(client.state != .connected)

        await client.eventsMissed()
        #expect(client.state == .connected)
        #expect(seeded == 1, "seeded \(seeded) times")
        #expect(client.boardGeneration(for: board) == before + 1)
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

    private struct BoardHost: View {
        @ObservedObject var holder: Holder
        var body: some View {
            TaskBoardView(store: holder.store, client: holder.client, agents: .none, onGoTo: { _ in })
        }
    }

    /// A runner removed and added back gets a new client, and the window hands
    /// the same board a new store in the same place. The first read is keyed
    /// on the store, so that store is read — rather than the board sitting on
    /// empty columns until the next event.
    @Test func aReplacementStoreIsRead() async {
        let before = Reads()
        let after = Reads()
        let oldClient = client(before)
        let holder = Holder(
            oldClient, TaskBoardStore(client: oldClient, workspace: Self.summary(Self.main, "Main", isMain: true)))
        let view = AnyView(BoardHost(holder: holder))
        let host = NSHostingView(rootView: view.frame(width: 600, height: 300))
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 600, height: 300),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        for _ in 0..<100 where before.count(Self.main) == 0 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(10))
        }
        #expect(before.count(Self.main) == 1, "the first store was read")

        let newClient = client(after)
        holder.client = newClient
        holder.store = TaskBoardStore(client: newClient, workspace: Self.summary(Self.main, "Main", isMain: true))
        // Until the read has LANDED, not just started: the stub counts a
        // read when it is asked, and answers a beat later.
        for _ in 0..<100 where !holder.store.hasRead {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(10))
        }
        #expect(after.count(Self.main) == 1, "the replacement store was never read")
        #expect(holder.store.hasRead)
        window.close()
    }

    // MARK: - Going to an agent

    /// Chosen from a menu that was open while the fleet moved: the pane when
    /// it's still there, its worktree when only the pane has gone, and
    /// nothing — stay on the board — when both have.
    @Test func goingToAnAgentLandsWhereTheFleetIsNow() {
        let agent = Self.terminal("agent")
        let pane = BoardPane(terminal: agent, worktree: Self.worktree("lane", [agent]))
        func fleet(_ worktrees: [Worktree]) -> Fleet {
            Fleet(runtimeHealthy: true, livePanes: 0, worktrees: worktrees, branchPrefix: nil)
        }
        #expect(
            BoardPane.landing(for: pane, in: fleet([Self.worktree("lane", [agent])]))
                == .looseWorktree(host: "", worktree: "lane", terminal: "agent"))
        #expect(
            BoardPane.landing(for: pane, in: fleet([Self.worktree("lane", [])]))
                == .looseWorktree(host: "", worktree: "lane", terminal: nil))
        #expect(BoardPane.landing(for: pane, in: fleet([Self.worktree("other", [])])) == nil)
        // Never another runner's worktree that happens to share the id.
        var elsewhere = Self.worktree("lane", [agent])
        elsewhere.host = "remote"
        #expect(BoardPane.landing(for: pane, in: fleet([elsewhere])) == nil)
    }

    /// Two agents in one worktree are two menu items you can tell apart.
    @Test func twoAgentsInOneWorktreeHaveDifferentMenuItems() {
        let a = Self.terminal("a1")
        let b = Self.terminal("b2")
        let lane = Self.worktree("lane", [a, b])
        let panes = [BoardPane(terminal: a, worktree: lane), BoardPane(terminal: b, worktree: lane)]
        let titles = BoardPane.titles(panes)
        #expect(titles == ["claude 1 in lane", "claude 2 in lane"])

        // Titled identically by the agent: the short ids tell them apart.
        var c = Self.terminal("c3")
        var d = Self.terminal("d4")
        c.title = "Fix it"
        d.title = "Fix it"
        let same = Self.worktree("same", [c, d])
        let named = BoardPane.titles([
            BoardPane(terminal: c, worktree: same), BoardPane(terminal: d, worktree: same),
        ])
        #expect(named == ["Fix it in same (c3)", "Fix it in same (d4)"])
        #expect(Set(named).count == 2)
    }

    // MARK: - Which board stores the window keeps

    /// A store whose runner has gone — removed, or back as a new client — is
    /// let go, and its open card with it; one whose runner is still the one
    /// the window holds is kept.
    @Test func aStoreIsLetGoWithItsRunner() {
        let reads = Reads()
        let old = client(reads), current = client(reads)
        let stale = TaskBoardStore(client: old, workspace: Self.implicit(Self.repoA))
        let live = TaskBoardStore(client: current, workspace: Self.implicit(Self.repoA))
        stale.opened = Self.row()
        live.opened = Self.row()

        let held = ContentView.heldBoardStores(
            ["old/a": stale, "/a": live], clients: ["": current])
        #expect(Array(held.keys) == ["/a"])
        #expect(stale.opened == nil, "a let-go store kept its card open")
        #expect(live.opened != nil, "a kept store's card was closed")
    }

    /// A project a connected runner has listed without is let go. Before the
    /// runner has listed anything — connecting, or reconnecting — it is kept:
    /// "gone" can't be told from "not answering" then.
    @Test func aStoreIsLetGoOnlyOnceItsRunnerHasListedWithoutIt() async {
        let reads = Reads()
        let client = client(reads)
        let a = TaskBoardStore(client: client, workspace: Self.implicit(Self.repoA))
        let b = TaskBoardStore(client: client, workspace: Self.implicit(Self.repoB))
        let stores = ["/a": a, "/b": b]

        // Not connected, nothing listed: both kept.
        #expect(ContentView.heldBoardStores(stores, clients: ["": client]).count == 2)

        // Connected, and a list that didn't decode: still can't say.
        await client.refresh()
        #expect(client.state == .connected)
        await client.refreshRepositories()
        #expect(!client.repositoriesListed)
        #expect(ContentView.heldBoardStores(stores, clients: ["": client]).count == 2)

        // Connected and listed with only A: B goes.
        reads.listed = [Self.repoA]
        await client.refreshRepositories()
        #expect(client.repositoriesListed)
        #expect(Array(ContentView.heldBoardStores(stores, clients: ["": client]).keys) == ["/a"])
        client.stopEvents()
    }

    /// A workspace deleted on a connected runner takes its board store with
    /// it — the fleet no longer lists it — while one still listed is kept, and
    /// a runner that lists no workspaces at all (older, or not read yet)
    /// can't say one is gone.
    @Test func aStoreIsLetGoOnceItsWorkspaceIsNoLongerListed() async {
        let reads = Reads()
        reads.listed = [Self.repoA]
        reads.workspaces = [Self.main, Self.billing]
        let client = client(reads)
        let m = TaskBoardStore(client: client, workspace: Self.summary(Self.main, "Main", isMain: true))
        let b = TaskBoardStore(client: client, workspace: Self.summary(Self.billing, "Billing"))
        let stores = ["/m": m, "/b": b]
        await client.refresh()
        await client.refreshRepositories()
        #expect(ContentView.heldBoardStores(stores, clients: ["": client]).count == 2)

        reads.workspaces = [Self.main]
        await client.refresh()
        #expect(Array(ContentView.heldBoardStores(stores, clients: ["": client]).keys) == ["/m"])
        client.stopEvents()
    }

    // MARK: - The sidebar: repository, workspace, then its rows

    private static func worktree(
        _ id: String, workspace: String?, repository: String = repoA, name: String = "overnight",
        terminals: [Terminal] = [], state: String = "active"
    ) -> Worktree {
        Worktree(
            id: id, short: id, task: id, branch: "feat/\(id)", repository: name, host: "",
            path: "/tmp/\(id)", state: state, terminals: terminals, repositoryID: repository,
            workspace: workspace)
    }

    /// One runner's fleet, with its workspaces — or nil for a runner without
    /// `workstreams`, whose envelope has no `workspaces` key.
    private static func fleet(workspaces: [WorkspaceSummary]?, worktrees: [Worktree]) -> Fleet {
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: worktrees, branchPrefix: nil)
        if let workspaces { fleet.runnerWorkspaces[""] = workspaces }
        return fleet
    }

    private static func orchestrator(_ id: String, workspace: String) -> Terminal {
        var t = terminal(id, taskId: nil)
        t.workspace = workspace
        t.role = "orchestrator"
        return t
    }

    /// The workspace level is drawn even when Main is the only workspace, so
    /// the model is visible before the first split.
    @Test func theSidebarShowsTheWorkspaceLevelEvenWithOnlyMain() {
        let rows = ContentView.sidebarRows(fleet: Self.fleet(
            workspaces: [Self.summary(Self.main, "Main", isMain: true)],
            worktrees: [Self.worktree("lane", workspace: Self.main)]))
        #expect(rows.map(\.kind) == [.repository, .workspace("Main"), .worktrees(count: 1)])
        let open = ContentView.sidebarRows(
            fleet: Self.fleet(
                workspaces: [Self.summary(Self.main, "Main", isMain: true)],
                worktrees: [Self.worktree("lane", workspace: Self.main)]),
            open: { _ in true })
        #expect(open.map(\.kind) == [.repository, .workspace("Main"), .worktrees(count: 1), .worktree("lane")])
    }

    @Test func unclaimedWorktreesSitBelowTheWorkspaces() {
        let rows = ContentView.sidebarRows(fleet: Self.fleet(
            workspaces: [Self.summary(Self.main, "Main", isMain: true)],
            worktrees: [Self.worktree("lane", workspace: Self.main), Self.worktree("stray", workspace: nil)]))
        #expect(rows.map(\.kind).last == .unclaimed(count: 1))
        #expect(rows.last?.worktrees.map(\.id) == ["stray"])
        #expect(!rows.map(\.kind).contains(.worktree("stray")), "an unclaimed worktree was drawn twice")
    }

    /// Main first, then the rest by their ordinal, one row each, and a
    /// workspace with no worktrees still listed. Its worktrees are one click
    /// down, under Worktrees, and the repository lists none of its own
    /// (spec §4.5). Hidden worktrees stay the repository's, last.
    @Test("A repository lists workspaces, not worktrees")
    func aRepositoryListsWorkspacesNotWorktrees() {
        let ops = "0198f2c0-0000-7000-8000-0000000000ee"
        let fleet = Self.fleet(
            workspaces: [
                Self.summary(ops, "Ops", ordinal: 2), Self.summary(Self.billing, "Billing", ordinal: 1),
                Self.summary(Self.main, "Main", isMain: true),
            ],
            worktrees: [
                Self.worktree("bill", workspace: Self.billing), Self.worktree("lane", workspace: Self.main),
                Self.worktree("stray", workspace: nil),
                Self.worktree("old", workspace: Self.main, state: "hidden"),
            ])
        let rows = ContentView.sidebarRows(fleet: fleet)
        #expect(rows.map(\.kind) == [
            .repository,
            .workspace("Main"), .worktrees(count: 1),
            .workspace("Billing"), .worktrees(count: 1),
            .workspace("Ops"), .worktrees(count: 0),
            .unclaimed(count: 1), .hidden(count: 1),
        ])
        #expect(!rows.contains { if case .worktree = $0.kind { true } else { false } })
        #expect(rows.filter { if case .workspace = $0.kind { true } else { false } }.map(\.workspace?.id) == [Self.main, Self.billing, ops])
        // The repository is a section header, so its workspaces sit flush
        // under it, and Unclaimed and Hidden with them; a workspace's
        // Worktrees is the one step in (ov-63).
        #expect(rows.map(\.depth) == [0, 0, 1, 0, 1, 0, 1, 0, 0])

        // Billing's disclosure opened: its worktree, under it, one step in
        // like the disclosure and no deeper.
        let open = ContentView.sidebarRows(
            fleet: fleet, open: { $0 == SidebarEntry.openKey(host: "", workspace: Self.billing) })
        #expect(open.map(\.kind).prefix(6) == [
            .repository, .workspace("Main"), .worktrees(count: 1), .workspace("Billing"), .worktrees(count: 1),
            .worktree("bill"),
        ])
        #expect(open.map(\.depth).prefix(6) == [0, 0, 1, 0, 1, 1])

        // Ops's, empty: its sentence starts where a worktree's title does,
        // a gutter past the worktree row's chevron (checklist step 20).
        let empty = ContentView.sidebarRows(
            fleet: fleet, open: { $0 == SidebarEntry.openKey(host: "", workspace: ops) })
        #expect(empty.first { $0.kind == .noWorktrees }?.depth == 2)
    }

    /// A runner without `workstreams` has one implicit workspace per
    /// repository: a row called Main, with the repository's worktrees under
    /// it, and nothing unclaimed.
    @Test func aRunnerWithoutWorkspacesListsItsImplicitWorkspace() {
        let rows = ContentView.sidebarRows(fleet: Self.fleet(
            workspaces: nil,
            worktrees: [Self.worktree("a", workspace: nil), Self.worktree("b", workspace: nil)]))
        #expect(rows.map(\.kind) == [.repository, .workspace("Main"), .worktrees(count: 2)])
        #expect(rows[1].workspace == WorkspaceSummary.implicit(repository: Self.repoA))
        #expect(rows[1].menu.isEmpty, "an implicit workspace has no orchestrator or charter")
    }

    /// A workspace row's count is the items counted under it, on its runner,
    /// not its terminals wanting attention: an ask and a decision about one
    /// task are one item. An implicit workspace counts its repository's
    /// items with no workspace.
    @Test("A workspace row counts its items")
    func aWorkspaceRowCountsItsItems() {
        func item(_ id: String, workspace: String?, repository: String = Self.repoA, runner: String = "") -> NeedsYouItem {
            NeedsYouItem(
                id: id, kind: .ask, also: [.decision], rank: 1, since: nil, workspaceID: workspace,
                repositoryID: repository, question: "q", runner: runner)
        }
        let items = [
            item("a", workspace: Self.billing), item("b", workspace: Self.billing),
            item("c", workspace: Self.main), item("d", workspace: Self.billing, runner: "remote"),
            item("e", workspace: nil),
        ]
        #expect(WorkspaceCounts.count(for: Self.summary(Self.billing, "Billing"), host: "", in: items) == 2)
        #expect(WorkspaceCounts.count(for: Self.summary(Self.main, "Main", isMain: true), host: "", in: items) == 1)
        #expect(WorkspaceCounts.count(for: .implicit(repository: Self.repoA), host: "", in: items) == 1)
        #expect(WorkspaceCounts.count(for: Self.summary(Self.billing, "Billing"), host: "remote", in: items) == 1)
    }

    /// Worktrees lists the workspace's worktrees in the runner's order, each
    /// named with its open tasks' keys, "fc-3-webhooks · bil-9", with no
    /// board read (spec §3.2).
    @Test("The Worktrees disclosure lists the workspace's worktrees with their open task keys")
    func theWorktreesDisclosureListsTheWorkspacesWorktreesWithTheirOpenTaskKeys() {
        var hooks = Self.worktree("fc-3-webhooks", workspace: Self.billing)
        hooks.openTasks = [NeedsYouTask(id: "t-9", key: "bil-9", title: "Invoice PDF export", status: "in_progress")]
        let scratch = Self.worktree("scratch", workspace: Self.billing)
        let rows = ContentView.sidebarRows(
            fleet: Self.fleet(
                workspaces: [Self.summary(Self.main, "Main", isMain: true), Self.summary(Self.billing, "Billing")],
                worktrees: [hooks, Self.worktree("lane", workspace: Self.main), scratch]),
            open: { $0 == SidebarEntry.openKey(host: "", workspace: Self.billing) })
        let disclosure = rows.first { $0.kind == .worktrees(count: 2) && $0.workspace?.id == Self.billing }
        #expect(disclosure?.worktrees.map(\.rowTitle) == ["fc-3-webhooks · bil-9", "scratch"])
        let listed = rows.compactMap { row -> String? in
            if case .worktree = row.kind, row.workspace?.id == Self.billing { return row.worktree?.rowTitle }
            return nil
        }
        #expect(listed == ["fc-3-webhooks · bil-9", "scratch"])
    }

    /// A workspace with no worktrees still has its disclosure, and opened,
    /// it says so and offers New Worktree… (spec §8). Not while searching,
    /// where an empty disclosure would look like a hit.
    @Test("A workspace with no worktrees says so when its Worktrees is opened")
    func aWorkspaceWithNoWorktreesSaysSo() {
        let fleet = Self.fleet(
            workspaces: [Self.summary(Self.main, "Main", isMain: true), Self.summary(Self.billing, "Billing")],
            worktrees: [Self.worktree("lane", workspace: Self.main)])
        let closed = ContentView.sidebarRows(fleet: fleet)
        #expect(closed.map(\.kind) == [.repository, .workspace("Main"), .worktrees(count: 1), .workspace("Billing"), .worktrees(count: 0)])
        let open = ContentView.sidebarRows(
            fleet: fleet, open: { $0 == SidebarEntry.openKey(host: "", workspace: Self.billing) })
        #expect(open.last?.kind == .noWorktrees)
        #expect(open.last?.workspace?.id == Self.billing)
        #expect(NoWorktreesRow.sentence == "No worktrees yet. The orchestrator makes them as it dispatches tasks.")
        let searching = ContentView.sidebarRows(fleet: fleet, query: "billing")
        #expect(!searching.contains { $0.kind == .worktrees(count: 0) || $0.kind == .noWorktrees })
    }

    /// A workspace row carries ov-60's menu, Replace in place of Start while
    /// one runs.
    @Test("A workspace row offers Show Board, Start Orchestrator and Show Charter")
    func aWorkspaceRowOffersShowBoardStartOrchestratorAndShowCharter() {
        let conductor = Self.orchestrator("conductor", workspace: Self.billing)
        let rows = ContentView.sidebarRows(fleet: Self.fleet(
            workspaces: [
                Self.summary(Self.main, "Main", isMain: true),
                Self.summary(Self.billing, "Billing", orchestrator: "conductor"),
            ],
            worktrees: [Self.worktree("checkout", workspace: Self.main, terminals: [conductor])]))
        #expect(rows.first { $0.kind == .workspace("Main") }?.menu == [.showBoard, .startOrchestrator, .showCharter])
        #expect(rows.first { $0.kind == .workspace("Billing") }?.menu == [.showBoard, .replaceOrchestrator, .showCharter])
    }

    /// The orchestrator runs in the main checkout, so the runner lists it
    /// among that worktree's terminals. It gets its own row under ITS
    /// workspace and is left out of the worktree's, so it is drawn once.
    @Test func theOrchestratorIsDrawnOnceInItsOwnRow() {
        let conductor = Self.orchestrator("conductor", workspace: Self.billing)
        let shell = Self.terminal("shell", preset: "zsh", taskId: nil)
        let rows = ContentView.sidebarRows(fleet: Self.fleet(
            workspaces: [
                Self.summary(Self.main, "Main", isMain: true), Self.summary(Self.billing, "Billing"),
            ],
            worktrees: [Self.worktree("checkout", workspace: Self.main, terminals: [conductor, shell])]),
            open: { _ in true })
        let orchestrators = rows.filter { if case .workspace = $0.kind { true } else { false } }
        #expect(orchestrators.map(\.orchestrator?.terminal.id) == [nil, "conductor"])
        #expect(orchestrators.last?.orchestrator?.worktree.id == "checkout", "where selecting it goes")
        let checkout = rows.first { $0.kind == .worktree("checkout") }?.worktree
        #expect(checkout?.terminals.map(\.id) == ["shell"], "the orchestrator was drawn twice")
    }

    /// The row shows the runner's live seat, `WorkspaceSummary.orchestrator`.
    /// A stopped orchestrator left beside it — stopped, then a new one started
    /// without `--replace` — is not the seat, so it stays among its
    /// worktree's terminals rather than being drawn nowhere, whichever of the
    /// two the runner lists last.
    @Test func aStoppedOrchestratorBesideTheLiveOneIsNotLost() {
        var stopped = Self.orchestrator("stopped", workspace: Self.billing)
        stopped.state = "exited"
        let live = Self.orchestrator("live", workspace: Self.billing)
        for terminals in [[live, stopped], [stopped, live]] {
            let rows = ContentView.sidebarRows(fleet: Self.fleet(
                workspaces: [
                    Self.summary(Self.main, "Main", isMain: true),
                    Self.summary(Self.billing, "Billing", orchestrator: "live"),
                ],
                worktrees: [Self.worktree("checkout", workspace: Self.main, terminals: terminals)]),
                open: { _ in true })
            let seat = rows.last { $0.kind == .workspace("Billing") }?.orchestrator?.terminal.id
            #expect(seat == "live", "\(terminals.map(\.id))")
            let checkout = rows.first { $0.kind == .worktree("checkout") }?.worktree
            #expect(checkout?.terminals.map(\.id) == ["stopped"], "\(terminals.map(\.id))")
        }

        // The seat wins over a role match even when both are running: the
        // runner's word for which one is live, not whichever it lists first.
        let other = Self.orchestrator("other", workspace: Self.billing)
        let seated = ContentView.sidebarRows(fleet: Self.fleet(
            workspaces: [
                Self.summary(Self.main, "Main", isMain: true),
                Self.summary(Self.billing, "Billing", orchestrator: "live"),
            ],
            worktrees: [Self.worktree("checkout", workspace: Self.main, terminals: [other, live])]),
            open: { _ in true })
        #expect(seated.last { $0.kind == .workspace("Billing") }?.orchestrator?.terminal.id == "live")
        #expect(
            seated.first { $0.kind == .worktree("checkout") }?.worktree?.terminals.map(\.id)
                == ["other"])

        // A runner that names no seat: only a live orchestrator is taken by
        // its role, and a stopped one alone leaves the row empty and itself
        // in its worktree.
        let unseated = ContentView.sidebarRows(fleet: Self.fleet(
            workspaces: [
                Self.summary(Self.main, "Main", isMain: true), Self.summary(Self.billing, "Billing"),
            ],
            worktrees: [Self.worktree("checkout", workspace: Self.main, terminals: [stopped])]),
            open: { _ in true })
        #expect(unseated.last { $0.kind == .workspace("Billing") }?.orchestrator == nil)
        #expect(
            unseated.first { $0.kind == .worktree("checkout") }?.worktree?.terminals.map(\.id)
                == ["stopped"])
    }

    /// ⌘] and ⌘[, ⌥⌘↓ and ⌥⌘↑ and ⌘1… step through the terminals of the
    /// view on screen, column by column, each layout's panes in tmux's
    /// order: the conversation, then the task's agent. The sidebar no longer
    /// lists terminals, so walking all of them walked a list nobody saw.
    @Test func steppingFollowsTheViewOnScreen() {
        let lane = Self.worktree("lane", workspace: Self.billing, terminals: [Self.terminal("a1"), Self.terminal("a2")])
        let checkout = Self.worktree("checkout", workspace: Self.main, terminals: [Self.orchestrator("conductor", workspace: Self.billing)])
        func pane(_ id: String) -> PaneRect {
            PaneRect(id: id, short: id, title: nil, left: 0, top: 0, columns: 80, rows: 24, focused: false, zoomed: false)
        }
        func group(_ id: String, _ panes: [String]) -> PaneGroup {
            PaneGroup(id: id, name: "", active: true, columns: 80, rows: 24, layout: id, panes: panes.map(pane))
        }
        let shown = [
            ShownLayout(column: .conversation, worktree: checkout, group: group("@1", ["conductor"]), groups: []),
            ShownLayout(column: .task, worktree: lane, group: group("@2", ["a2", "a1"]), groups: []),
        ]
        #expect(ContentView.stepOrder(shown).map(\.terminal) == ["conductor", "a2", "a1"])
        #expect(ContentView.stepOrder(shown).map(\.worktree) == ["checkout", "lane", "lane"])
    }

    /// A board's store survives its orchestrator starting or stopping — a new
    /// one would close the open card and read the board again — and is
    /// replaced for a rename, which its title shows, or a new client.
    @Test func aBoardStoreOutlivesItsOrchestratorChanging() {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let store = TaskBoardStore(client: client, workspace: Self.summary(Self.billing, "Billing"))
        #expect(ContentView.keeps(
            store, for: Self.summary(Self.billing, "Billing", orchestrator: "t1"), client: client))
        #expect(!ContentView.keeps(store, for: Self.summary(Self.billing, "Payments"), client: client))
        #expect(!ContentView.keeps(
            store, for: Self.summary(Self.billing, "Billing"),
            client: DaemonClient(target: "", notifications: NotificationCenter())))
    }

    /// Rows are joined to their repository by the uuid, not the display name:
    /// two repositories called the same are two groups, and an unclaimed
    /// worktree lands under its own.
    @Test func rowsJoinTheirRepositoryByItsID() {
        let otherMain = "0198f2c0-0000-7000-8000-0000000000ff"
        let rows = ContentView.sidebarRows(fleet: Self.fleet(
            workspaces: [
                Self.summary(Self.main, "Main", isMain: true),
                Self.summary(otherMain, "Main", isMain: true, repository: Self.repoB),
            ],
            worktrees: [
                Self.worktree("a", workspace: Self.main),
                Self.worktree("loose", workspace: nil, repository: Self.repoB),
            ]))
        #expect(rows.map(\.kind) == [
            .repository, .workspace("Main"), .worktrees(count: 1),
            .repository, .workspace("Main"), .worktrees(count: 0), .unclaimed(count: 1),
        ])
        #expect(rows.filter { $0.kind == .repository }.map(\.repositoryID) == [Self.repoA, Self.repoB])
        #expect(rows.last?.repositoryID == Self.repoB)
    }

    /// A drag reorders within one workspace, or within Unclaimed — where a
    /// worktree naming a workspace the runner doesn't list is drawn too — and
    /// not across: the order can't move a worktree to another workspace.
    @Test func aDragReordersOnlyWithinOnePlace() {
        let lane = Self.worktree("lane", workspace: Self.main)
        let next = Self.worktree("next", workspace: Self.main)
        let bill = Self.worktree("bill", workspace: Self.billing)
        let stray = Self.worktree("stray", workspace: nil)
        let orphan = Self.worktree("orphan", workspace: "0198f2c0-0000-7000-8000-0000000000ee")
        let fleet = Self.fleet(
            workspaces: [
                Self.summary(Self.main, "Main", isMain: true), Self.summary(Self.billing, "Billing"),
            ],
            worktrees: [lane, next, bill, stray, orphan])
        #expect(ContentView.sameSidebarPlace(lane, next, in: fleet))
        #expect(!ContentView.sameSidebarPlace(lane, bill, in: fleet))
        #expect(!ContentView.sameSidebarPlace(lane, stray, in: fleet))
        #expect(ContentView.sameSidebarPlace(stray, orphan, in: fleet))
    }

    // MARK: - Decoding the fields the sidebar groups by

    /// What `worktree list --json` sends: the envelope's `workspaces`, each
    /// row's `repository_id` and `workspace`, and each terminal's `workspace`
    /// and `role` — all landing on the model. The client files the list
    /// under its own runner, because the wire doesn't say which that is.
    @Test func theFleetsWorkspaceFieldsLandOnTheModel() async throws {
        let json = #"""
            {"runtime_healthy":true,"live_panes":1,"branch_prefix":"","worktrees":[
              {"id":"w1","short":"w1","task":"lane","branch":"lane","repository":"overnight",
               "repository_id":"\#(Self.repoA)","workspace":"\#(Self.billing)","claim_source":"explicit",
               "foreign_writers":[],"host":"","worktree":"/tmp/lane","state":"active","terminals":[
                 {"id":"t1","short":"t1","title":"","preset":"claude","state":"running","epoch":0,
                  "workspace":"\#(Self.billing)","role":"orchestrator"}]}],
             "workspaces":[{"id":"\#(Self.billing)","short":"000000dd","repository":"\#(Self.repoA)",
               "name":"Billing","task_prefix":"bil","is_main":false,"ordinal":1,"orchestrator":"t1",
               "home":null,"charter":null}]}
            """#
        let fleet = try JSONDecoder().decode(Fleet.self, from: Data(json.utf8))
        let row = try #require(fleet.worktrees.first)
        #expect(row.repositoryID == Self.repoA)
        #expect(row.workspace == Self.billing)
        #expect(row.terminals.first?.workspace == Self.billing)
        #expect(row.terminals.first?.isOrchestrator == true)
        #expect(fleet.workspaces?.map(\.name) == ["Billing"])

        // An older runner sends none of them, and still decodes.
        let older = #"""
            {"runtime_healthy":true,"live_panes":0,"worktrees":[
              {"id":"w1","short":"w1","task":"lane","branch":"lane","repository":"overnight",
               "worktree":"/tmp/lane","state":"active","terminals":[
                 {"id":"t1","short":"t1","title":"","preset":"claude","state":"running","epoch":0}]}]}
            """#
        let old = try JSONDecoder().decode(Fleet.self, from: Data(older.utf8))
        #expect(old.workspaces == nil)
        #expect(old.worktrees.first?.repositoryID == nil)
        #expect(old.worktrees.first?.terminals.first?.isOrchestrator == false)

        // Read through a client, the list is filed under that client's runner.
        let reads = Reads()
        reads.workspaces = [Self.main]
        let client = client(reads)
        await client.refresh()
        #expect(client.fleet.runnerWorkspaces[""]?.map(\.id) == [Self.main])
        client.stopEvents()
    }

    /// Every runner's workspaces survive the merge, each under its own runner.
    @Test func theMergedFleetKeepsEachRunnersWorkspaces() {
        var here = Self.fleet(workspaces: [Self.summary(Self.main, "Main", isMain: true)], worktrees: [])
        here.workspaces = here.runnerWorkspaces[""]
        var there = Fleet.empty
        there.runnerWorkspaces["remote"] = [Self.summary(Self.billing, "Billing")]
        let merged = FleetStore.merge([
            (host: "", state: .connected, fleet: here), (host: "remote", state: .connected, fleet: there),
            (host: "old", state: .connected, fleet: .empty),
        ]).fleet
        #expect(merged.runnerWorkspaces[""]?.map(\.id) == [Self.main])
        #expect(merged.runnerWorkspaces["remote"]?.map(\.id) == [Self.billing])
        #expect(merged.runnerWorkspaces["old"] == nil, "a runner without workspaces was given some")
    }

    // MARK: - Where ⇧⌘B and ⌘N go

    /// ⇧⌘B opens the board of the workspace the selection is in: a claimed
    /// worktree's own, Main's for one in Unclaimed, the orchestrator's own
    /// workspace for its row, and the repository's one board on a runner
    /// without workspaces.
    @Test func showBoardOpensTheSelectionsWorkspace() {
        let conductor = Self.orchestrator("conductor", workspace: Self.billing)
        let workspaces = [
            Self.summary(Self.main, "Main", isMain: true), Self.summary(Self.billing, "Billing"),
        ]
        let fleet = Self.fleet(
            workspaces: workspaces,
            worktrees: [
                Self.worktree("bill", workspace: Self.billing), Self.worktree("stray", workspace: nil),
                Self.worktree("checkout", workspace: Self.main, terminals: [conductor]),
            ])
        func target(_ selection: ContentView.Selection) -> String? {
            ContentView.boardWorkspace(for: selection, in: fleet)?.id
        }
        #expect(target(.workspace(host: "", workspace: Self.billing, focus: .worktree("bill", terminal: nil))) == Self.billing)
        #expect(target(.looseWorktree(host: "", worktree: "stray", terminal: nil)) == Self.main)
        #expect(target(.workspace(host: "", workspace: Self.billing, focus: nil)) == Self.billing)
        #expect(target(.workspace(host: "", workspace: Self.main, focus: .worktree("checkout", terminal: nil))) == Self.main)
        #expect(target(.looseWorktree(host: "", worktree: "gone", terminal: nil)) == nil)
        #expect(target(.needsYou) == nil)

        let older = Self.fleet(workspaces: nil, worktrees: [Self.worktree("a", workspace: nil)])
        #expect(
            ContentView.boardWorkspace(for: .looseWorktree(host: "", worktree: "a", terminal: nil), in: older)
                == WorkspaceSummary.implicit(repository: Self.repoA))
    }

    /// A new worktree — from ⌘N, or a repository header's "New Worktree in
    /// X…" — is claimed for the workspace you are in, or have selected, when
    /// it is being made in that workspace's repository on that runner.
    /// Anywhere else it is claimed for the repository's Main: from
    /// Unclaimed, from another repository or runner, or with nothing
    /// selected. Never left Unclaimed, where nothing would ever claim it.
    /// Only a runner without workspaces gets no claim.
    @Test func aNewWorktreeIsClaimedForTheWorkspaceYouAreIn() {
        let fleet = Self.fleet(
            workspaces: [
                Self.summary(Self.main, "Main", isMain: true), Self.summary(Self.billing, "Billing"),
                Self.summary(Self.otherMain, "Main", isMain: true, repository: Self.repoB),
            ],
            worktrees: [Self.worktree("bill", workspace: Self.billing), Self.worktree("stray", workspace: nil)])
        func claim(_ selection: ContentView.Selection?, host: String = "", repository: String = Self.repoA) -> String? {
            ContentView.claim(newWorktreeIn: repository, on: host, from: selection, in: fleet)
        }
        let bill = ContentView.Selection.workspace(host: "", workspace: Self.billing, focus: .worktree("bill", terminal: nil))
        #expect(claim(bill) == Self.billing)
        #expect(claim(.workspace(host: "", workspace: Self.main, focus: nil)) == Self.main)
        #expect(claim(.looseWorktree(host: "", worktree: "stray", terminal: nil)) == Self.main, "Unclaimed claims for Main")
        #expect(claim(bill, repository: Self.repoB) == Self.otherMain)
        #expect(claim(bill, host: "remote") == nil, "no workspaces there")
        // Not even where another runner lists a workspace by the same id: a
        // selection on this Mac says nothing about a worktree made there, so
        // it's that runner's Main.
        var twice = fleet
        twice.runnerWorkspaces["remote"] = fleet.runnerWorkspaces[""]
        #expect(
            ContentView.claim(
                newWorktreeIn: Self.repoA, on: "remote", from: bill, in: twice)
                == Self.main)
        #expect(claim(nil) == Self.main, "nothing selected claims for Main")
        let older = Self.fleet(workspaces: nil, worktrees: [Self.worktree("a", workspace: nil)])
        #expect(ContentView.claim(newWorktreeIn: Self.repoA, on: "", from: .workspace(host: "", workspace: Self.repoA, focus: nil), in: older) == nil)
    }

    // MARK: - A repository header's actions

    /// "New Terminal in X" opens in the checkout of the repository whose
    /// header was clicked, found by its uuid: two repositories on one runner
    /// can both be called "overnight", and the first checkout by that name is
    /// the other one's. Only a CLI too old to send `repository_id` is matched
    /// by the name.
    @Test func newTerminalFindsItsCheckoutByTheRepositorysID() {
        var first = Self.worktree("first", workspace: nil, repository: Self.repoA)
        first.is_main_checkout = true
        var second = Self.worktree("second", workspace: nil, repository: Self.repoB)
        second.is_main_checkout = true
        let lane = Self.worktree("lane", workspace: nil, repository: Self.repoB)
        let worktrees = [first, lane, second]
        func checkout(_ repository: String?, host: String = "") -> String? {
            ContentView.mainCheckout(
                host: host, repositoryID: repository, project: "overnight", in: worktrees)?.id
        }
        #expect(checkout(Self.repoB) == "second")
        #expect(checkout(Self.repoA) == "first")
        #expect(checkout(Self.repoB, host: "remote") == nil, "another runner's")
        var older = first
        older.repositoryID = nil
        var another = second
        another.repositoryID = nil
        another.repository = "billing-service"
        #expect(
            ContentView.mainCheckout(
                host: "", repositoryID: nil, project: "overnight", in: [lane, another, older])?.id
                == "first", "an older CLI's checkout is found by its name")
    }

    // MARK: - Dragging a worktree onto another workspace

    /// Dropping a row reorders within its own workspace, and moves the
    /// worktree to another workspace of its repository — dropped on one of
    /// that workspace's rows or on its header — the drag's version of
    /// `farcooler worktree assign`. Never into Unclaimed, never the main
    /// checkout, never on a runner without `workstreams`, and never across
    /// repositories.
    @Test func aWorktreeDroppedOnAnotherWorkspaceMovesThere() {
        let billing = Self.summary(Self.billing, "Billing")
        let main = Self.summary(Self.main, "Main", isMain: true)
        let lane = Self.worktree("lane", workspace: Self.main)
        let next = Self.worktree("next", workspace: Self.main)
        let bill = Self.worktree("bill", workspace: Self.billing)
        let stray = Self.worktree("stray", workspace: nil)
        var checkout = Self.worktree("checkout", workspace: Self.main)
        checkout.is_main_checkout = true
        let elsewhere = Self.worktree("elsewhere", workspace: Self.otherMain, repository: Self.repoB)
        let hidden = Self.worktree("old", workspace: Self.billing, state: "hidden")
        let fleet = Self.fleet(
            workspaces: [main, billing, Self.summary(Self.otherMain, "Main", isMain: true, repository: Self.repoB)],
            worktrees: [lane, next, bill, stray, checkout, elsewhere, hidden])
        func meaning(
            _ dragged: Worktree, _ target: WorktreeDrag.Target, assigns: Bool = true
        ) -> ContentView.DropMeaning? {
            ContentView.dropMeaning(dragged, onto: target, in: fleet, assigns: assigns)
        }
        #expect(meaning(lane, .worktree("next", .above)) == .reorder)
        #expect(meaning(lane, .worktree("bill", .below)) == .assign(billing))
        #expect(meaning(lane, .workspace(Self.billing)) == .assign(billing))
        #expect(meaning(stray, .worktree("bill", .above)) == .assign(billing), "Unclaimed is claimed")
        #expect(meaning(stray, .workspace(Self.main)) == .assign(main))
        #expect(meaning(lane, .workspace(Self.main)) == nil, "already there")
        #expect(meaning(lane, .worktree("stray", .above)) == nil, "into Unclaimed")
        #expect(meaning(bill, .worktree("stray", .below)) == nil, "into Unclaimed")
        #expect(meaning(checkout, .workspace(Self.billing)) == nil, "the main checkout")
        #expect(meaning(checkout, .worktree("bill", .above)) == nil, "the main checkout")
        #expect(meaning(checkout, .worktree("lane", .above)) == .reorder, "the checkout still reorders")
        #expect(meaning(lane, .workspace(Self.billing), assigns: false) == nil, "no workstreams")
        #expect(meaning(lane, .worktree("bill", .above), assigns: false) == nil, "no workstreams")
        #expect(meaning(lane, .workspace(Self.otherMain)) == nil, "another repository's")
        #expect(meaning(lane, .worktree("elsewhere", .above)) == nil, "another repository's")
        #expect(meaning(lane, .worktree("old", .above)) == nil, "a hidden row")
        #expect(meaning(hidden, .worktree("bill", .above)) == nil, "a hidden row dragged")
        #expect(meaning(lane, .worktree("lane", .above)) == nil, "onto itself")
        let loose = Self.worktree("loose", workspace: nil, repository: Self.repoB)
        let across = Self.fleet(workspaces: [main], worktrees: [stray, loose])
        #expect(
            ContentView.dropMeaning(stray, onto: .worktree("loose", .above), in: across, assigns: true) == nil,
            "Unclaimed onto another repository's Unclaimed")

        // The seam the hover and the drop share: by id, asking whether that
        // worktree's runner assigns.
        var asked: [String] = []
        let seam = ContentView.dropMeaning(
            of: "lane", onto: .workspace(Self.billing), in: fleet,
            assigns: { asked.append($0.id); return true })
        #expect(seam?.0.id == "lane")
        #expect(seam?.1 == .assign(billing))
        #expect(asked == ["lane"])
        #expect(
            ContentView.dropMeaning(of: "lane", onto: .workspace(Self.billing), in: fleet, assigns: { _ in false })
                == nil)
        #expect(ContentView.dropMeaning(of: "gone", onto: .workspace(Self.billing), in: fleet, assigns: { _ in true }) == nil)

        var remote = lane
        remote.host = "remote"
        #expect(meaning(remote, .workspace(Self.billing)) == nil, "another runner's")
    }

    /// A row refuses a card it would only ignore — no insertion line, no
    /// drop — by asking the sidebar's rule while it hovers, and a header
    /// lights only for a card it would take.
    @Test func aRowTakesOnlyADropThatMeansSomething() {
        let drag = WorktreeDrag.shared
        let before = drag.accepts
        defer {
            drag.accepts = before
            drag.cancel()
        }
        drag.accepts = { _, target in target != .workspace("unclaimed") && target != .worktree("stray", .above) }

        drag.begin("lane")
        drag.hover("stray", .above)
        #expect(drag.landing == nil, "an insertion line for a drop that does nothing")
        drag.hover(workspace: "unclaimed")
        #expect(drag.workspaceLanding == nil)
        #expect(!drag.drop(on: .worktree("stray", .above)))
        #expect(drag.completion?.target != .worktree("stray", .above))

        drag.begin("lane")
        drag.hover(workspace: "billing")
        #expect(drag.workspaceLanding == "billing")
        #expect(drag.drop(on: .workspace("billing")))
        #expect(drag.completion?.target == .workspace("billing"))
        #expect(drag.workspaceLanding == nil, "the header stayed lit after the drop")

        // A header lit for a card the rule then refuses goes dark.
        drag.begin("lane")
        drag.hover(workspace: "billing")
        #expect(drag.workspaceLanding == "billing")
        drag.accepts = { _, _ in false }
        drag.hover(workspace: "billing")
        #expect(drag.workspaceLanding == nil, "a refused header stayed lit")
    }

    /// A worktree drag that ended without a drop — released over a row that
    /// refused it, over nothing, or with Escape — has no end hook to clear
    /// it. A later, unrelated drag released on a workspace header must not
    /// move that worktree: a pane drag beginning ends it.
    @Test func anUnrelatedDropAfterARefusedWorktreeDragAssignsNothing() {
        let drag = WorktreeDrag.shared
        let before = drag.accepts
        defer {
            drag.accepts = before
            drag.cancel()
        }
        drag.accepts = { _, target in target != .worktree("stray", .above) }
        let completed = drag.completion

        drag.begin("lane")
        drag.hover("stray", .above)  // refused, so no drop is ever performed
        PaneDrag.shared.begin("t7")
        drag.hover(workspace: "billing")
        #expect(drag.workspaceLanding == nil, "a pane drag lit a header for a worktree")
        #expect(!drag.drop(on: .workspace("billing")))
        #expect(drag.completion == completed, "a stale worktree was dropped")

        // Text dragged in from another app begins no pane drag here. The drop
        // is read, and its payload isn't the stale worktree's id.
        drag.begin("lane")
        drag.hover("stray", .above)
        #expect(!drag.drop(on: .workspace("billing"), carrying: "some text"))
        #expect(drag.completion == completed, "a stale worktree was dropped by text from elsewhere")
        #expect(drag.dragged == nil)
    }

    // MARK: - An orchestrator's pane in the detail

    /// Selecting an orchestrator's row selects its pane, which the runner
    /// opens in the main checkout — as its own tmux window, so the layout
    /// holding it is its own. The detail shows that layout alone, titled
    /// with the workspace, rather than as one of the checkout's layouts
    /// under the checkout's name. A shell in the checkout keeps the
    /// checkout's layouts and title.
    @Test func anOrchestratorsPaneIsFramedAsItsWorkspaces() {
        let conductor = Self.orchestrator("conductor", workspace: Self.billing)
        let shell = Self.terminal("shell", preset: "zsh", taskId: nil)
        var checkout = Self.worktree("checkout", workspace: Self.main, terminals: [conductor, shell])
        checkout.is_main_checkout = true
        let rows = ContentView.sidebarRows(fleet: Self.fleet(
            workspaces: [
                Self.summary(Self.main, "Main", isMain: true),
                Self.summary(Self.billing, "Billing", orchestrator: "conductor"),
            ],
            worktrees: [checkout]))
        func pane(_ id: String) -> PaneRect {
            PaneRect(id: id, short: id, title: nil, left: 0, top: 0, columns: 80, rows: 24, focused: true, zoomed: false)
        }
        let shells = PaneGroup(id: "@1", name: "zsh", active: true, columns: 80, rows: 24, layout: "a", panes: [pane("shell")])
        let own = PaneGroup(id: "@2", name: "orchestrator", active: false, columns: 80, rows: 24, layout: "b", panes: [pane("conductor")])

        let fleet = Self.fleet(
            workspaces: [
                Self.summary(Self.main, "Main", isMain: true),
                Self.summary(Self.billing, "Billing", orchestrator: "conductor"),
            ],
            worktrees: [checkout])
        #expect(rows.contains { $0.kind == .workspace("Billing") && $0.orchestrator?.terminal.id == "conductor" })
        func shown(_ selection: ContentView.Selection) -> [ShownLayout] {
            WorkspaceScreen.shown(selection, in: fleet, layouts: { _, _ in [shells, own] })
        }
        let billing = shown(.workspace(host: "", workspace: Self.billing, focus: nil))
        #expect(billing.map(\.column) == [.conversation])
        #expect(billing.first?.groups.map(\.id) == ["@2"], "the checkout's other layouts offered beside it")
        guard let framed = billing.first else { return }
        #expect(ContentView.frame(of: framed, in: fleet).title == "Billing")
        #expect(ContentView.frame(of: framed, in: fleet).subtitle == "overnight · Orchestrator")

        let asShell = shown(.workspace(host: "", workspace: Self.main, focus: .worktree("checkout", terminal: "shell")))
        #expect(asShell.map(\.column) == [.worktree], "Main has no orchestrator of its own here")
        #expect(asShell.first?.groups.map(\.id) == ["@1"], "Billing's orchestrator offered in the checkout's bar")
        #expect(asShell.first.map { ContentView.frame(of: $0, in: fleet).title } == checkout.windowTitle)
        #expect(shown(.workspace(host: "remote", workspace: Self.billing, focus: nil)).isEmpty)
    }

    // MARK: - The main checkout's own layouts

    /// The checkout's layouts and the orchestrator's window, as tmux has them
    /// once the orchestrator was the last thing focused: its window active.
    private static func checkoutWithAnOrchestrator() -> (
        checkout: Worktree, rows: [SidebarEntry], layouts: [PaneGroup]
    ) {
        let conductor = Self.orchestrator("conductor", workspace: Self.billing)
        var checkout = Self.worktree(
            "checkout", workspace: Self.main,
            terminals: [
                conductor, Self.terminal("s1", preset: "zsh", taskId: nil),
                Self.terminal("s2", preset: "zsh", taskId: nil), Self.terminal("s3", preset: "zsh", taskId: nil),
            ])
        checkout.is_main_checkout = true
        let rows = ContentView.sidebarRows(fleet: Self.fleet(
            workspaces: [
                Self.summary(Self.main, "Main", isMain: true),
                Self.summary(Self.billing, "Billing", orchestrator: "conductor"),
            ],
            worktrees: [checkout]))
        func pane(_ id: String, left: Int = 0, focused: Bool = false) -> PaneRect {
            PaneRect(
                id: id, short: id, title: nil, left: left, top: 0, columns: 40, rows: 24,
                focused: focused, zoomed: false)
        }
        func window(_ id: String, active: Bool, _ panes: [PaneRect]) -> PaneGroup {
            PaneGroup(id: id, name: "", active: active, columns: 80, rows: 24, layout: id, panes: panes)
        }
        let layouts = [
            window("@1", active: false, [pane("s1", focused: true), pane("s2", left: 40)]),
            window("@2", active: true, [pane("conductor", focused: true)]),
            window("@3", active: false, [pane("s3", focused: true)]),
        ]
        return (checkout, rows, layouts)
    }

    /// Selecting the main checkout's row opens on its own layout, never on
    /// the orchestrator's window — which the runner opens in the checkout's
    /// tmux session, and which tmux calls the active one once it was the
    /// last focused. Its bar offers only its own layouts, whichever of its
    /// panes is selected; the orchestrator is reached through its own row.
    /// That holds with no row drawn for it too: a fleet read with no
    /// workspaces for the runner still lists the orchestrator's window.
    @Test func theCheckoutsRowNeverOpensOnAnOrchestratorsWindow() {
        let (checkout, rows, layouts) = Self.checkoutWithAnOrchestrator()
        #expect(rows.contains { $0.kind == .workspace("Billing") && $0.orchestrator?.terminal.id == "conductor" })
        #expect(ContentView.orchestrators(in: checkout) == ["conductor"])

        #expect(ContentView.shownLayout(layouts, of: checkout)?.id == "@1", "opened on the orchestrator")
        #expect(ContentView.ownLayouts(layouts, of: checkout).map(\.id) == ["@1", "@3"])
        // Its own active layout, when it has one, is still the one shown.
        var third = layouts
        third[1].active = false
        third[2].active = true
        #expect(ContentView.shownLayout(third, of: checkout)?.id == "@3")

        let fleet = Self.fleet(workspaces: nil, worktrees: [checkout])
        let framed = WorkspaceScreen.shown(
            .looseWorktree(host: "", worktree: "checkout", terminal: "s3"), in: fleet, layouts: { _, _ in layouts })
        #expect(framed.first?.groups.map(\.id) == ["@1", "@3"], "the checkout's bar offers the orchestrator")

        // A shell moved into the orchestrator's window is shown there, alone,
        // rather than in a bar that doesn't list the layout on screen.
        var shared = layouts[1]
        shared.panes.append(layouts[2].panes[0])
        let moved = WorkspaceScreen.shown(
            .looseWorktree(host: "", worktree: "checkout", terminal: "s3"), in: fleet,
            layouts: { _, _ in [layouts[0], shared] })
        #expect(moved.first?.groups.map(\.id) == ["@2"])

        // No orchestrator row drawn — no workspaces in the fleet read for
        // this runner — and its window is still left out.
        let unseated = ContentView.sidebarRows(fleet: Self.fleet(workspaces: nil, worktrees: [checkout]))
        #expect(!unseated.contains { $0.orchestrator != nil })
        #expect(
            ContentView.shownLayout(layouts, of: checkout)?.id == "@1",
            "opened on the orchestrator with no row drawn for it")
        // A worktree with no orchestrator: every layout is its own.
        var lane = checkout
        lane.terminals.removeAll { $0.isOrchestrator }
        #expect(ContentView.shownLayout(layouts, of: lane)?.id == "@2")
    }

    /// The number keys stay in the checkout's own panes with an orchestrator
    /// running beside them. ⌘1… selects terminals in the order the sidebar
    /// draws them, the orchestrator in its own row. ⌃B and a digit counts
    /// the panes of the layout on screen, not of the orchestrator's window
    /// tmux calls active, and ⌃B n and ⌃B p step through the checkout's own
    /// layouts, never into the orchestrator's.
    @Test func numberKeysStayInTheCheckoutsOwnLayouts() {
        let (checkout, _, layouts) = Self.checkoutWithAnOrchestrator()
        let fleet = Self.fleet(
            workspaces: [
                Self.summary(Self.main, "Main", isMain: true),
                Self.summary(Self.billing, "Billing", orchestrator: "conductor"),
            ],
            worktrees: [checkout])
        _ = fleet

        let shown = ContentView.shownLayout(layouts, of: checkout)
        #expect(ContentView.pane(numbered: 1, in: shown)?.id == "s1")
        #expect(ContentView.pane(numbered: 2, in: shown)?.id == "s2", "⌃B 2 counted another layout")
        #expect(ContentView.pane(numbered: 3, in: shown) == nil)
        #expect(ContentView.pane(numbered: 0, in: shown) == nil)

        let own = ContentView.ownLayouts(layouts, of: checkout)
        #expect(ContentView.layout(stepping: 1, from: "@1", in: own)?.id == "@3", "⌃B n into the orchestrator")
        #expect(ContentView.layout(stepping: 1, from: "@3", in: own)?.id == "@1")
        #expect(ContentView.layout(stepping: -1, from: "@1", in: own)?.id == "@3")
        #expect(ContentView.layout(stepping: 1, from: "@2", in: [layouts[1]]) == nil)
    }

    // MARK: - Searching for an orchestrator

    /// Typing an orchestrator's name finds its own row, under its
    /// workspace, and not the checkout it runs in — which, with the
    /// orchestrator drawn elsewhere, would be a hit with nothing in it that
    /// matched. A search for the checkout's own shell still finds the
    /// checkout, and a search nothing matches leaves the repository out.
    @Test func searchingForAnOrchestratorFindsItsRow() {
        var conductor = Self.orchestrator("conductor", workspace: Self.billing)
        conductor.title = "orchestrator"
        var shell = Self.terminal("shell", preset: "zsh", taskId: nil)
        shell.title = "server"
        var checkout = Self.worktree("checkout", workspace: Self.main, terminals: [conductor, shell])
        checkout.is_main_checkout = true
        let fleet = Self.fleet(
            workspaces: [
                Self.summary(Self.main, "Main", isMain: true),
                Self.summary(Self.billing, "Billing", orchestrator: "conductor"),
            ],
            worktrees: [checkout, Self.worktree("bill", workspace: Self.billing)])

        let found = ContentView.sidebarRows(fleet: fleet, query: "orchestr")
        #expect(found.map(\.kind) == [.repository, .workspace("Billing")])
        #expect(found.last?.orchestrator?.terminal.id == "conductor")

        let server = ContentView.sidebarRows(fleet: fleet, query: "server")
        #expect(server.map(\.kind) == [.repository, .workspace("Main"), .worktrees(count: 1), .worktree("checkout")])
        #expect(server.last?.worktree?.terminals.map(\.id) == ["shell"])

        #expect(ContentView.sidebarRows(fleet: fleet, query: "nothing like it").isEmpty)
    }
}
