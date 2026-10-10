import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The plan layer on the Mac (ov-273), the primary way of working since
/// ov-298: no Tasks | Plan control. A planned board's navigator lists its
/// themes and the task index as peers; a runner without `board_plan`, or a
/// board with nothing planned, is drawn as it always was.
@MainActor
@Suite(.serialized)
struct PlanViewTests {
    /// `test/fixtures/plan.json`: the CLI's `plan --json`, byte for byte.
    static func fixture() throws -> Data {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try Data(contentsOf: root.appendingPathComponent("test/fixtures/plan.json"))
    }

    /// The fixture's three cards as the board lists them; ov-1 waits on a
    /// decision.
    static let tasks = [
        ("00000000-0000-0000-0000-000000001001", "ov-1", "needs_decision"),
        ("00000000-0000-0000-0000-000000001002", "ov-2", "backlog"),
        ("00000000-0000-0000-0000-000000001003", "ov-3", "done"),
    ]

    /// What the stubbed CLI was asked, in order, and the plan it answers
    /// with, which a test may change between reads.
    final class Calls {
        var args: [[String]] = []
        var plan: Data?
    }

    /// The fixture with nothing planned: no theme, no lane, no ruling.
    static func emptyPlan() throws -> Data {
        var object = try #require(try JSONSerialization.jsonObject(with: fixture()) as? [String: Any])
        for key in ["themes", "lanes", "order", "cards", "rulings"] { object[key] = [] }
        return try JSONSerialization.data(withJSONObject: object)
    }

    static func store(plan: Bool, defaults: UserDefaults, calls: Calls = Calls()) async throws -> TaskBoardStore {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        if calls.plan == nil { calls.plan = try fixture() }
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        client.commandRunnerForTesting = { args in
            calls.args.append(args)
            if args.starts(with: ["task", "list"]) {
                let rows = Self.tasks.map { id, key, status in
                    #"{"id":"\#(id)","key":"\#(key)","title":"Title of \#(key)","status":"\#(status)","status_since":\#(now),"created_at":\#(now),"updated_at":\#(now)}"#
                }
                return (Data(#"{"tasks":[\#(rows.joined(separator: ","))]}"#.utf8), nil)
            }
            if args.first == "plan" { return (calls.plan, nil) }
            return (Data(), nil)
        }
        client.daemonBuild = DaemonBuild(
            version: "test", matches: true, platform: "macos",
            capabilities: Set(Capability.allCases.map(\.rawValue).filter { plan || $0 != "board_plan" }))
        let store = TaskBoardStore(
            client: client, workspace: .implicit(repository: "r"), readStore: DefaultsBoardReads(defaults))
        store.planDefaults = defaults
        await store.readIfNeverRead()
        return store
    }

    struct Hosted: View {
        let store: TaskBoardStore
        let defaults: UserDefaults
        let seen: NavigatorFilterTests.Seen
        let opened: (PlanPage) -> Void
        /// An orchestrator, terminals and worktrees in the navigator, as well
        /// as the tasks.
        var full = false

        var body: some View {
            TaskBoardView(
                store: store, client: store.client, agents: .none, onGoTo: { _ in }, defaults: defaults,
                hasKeyboard: true, worktrees: { _ in full ? PlanViewTests.loose : .none },
                orchestrator: full
                    ? NavigatorOrchestrator(state: .working, agent: "claude", status: .working, nowDoing: "Reading")
                    : nil,
                onPlan: opened)
            .frame(width: 300, height: 900, alignment: .topLeading)
            .environment(\.gridProbing, true)
            .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                GeometryReader { proxy in
                    let _ = seen.views = Dictionary(
                        probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                    Color.clear
                }
            }
        }
    }

    /// Hosted's terminals and worktrees, when `full`.
    static let loose: BoardWorktrees = {
        var shell = ProjectTerminals(terminals: [
            Terminal(id: "term0", short: "term0", title: "proxy", preset: "zsh", state: "running", epoch: 0)
        ])
        shell.onNew = {}
        var loose = BoardWorktrees(
            shown: [
                Worktree(
                    id: "w0", short: "w0", task: "spike-0", branch: "spike-0", repository: "r", host: "",
                    path: "/tmp/w0", state: "active", terminals: [], repositoryID: "r", workspace: nil)
            ], terminals: shell)
        loose.onNew = {}
        return loose
    }()

    /// The board drawn offscreen: what it drew, and where.
    final class Drawn {
        let seen = NavigatorFilterTests.Seen()
        var opened: [PlanPage] = []
        var host: NSHostingView<Hosted>!
        var window: NavigatorFilterTests.KeyWindow!

        var ids: Set<String> { Set(seen.views.keys) }

        @MainActor
        func settle() async {
            for _ in 0..<15 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }

        @MainActor
        func press(_ id: String) -> Bool {
            guard let frame = seen.views[id] else { return false }
            let at = NSPoint(x: frame.midX, y: 900 - frame.midY)
            for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
                window.sendEvent(
                    NSEvent.mouseEvent(
                        with: type, location: at, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                        windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!)
            }
            return true
        }
    }

    static func draw(_ store: TaskBoardStore, defaults: UserDefaults, full: Bool = false) async -> Drawn {
        let drawn = Drawn()
        drawn.host = NSHostingView(
            rootView: Hosted(
                store: store, defaults: defaults, seen: drawn.seen, opened: { drawn.opened.append($0) }, full: full))
        drawn.window = NavigatorFilterTests.KeyWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: 300, height: 900), styleMask: [.borderless],
            backing: .buffered, defer: false)
        drawn.window.isReleasedWhenClosed = false
        drawn.window.contentView = drawn.host
        drawn.window.makeKeyAndOrderFront(nil)
        await drawn.settle()
        return drawn
    }

