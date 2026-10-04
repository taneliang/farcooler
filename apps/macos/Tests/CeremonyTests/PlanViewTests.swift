import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The plan layer on the Mac (ov-273): opt-in behind a Tasks | Plan control
/// that only a runner with `board_plan` offers, Tasks the default, and a
/// board drawn as it was whenever Plan isn't chosen.
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

    /// What the stubbed CLI was asked, in order.
    final class Calls { var args: [[String]] = [] }

    static func store(plan: Bool, defaults: UserDefaults, calls: Calls = Calls()) async throws -> TaskBoardStore {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let planData = try fixture()
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        client.commandRunnerForTesting = { args in
            calls.args.append(args)
            if args.starts(with: ["task", "list"]) {
                let rows = Self.tasks.map { id, key, status in
                    #"{"id":"\#(id)","key":"\#(key)","title":"Title of \#(key)","status":"\#(status)","status_since":\#(now),"created_at":\#(now),"updated_at":\#(now)}"#
                }
                return (Data(#"{"tasks":[\#(rows.joined(separator: ","))]}"#.utf8), nil)
            }
            if args.first == "plan" { return (planData, nil) }
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

    @Test("A runner without board_plan draws no control and the board as it was")
    func oldRunner() async throws {
        let defaults = Self.defaults()
        let store = try await Self.store(plan: false, defaults: defaults)
        store.plan.shown = true  // even chosen before, on another build
        let drawn = await Self.draw(store, defaults: defaults)
        defer { drawn.window.close() }
        #expect(!drawn.ids.contains("board-plan-toggle"))
        #expect(!drawn.ids.contains("plan-overview"))
        #expect(drawn.ids.contains("board-row-ov-1"), "the task rows are drawn: \(drawn.ids)")
    }

    @Test("With board_plan, Tasks is the default and the board under the control is the board as it was")
    func tasksByDefault() async throws {
        let old = Self.defaults()
        let without = await Self.draw(try await Self.store(plan: false, defaults: old), defaults: old)
        defer { without.window.close() }
        let fresh = Self.defaults()
        let store = try await Self.store(plan: true, defaults: fresh)
        #expect(store.plan.shown == false, "Tasks is the default")
        let with = await Self.draw(store, defaults: fresh)
        defer { with.window.close() }
        #expect(with.ids.contains("board-plan-toggle"))
        #expect(!with.ids.contains("plan-overview"))
        // The same views, the control aside, each the same size: only moved
        // down by the control's row. The navigator's panes fill what height
        // is left, so they give up the control's.
        #expect(with.ids.subtracting(["board-plan-toggle"]) == without.ids)
        let toggle = try #require(with.seen.views["board-plan-toggle"])
        for id in without.ids where !id.hasPrefix("navigator-") {
            let (a, b) = (without.seen.views[id]!, with.seen.views[id]!)
            #expect(abs(a.width - b.width) < 0.5 && abs(a.height - b.height) < 0.5, "\(id) changed size")
            #expect(b.minY > toggle.minY, "\(id) is above the control")
        }
        #expect(store.plan.plan.isEmpty && !store.plan.hasRead, "the plan isn't read until Plan is chosen")
    }

    @Test("Plan shows Next Up with each lane's theme, Now and Themes, and a lane opens its page")
    func planShown() async throws {
        let defaults = Self.defaults()
        let store = try await Self.store(plan: true, defaults: defaults)
        store.plan.shown = true
        let drawn = await Self.draw(store, defaults: defaults)
        defer { drawn.window.close() }
        #expect(drawn.ids.isSuperset(of: ["board-plan-toggle", "plan-overview", "plan-next-up", "plan-now", "plan-themes"]))
        #expect(!drawn.ids.contains("board-row-ov-1"), "no task rows while the plan is shown")
        #expect(drawn.ids.contains("plan-lane-mac-fu3-theme"), "Next Up names the theme it serves")
        #expect(drawn.ids.contains("plan-theme-Visual language"))
        #expect(drawn.press("plan-lane-mac-ux"))
        await drawn.settle()
        #expect(drawn.opened.last == .lane("00000000-0000-0000-0000-000000002002"))
        #expect(drawn.press("plan-theme-Visual language"))
        await drawn.settle()
        #expect(drawn.opened.last == .theme("00000000-0000-0000-0000-000000003001"))
    }

    @Test("Plan replaces only the Tasks section: the orchestrator, the filter, Terminals and Worktrees stay")
    func planReplacesOnlyTasks() async throws {
        let defaults = Self.defaults()
        let store = try await Self.store(plan: true, defaults: defaults)
        let tasks = await Self.draw(store, defaults: defaults, full: true)
        let before = tasks.ids
        tasks.window.close()
        let kept: Set<String> = [
            "navigator-divider", "section-header-tasks", "section-header-terminals", "section-header-worktrees",
            "navigator-terminal-term0", "navigator-pane-terminals", "navigator-pane-worktrees",
        ]
        #expect(before.isSuperset(of: kept), "the board with Tasks chosen: \(before)")
        #expect(before.contains("board-row-ov-1"))

        store.plan.shown = true
        let drawn = await Self.draw(store, defaults: defaults, full: true)
        defer { drawn.window.close() }
        #expect(drawn.ids.isSuperset(of: kept), "lost with Plan chosen: \(kept.subtracting(drawn.ids))")
        #expect(drawn.ids.contains("plan-overview"))
        #expect(!drawn.ids.contains("board-row-ov-1"), "the plan is in the Tasks section, in place of its rows")
    }

    @Test("The choice is kept per board on this Mac")
    func keptPerBoard() async throws {
        let defaults = Self.defaults()
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let a = WorkspaceSummary.implicit(repository: "a")
        let b = WorkspaceSummary.implicit(repository: "b")
        PlanStore(client: client, workspace: a, host: "local", defaults: defaults).shown = true
        #expect(PlanStore(client: client, workspace: a, host: "local", defaults: defaults).shown)
        #expect(!PlanStore(client: client, workspace: b, host: "local", defaults: defaults).shown)
        #expect(!PlanStore(client: client, workspace: a, host: "other", defaults: defaults).shown)
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

    @Test("A task's page names its lane and theme only while the plan is shown")
    func taskLine() async throws {
        let defaults = Self.defaults()
        let store = try await Self.store(plan: true, defaults: defaults)
        await store.plan.reload()
        let seen = NavigatorFilterTests.Seen()
        func drawn() async -> Set<String> {
            let host = NSHostingView(
                rootView: PlanTaskLineView(plan: store.plan, task: "00000000-0000-0000-0000-000000001001") { _ in }
                    .frame(width: 600, height: 60)
                    .environment(\.gridProbing, true)
                    .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                        let _ = seen.views = Dictionary(probed.map { ($0.id, .zero) }, uniquingKeysWith: { a, _ in a })
                        Color.clear
                    })
            host.frame = CGRect(x: 0, y: 0, width: 600, height: 60)
            for _ in 0..<5 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
            return Set(seen.views.keys)
        }
        #expect(!(await drawn()).contains("plan-task-line"))
        store.plan.shown = true
        seen.views = [:]
        #expect((await drawn()).contains("plan-task-line"))
        #expect(store.plan.plan.taskLine("00000000-0000-0000-0000-000000001001")?.text == "In lane mac-ux · Visual language")
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

    @Test("View ▸ Show Plan says what it will do, and acts only where the board offers a plan")
    func menu() {
        var focus = MainWindowFocus(overlayOpen: false, hasNavigator: true)
        #expect(MainWindowFocus.planTitle(nil) == "Show Plan")
        #expect(MainWindowFocus.planTitle(false) == "Show Plan")
        #expect(MainWindowFocus.planTitle(true) == "Show Tasks")
        #expect(MainWindowFocus.togglesPlan(focus, plan: false))
        #expect(!MainWindowFocus.togglesPlan(focus, plan: nil), "a runner without a plan")
        focus.hasNavigator = false
        #expect(!MainWindowFocus.togglesPlan(focus, plan: false))
    }
}
