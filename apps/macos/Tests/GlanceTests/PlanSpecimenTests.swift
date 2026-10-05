import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The Plan view, a theme's page, a lane's page and a task page's line
/// (ov-273), rendered offscreen in both appearances at a narrow and a wide
/// width, written where `FARCOOLER_GLANCE_OUT` says. Beside them, the board
/// with Tasks chosen and the board on a runner without a plan, which should
/// differ by the control alone.
///
/// With `FARCOOLER_PLAN_HOME` set to a scratch daemon's home (and
/// `FARCOOLER_BIN` to the CLI), every read goes to that daemon through the
/// real CLI: the captures in `.claude/agent/reports/ov-273/` are of a board
/// seeded by `seed.sh`. Without it, the shared fixture draws.
@MainActor
struct PlanSpecimenTests {
    struct Source {
        var store: TaskBoardStore
        var old: TaskBoardStore
    }

    /// Run the CLI against the scratch daemon, as `DaemonClient.runRaw` would.
    nonisolated static func cli(_ args: [String], bin: String, home: String) -> (Data?, String?) {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: bin)
        process.arguments = args
        process.environment = ProcessInfo.processInfo.environment.merging(["FARCOOLER_HOME": home]) { _, new in new }
        let out = Pipe()
        process.standardOutput = out
        process.standardError = Pipe()
        do { try process.run() } catch { return (nil, "\(error)") }
        let data = out.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        return process.terminationStatus == 0 ? (data, nil) : (nil, "exit \(process.terminationStatus)")
    }

    static func source() async throws -> Source {
        let env = ProcessInfo.processInfo.environment
        func client(plan: Bool) -> DaemonClient {
            let client = DaemonClient(target: "", notifications: NotificationCenter())
            if let home = env["FARCOOLER_PLAN_HOME"], let bin = env["FARCOOLER_BIN"] {
                client.commandRunnerForTesting = { args in Self.cli(args, bin: bin, home: home) }
            } else {
                let plan = try? Data(contentsOf: Self.root.appendingPathComponent("test/fixtures/plan.json"))
                client.commandRunnerForTesting = { args in
                    if args.first == "plan" { return (plan, nil) }
                    if args.starts(with: ["task", "list"]) {
                        return (Data(#"{"tasks":[{"id":"00000000-0000-0000-0000-000000001001","key":"ov-1","title":"Title of ov-1","status":"in_progress","status_since":0,"created_at":0,"updated_at":0}]}"#.utf8), nil)
                    }
                    return (Data(), nil)
                }
            }
            client.daemonBuild = DaemonBuild(
                version: "capture", matches: true, platform: "macos",
                capabilities: Set(Capability.allCases.map(\.rawValue).filter { plan || $0 != "board_plan" }))
            return client
        }
        var workspace = WorkspaceSummary.implicit(repository: "r")
        if let home = env["FARCOOLER_PLAN_HOME"], let bin = env["FARCOOLER_BIN"],
            let data = cli(["workspace", "list", "--json"], bin: bin, home: home).0,
            let listed = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let first = (listed["workspaces"] as? [[String: Any]])?.first,
            let id = first["id"] as? String, let repository = first["repository"] as? String
        {
            workspace = WorkspaceSummary(id: id, name: "Main", taskPrefix: "ov", isMain: true, ordinal: 0, repository: repository)
        }
        let defaults = UserDefaults(suiteName: "ov273-capture-\(UUID().uuidString)")!
        func store(plan: Bool) async -> TaskBoardStore {
            let store = TaskBoardStore(
                client: client(plan: plan), workspace: workspace, readStore: DefaultsBoardReads(defaults))
            store.planDefaults = defaults
            await store.readIfNeverRead()
            // Everything read, so Unread doesn't list the whole board.
            for row in store.board.rows { store.markRead(row) }
            return store
        }
        let made = await store(plan: true)
        await made.plan.reload()
        return Source(store: made, old: await store(plan: false))
    }

    static var root: URL {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return root
    }

    static var directory: URL {
        URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
    }

    /// Draw `view` at `size` in a borderless offscreen window, both
    /// appearances, and write `name-light.png` and `name-dark.png`.
    static func write<V: View>(_ name: String, size: CGSize, @ViewBuilder _ view: () -> V) async throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        for dark in [false, true] {
            let host = NSHostingView(rootView: view().frame(width: size.width, height: size.height, alignment: .topLeading))
            let window = NSWindow(
                contentRect: NSRect(x: -6000, y: -6000, width: size.width, height: size.height),
                styleMask: [.borderless], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            window.contentView = host
            window.orderFrontRegardless()
            for _ in 0..<25 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
            let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: rep)
            let png = try #require(rep.representation(using: .png, properties: [:]))
            try png.write(to: directory.appendingPathComponent("\(name)-\(dark ? "dark" : "light").png"))
            window.close()
        }
    }

    /// The board in the navigator, on the window's plane.
    struct Board: View {
        let store: TaskBoardStore
        var page: PlanPage?

        var body: some View {
            TaskBoardView(
                store: store, client: store.client, agents: .none, onGoTo: { _ in }, defaults: store.planDefaults,
                planPage: page)
            .background(WorkspaceStyle.canvas)
        }
    }

    @Test("Write the Plan view sheets")
    func writeSheets() async throws {
        let source = try await Self.source()
        let store = source.store
        let plan = store.plan
        #expect(plan.hasRead && !plan.plan.isEmpty, "the plan was read")
        let narrow = CGSize(width: 300, height: 1100)
        let wide = CGSize(width: 580, height: 1100)

        // On a runner without a plan, and the planned navigator (ov-298).
        try await Self.write("board-old-runner-narrow", size: narrow) { Board(store: source.old) }
        let model = plan.plan
        let lane = model.lanes.first { $0.name == "mac-rel" } ?? model.lanes[0]
        let theme = model.shownThemes.first { $0.name == "Visual language" } ?? model.themes[0]
        try await Self.write("plan-navigator-narrow", size: narrow) { Board(store: store, page: .theme(theme.id)) }

        // The overview, on its own.
        try await Self.write("plan-overview-narrow", size: narrow) {
            PlanOverviewView(plan: plan, statuses: store.board.statuses, selected: .lane(lane.id), onOpen: { _ in })
                .padding(.horizontal, NavigatorGrid.edge).background(WorkspaceStyle.canvas)
        }
        try await Self.write("plan-overview-wide", size: wide) {
            PlanOverviewView(plan: plan, statuses: store.board.statuses, selected: nil, onOpen: { _ in })
                .padding(.horizontal, NavigatorGrid.edge).background(WorkspaceStyle.canvas)
        }

        // The pages, in the main area beside it.
        let context = PlanPageContext(
            rows: Dictionary(store.board.rows.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a }),
            onTask: { _ in }, onOpen: { _ in })
        await plan.readRecord(.theme(theme.id))
        await plan.readRecord(.lane(lane.id))
        let page = { (width: CGFloat) in CGSize(width: width, height: 1250) }
        try await Self.write("plan-theme-narrow", size: page(560)) {
            PlanThemePage(
                theme: theme, plan: model, record: plan.records[.theme(theme.id)], context: context, showingChange: true)
        }
        try await Self.write("plan-theme-wide", size: page(980)) {
            PlanThemePage(theme: theme, plan: model, record: plan.records[.theme(theme.id)], context: context)
        }
        try await Self.write("plan-lane-narrow", size: page(560)) {
            PlanLanePage(lane: lane, plan: model, record: plan.records[.lane(lane.id)], context: context)
        }
        try await Self.write("plan-lane-wide", size: page(980)) {
            PlanLanePage(lane: lane, plan: model, record: plan.records[.lane(lane.id)], context: context)
        }

        // A task page's header, with its line.
        if let card = lane.cards.first, let row = store.board.rows.first(where: { $0.id == card.task }) {
            try await Self.write("plan-task-line", size: CGSize(width: 760, height: 90)) {
                VStack(spacing: 0) {
                    TaskViewHeader(row: row)
                    PlanTaskLineView(plan: plan, task: row.id) { _ in }
                }
                .background(WorkspaceStyle.paper)
            }
        }
    }
}