    static func defaults() -> UserDefaults { UserDefaults(suiteName: "ov273-\(UUID().uuidString)")! }

    @Test("A runner without board_plan draws the board as it was, and is never asked for a plan")
    func oldRunner() async throws {
        let defaults = Self.defaults()
        let calls = Calls()
        let store = try await Self.store(plan: false, defaults: defaults, calls: calls)
        let drawn = await Self.draw(store, defaults: defaults)
        defer { drawn.window.close() }
        #expect(!drawn.ids.contains("plan-themes"))
        #expect(drawn.ids.contains("board-row-ov-1"), "the task rows are drawn: \(drawn.ids)")
        #expect(drawn.ids.contains("section-header-summary"), "Unread, as before")
        #expect(!calls.args.contains { $0.first == "plan" })
    }

    @Test("A board with nothing planned is the board as it was: the same views, the same frames")
    func nothingPlanned() async throws {
        let old = Self.defaults()
        let without = await Self.draw(try await Self.store(plan: false, defaults: old), defaults: old)
        defer { without.window.close() }
        let fresh = Self.defaults()
        let calls = Calls()
        calls.plan = try Self.emptyPlan()
        let store = try await Self.store(plan: true, defaults: fresh, calls: calls)
        let with = await Self.draw(store, defaults: fresh)
        defer { with.window.close() }
        for _ in 0..<20 where !store.plan.hasRead { await with.settle() }
        await with.settle()
        #expect(store.plan.hasRead && !store.plan.planned, "read, and nothing in it")
        #expect(with.ids == without.ids, "added \(with.ids.subtracting(without.ids)), lost \(without.ids.subtracting(with.ids))")
        for id in without.ids {
            let (a, b) = (without.seen.views[id]!, with.seen.views[id]!)
            #expect(abs(a.minY - b.minY) < 0.5 && abs(a.height - b.height) < 0.5, "\(id) moved")
        }
    }

    @Test("A planned board's navigator: Themes, then the task index, collapsed, with no Unread and no toggle")
    func plannedNavigator() async throws {
        let defaults = Self.defaults()
        let store = try await Self.store(plan: true, defaults: defaults)
        let drawn = await Self.draw(store, defaults: defaults)
        defer { drawn.window.close() }
        for _ in 0..<20 where !drawn.ids.contains("plan-themes") { await drawn.settle() }
        #expect(store.plan.planned)
        #expect(drawn.ids.isSuperset(of: ["section-header-themes", "plan-theme-Visual language", "section-header-tasks"]))
        #expect(!drawn.ids.contains("board-plan-toggle"))
        #expect(!drawn.ids.contains("section-header-summary"), "Unread isn't in the index")
        #expect(!drawn.ids.contains("board-row-ov-1"), "every status group starts closed")
        #expect(drawn.ids.contains("section-header-status.needs_decision"), "the index's groups are drawn: \(drawn.ids)")
        let themes = try #require(drawn.seen.views["section-header-themes"])
        let tasks = try #require(drawn.seen.views["section-header-tasks"])
        #expect(themes.maxY <= tasks.minY + 0.5, "Themes comes before Tasks")
        #expect(drawn.ids.contains("plan-theme-Visual language-progress"), "each theme says how far it is")
        #expect(PlanNavigator.progress(try #require(store.plan.plan.themes.first).counts) == "1/3")
        #expect(drawn.press("plan-theme-Visual language"))
        await drawn.settle()
        #expect(drawn.opened.last == .theme("00000000-0000-0000-0000-000000003001"))
    }

    @Test("Terminals and Worktrees stay on a planned board, and the orchestrator's row is one line")
    func plannedKeepsTheRest() async throws {
        let defaults = Self.defaults()
        let store = try await Self.store(plan: true, defaults: defaults)
        let drawn = await Self.draw(store, defaults: defaults, full: true)
        defer { drawn.window.close() }
        for _ in 0..<20 where !drawn.ids.contains("plan-themes") { await drawn.settle() }
        let kept: Set<String> = [
            "navigator-divider", "section-header-themes", "section-header-tasks", "section-header-terminals",
            "section-header-worktrees", "navigator-terminal-term0", "navigator-pane-terminals", "navigator-pane-worktrees",
        ]
        #expect(drawn.ids.isSuperset(of: kept), "lost: \(kept.subtracting(drawn.ids))")
        let row = try #require(drawn.seen.views["navigator-orchestrator"])
        #expect(row.height < 1.5 * ColumnGrid.rowHeight, "one line, not \(row.height) pt")
    }

    @Test("The task index's groups start closed, and what's opened is kept per board, apart from the task list's")
    func indexKeptPerBoard() {
        let defaults = Self.defaults()
        #expect(PlanNavigator.collapsed(host: "local", workspace: "a", from: defaults) == Set(TaskBoardModel.order))
        PlanNavigator.setCollapsed([.done], host: "local", workspace: "a", in: defaults)
        #expect(PlanNavigator.collapsed(host: "local", workspace: "a", from: defaults) == [.done])
        #expect(PlanNavigator.collapsed(host: "local", workspace: "b", from: defaults) == Set(TaskBoardModel.order))
        #expect(BoardForm.collapsed(host: "local", workspace: "a", from: defaults) == BoardForm.collapsedByDefault)
    }

    @Test("A plan line from the runner re-reads that board's plan, and no other's, through the client's own stream")
    func planEventRereads() async throws {
        let calls = Calls()
        let store = try await Self.store(plan: true, defaults: Self.defaults(), calls: calls)
        await store.plan.reloadIfMoved()
        let reads = { calls.args.filter { $0.first == "plan" }.count }
        #expect(reads() == 1)
        await store.plan.reloadIfMoved()
        #expect(reads() == 1, "nothing moved")

        // The client's real `events` stream, from a stand-in CLI that says
        // another board's plan moved, then this one's, then waits.
        let client = store.client
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ov273-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let cli = directory.appendingPathComponent("farcooler")
        let lines = [
            #"{"kind":"plan","workspace":"someone-else","actor":"manager"}"#,
            #"{"kind":"plan","workspace":"\#(store.workspace.id)","actor":"manager"}"#,
        ]
        try "#!/bin/sh\nprintf '%s\\n' '\(lines.joined(separator: "' '"))'\nexec sleep 30\n"
            .write(to: cli, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: cli.path)
        client.binaryForTesting = cli.path
        let before = store.plan.generation
        client.startEvents()
        defer { client.stopEvents() }
        for _ in 0..<250 where client.planNews[store.workspace.id] == nil {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(client.planNews["someone-else"] == 1, "heard: \(client.planNews)")
        #expect(client.planNews[store.workspace.id] == 1, "heard: \(client.planNews)")
        #expect(store.plan.generation == before + 1, "only this board's line moves its plan")
        await store.plan.reloadIfMoved()
        #expect(reads() == 2, "this board's plan moved, once")
    }

    @Test("A task's page names its lane and theme wherever the runner keeps a plan")
    func taskLine() async throws {
        func drawn(_ store: TaskBoardStore) async -> Set<String> {
            let seen = NavigatorFilterTests.Seen()
            let host = NSHostingView(
                rootView: PlanTaskLineView(plan: store.plan, task: "00000000-0000-0000-0000-000000001001") { _ in }
                    .frame(width: 600, height: 60)
                    .environment(\.gridProbing, true)
                    .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                        let _ = seen.views = Dictionary(probed.map { ($0.id, .zero) }, uniquingKeysWith: { a, _ in a })
                        Color.clear
                    })
            host.frame = CGRect(x: 0, y: 0, width: 600, height: 60)
            for _ in 0..<10 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
            return Set(seen.views.keys)
        }
        let old = try await Self.store(plan: false, defaults: Self.defaults())
        #expect(!(await drawn(old)).contains("plan-task-line"))
        let store = try await Self.store(plan: true, defaults: Self.defaults())
        #expect((await drawn(store)).contains("plan-task-line"), "read on its own, with no navigator")
        #expect(store.plan.plan.taskLine("00000000-0000-0000-0000-000000001001")?.text == "In lane Mac interface polish · Visual language")
    }

    @Test("A plan page is kept across a relaunch, and its crumb is its name")
    func pageKept() {
        let theme = ContentView.Selection.workspace(host: "h", workspace: "w", focus: .plan(.theme("t-1")))
        let lane = ContentView.Selection.workspace(host: "h", workspace: "w", focus: .plan(.lane("l-1")))
        #expect(SelectionMemory.decode(SelectionMemory.encode(theme)!) == theme)
        #expect(SelectionMemory.decode(SelectionMemory.encode(lane)!) == lane)
        #expect(SelectionMemory.decode("h|w|plan:lane:") == nil)
        let crumbs = WorkspaceNavigation.crumbs(
            lane, trail: nil, workspace: "Main", task: { $0 }, worktree: { $0 }, plan: { _ in "mac-ux" })
        #expect(crumbs.map(\.title) == ["Main", "mac-ux"])
        #expect(crumbs.first?.target == .workspace(host: "h", workspace: "w", focus: nil))
    }
}
