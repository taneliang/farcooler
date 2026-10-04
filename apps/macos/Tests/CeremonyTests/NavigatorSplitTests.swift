import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The navigator's sections scroll on their own (ov-244): with a long board,
/// Terminals and Worktrees stay on screen; each pane scrolls without moving
/// another; and the heights a drag chose are what the window keeps and draws
/// again.
@MainActor
struct NavigatorSplitTests {
    typealias Pane = NavigatorSplit.Pane

    // MARK: - The heights, as values

    @Test("A long task list leaves the small panes their rows")
    func aLongTaskListLeavesTheSmallPanesTheirRows() {
        let panes = [
            Pane(id: "tasks", fills: true, content: 5000), Pane(id: "terminals", content: 90),
            Pane(id: "worktrees", content: 120),
        ]
        let heights = NavigatorSplit.viewports(panes, room: 700)
        #expect(heights["terminals"] == 90 && heights["worktrees"] == 120, "\(heights)")
        #expect(heights["tasks"] == 490, "\(heights)")
    }

    @Test("Many terminals stop at a third of the room while Tasks wants more")
    func manyTerminalsAreCapped() {
        let panes = [Pane(id: "tasks", fills: true, content: 5000), Pane(id: "terminals", content: 2000)]
        let heights = NavigatorSplit.viewports(panes, room: 900)
        #expect(heights["terminals"] == 300 && heights["tasks"] == 600, "\(heights)")
    }

    @Test("Few tasks: the panes under them take the room they leave")
    func fewTasksLeaveRoom() {
        let panes = [Pane(id: "tasks", fills: true, content: 200), Pane(id: "terminals", content: 2000)]
        let heights = NavigatorSplit.viewports(panes, room: 900)
        #expect(heights["tasks"] == 200 && heights["terminals"] == 700, "\(heights)")
    }

    @Test("A closed pane is only its header, and Tasks closed leaves the rest the room")
    func closedPanes() {
        let panes = [
            Pane(id: "tasks", fills: true, expanded: false, content: 5000), Pane(id: "terminals", content: 2000),
        ]
        let heights = NavigatorSplit.viewports(panes, room: 900)
        #expect(heights["tasks"] == nil && heights["terminals"] == 900, "\(heights)")
    }

    @Test("A drag's height holds, short of Tasks' floor")
    func aChosenHeightHolds() {
        let panes = [Pane(id: "tasks", fills: true, content: 5000), Pane(id: "terminals", content: 2000)]
        let chosen = NavigatorSplit.viewports(panes, room: 900, chosen: ["terminals": 500])
        #expect(chosen["terminals"] == 500 && chosen["tasks"] == 400, "\(chosen)")
        let greedy = NavigatorSplit.viewports(panes, room: 900, chosen: ["terminals": 880])
        #expect(greedy["tasks"] == NavigatorSplit.fillMinimum, "\(greedy)")
        #expect(greedy["terminals"] == 900 - NavigatorSplit.fillMinimum, "\(greedy)")
    }

    @Test("A window too short for every floor fits them all in it")
    func aShortWindow() {
        let panes = [
            Pane(id: "tasks", fills: true, content: 5000), Pane(id: "terminals", content: 2000),
            Pane(id: "worktrees", content: 2000),
        ]
        let heights = NavigatorSplit.viewports(panes, room: 200)
        #expect(abs(heights.values.reduce(0, +) - 200) < 0.001, "\(heights)")
    }

    @Test("A rule dragged up grows the pane under it; over a closed one, the pane over it")
    func dragging() {
        let panes = [
            Pane(id: "tasks", fills: true, content: 5000), Pane(id: "terminals", content: 400),
            Pane(id: "worktrees", expanded: false, content: 400),
        ]
        let heights: [String: CGFloat] = ["tasks": 500, "terminals": 200]
        let up = NavigatorSplit.drag(rule: 1, panes: panes, heights: heights, by: -60)
        #expect(up?.id == "terminals" && up?.height == 260)
        let down = NavigatorSplit.drag(rule: 2, panes: panes, heights: heights, by: 30)
        #expect(down?.id == "terminals" && down?.height == 230)
        #expect(NavigatorSplit.drag(rule: 0, panes: panes, heights: heights, by: 10) == nil)
    }

    @Test("What the window keeps reads back as written")
    func theCodecRoundTrips() {
        let chosen: [String: CGFloat] = ["terminals": 120, "worktrees": 80.4]
        let kept = NavigatorSplit.encode(chosen)
        #expect(kept == "terminals=120,worktrees=80")
        #expect(NavigatorSplit.decode(kept) == ["terminals": 120, "worktrees": 80])
        #expect(NavigatorSplit.decode("junk,terminals=x,worktrees=-3,tasks=10") == ["tasks": 10])
    }

    // MARK: - Drawn

    /// A board of `count` tasks to do, and `terminals` terminals.
    static func board(
        tasks count: Int, terminals: Int = 2, kept: Binding<String>? = nil, closed: [String] = []
    ) async -> TaskBoardView {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let now = Int64(Date().timeIntervalSince1970 * 1000) - 86_400_000
        let rows = (0..<count).map { i in
            let status = ["todo", "in_progress", "backlog"][i % 3]
            return #"{"id":"t\#(i)","key":"ov-\#(100 + i)","title":"Task \#(i)","status":"\#(status)","status_since":\#(now),"created_at":\#(now),"updated_at":\#(now)}"#
        }
        client.commandRunnerForTesting = { args in
            guard args.starts(with: ["task", "list"]) else { return (Data(), nil) }
            return (Data(#"{"tasks":[\#(rows.joined(separator: ","))]}"#.utf8), nil)
        }
        // Its reads kept in defaults of its own: in the standard ones, another
        // suite's board "r" would find its tasks read.
        let store = TaskBoardStore(
            client: client, workspace: .implicit(repository: "r"),
            readStore: DefaultsBoardReads(UserDefaults(suiteName: "split-reads-\(UUID().uuidString)")!))
        await store.readIfNeverRead()
        // Everything read, so Unread is short and the statuses are long.
        for row in store.board.rows { store.markRead(row) }
        var shell = ProjectTerminals(
            terminals: (0..<terminals).map {
                Terminal(id: "term\($0)", short: "term\($0)", title: "proxy \($0)", preset: "zsh", state: "running", epoch: 0)
            })
        shell.onNew = {}
        var loose = BoardWorktrees(
            shown: (0..<2).map {
                Worktree(
                    id: "w\($0)", short: "w\($0)", task: "spike-\($0)", branch: "spike-\($0)", repository: "r", host: "",
                    path: "/tmp/w\($0)", state: "active", terminals: [], repositoryID: "r", workspace: nil)
            }, terminals: shell)
        loose.onNew = {}
        let defaults = UserDefaults(suiteName: "split-\(UUID().uuidString)")!
        for id in closed { defaults.set(true, forKey: TaskBoardView.closedKey(id, store: store)) }
        return TaskBoardView(
            store: store, client: store.client, agents: .none, onGoTo: { _ in }, defaults: defaults,
            worktrees: { _ in loose },
            orchestrator: NavigatorOrchestrator(state: .working, agent: "claude", status: .working, nowDoing: "Reading"),
            split: kept)
    }

    /// A board drawn in an unshown window `height` tall.
    @MainActor final class Drawn {
        let host: NSHostingView<NavigatorRhythmTests.Probe<AnyView>>
        let window: NSWindow
        let box = NavigatorRhythmTests.Box()

        init(_ view: some View, width: CGFloat = WorkspaceColumns.navigatorDefault, height: CGFloat, dark: Bool = false) {
            host = NSHostingView(
                rootView: NavigatorRhythmTests.Probe(
                    box: box, content: AnyView(view.frame(width: width, height: height))))
            window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: width, height: height), styleMask: [.borderless],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            window.contentView = host
        }

        func settle() async {
            let box = box
            await NavigatorRhythmTests.settle(host) { "\(box.probes.map(\.1)) \(box.marks.map(\.1))" }
        }

        func probe(_ id: String) -> CGRect? { box.probes.first { $0.0 == id }?.1 }
        func boxes(_ row: String) -> [CGRect] {
            box.marks.filter { $0.0.row == row && $0.0.role == .box }.map(\.1)
        }

        func pixels() -> NSBitmapImageRep {
            let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds)!
            host.cacheDisplay(in: host.bounds, to: rep)
            return rep
        }

        /// The scroll views drawn, top to bottom.
        func scrollViews() -> [NSScrollView] {
            var found: [NSScrollView] = []
            func walk(_ view: NSView) {
                if let scroll = view as? NSScrollView { found.append(scroll) }
                view.subviews.forEach(walk)
            }
            walk(host)
            return found.sorted { $0.convert($0.bounds, to: host).minY < $1.convert($1.bounds, to: host).minY }
        }

        deinit { MainActor.assumeIsolated { window.close() } }
    }

    @Test("With 120 tasks, every terminal and worktree row is on screen, inside its pane")
    func terminalsStayInSight() async throws {
        let drawn = Drawn(await Self.board(tasks: 120), height: 800)
        await drawn.settle()
        let tasks = try #require(drawn.probe("navigator-pane-tasks"))
        #expect(tasks.height >= NavigatorSplit.fillMinimum, "Tasks squeezed to \(tasks)")
        for (row, pane) in [("projectTerminal", "terminals"), ("boardWorktree", "worktrees")] {
            let viewport = try #require(drawn.probe("navigator-pane-\(pane)"), "no \(pane) pane")
            #expect(viewport.maxY <= 800 && viewport.minY > tasks.maxY, "\(pane) at \(viewport), Tasks \(tasks)")
            let rows = drawn.boxes(row)
            #expect(rows.count == 2, "\(row): \(rows)")
            for frame in rows {
                #expect(viewport.insetBy(dx: -NavigatorGrid.edge, dy: -0.5).contains(frame), "\(row) at \(frame), outside \(viewport)")
            }
        }
    }

    /// Scrolled by its own scroll view, as a wheel would: Tasks' pixels move
    /// and nothing under it does.
    @Test("Each pane scrolls on its own")
    func eachPaneScrollsOnItsOwn() async throws {
        let drawn = Drawn(await Self.board(tasks: 120, terminals: 30), height: 800)
        await drawn.settle()
        let tasks = try #require(drawn.probe("navigator-pane-tasks"))
        let terminals = try #require(drawn.probe("navigator-pane-terminals"))
        let scrolls = drawn.scrollViews()
        #expect(scrolls.count == 3, "\(scrolls.count) scroll views")
        // One whose rows are taller than it, each: both scroll.
        for scroll in scrolls.prefix(2) {
            #expect((scroll.documentView?.frame.height ?? 0) > scroll.contentView.bounds.height + 40)
        }
        guard scrolls.count >= 2 else { return }

        func rows(_ rep: NSBitmapImageRep, _ rect: CGRect) -> Data {
            let scale = CGFloat(rep.pixelsHigh) / drawn.host.bounds.height
            let top = Int(rect.minY * scale), bottom = Int(rect.maxY * scale)
            let bytes = rep.bytesPerRow
            return Data(bytes: rep.bitmapData! + top * bytes, count: (bottom - top) * bytes)
        }
        for (moved, still) in [(0, (tasks, terminals)), (1, (terminals, tasks))] {
            let (movedRect, stillRect) = still
            let before = drawn.pixels()
            let clip = scrolls[moved].contentView
            clip.scroll(to: NSPoint(x: 0, y: clip.bounds.origin.y + 120))
            scrolls[moved].reflectScrolledClipView(clip)
            await drawn.settle()
            let after = drawn.pixels()
            #expect(rows(before, movedRect) != rows(after, movedRect), "pane \(moved) didn't scroll")
            #expect(rows(before, stillRect) == rows(after, stillRect), "pane \(moved) moved another")
        }
    }

    final class Kept { var value = "" }

    /// The heights the window keeps are drawn: a board drawn again with
    /// what was kept draws the pane at the height a drag chose.
    @Test("What a drag chose is kept, and drawn again")
    func chosenHeightsPersist() async throws {
        let kept = Kept()
        let binding = Binding(get: { kept.value }, set: { kept.value = $0 })
        // What the rule's drag writes, 90 pt up from the height it had.
        let first = Drawn(await Self.board(tasks: 120, terminals: 30, kept: binding), height: 800)
        await first.settle()
        let was = try #require(first.probe("navigator-pane-terminals")).height
        let panes = [
            Pane(id: "tasks", fills: true, content: 5000), Pane(id: "terminals", content: 2000),
            Pane(id: "worktrees", content: 100),
        ]
        let drag = try #require(NavigatorSplit.drag(rule: 1, panes: panes, heights: ["terminals": was], by: -90))
        binding.wrappedValue = NavigatorSplit.encode([drag.id: drag.height])
        // The window drawn again from what it kept, as a restored window is.
        let again = Drawn(await Self.board(tasks: 120, terminals: 30, kept: binding), height: 800)
        await again.settle()
        let redrawn = try #require(again.probe("navigator-pane-terminals")).height
        #expect(abs(redrawn - (was + 90)) < 1, "kept \(kept.value): \(redrawn), not \(was + 90)")
        #expect(kept.value.hasPrefix("terminals="), "kept \(kept.value)")
    }

    // MARK: - Captures

    /// Written when asked to (`FARCOOLER_SPLIT_OUT`): many tasks, few, and
    /// sections closed, light and dark.
    @Test("Write the split navigator's captures")
    func writeCaptures() async throws {
        guard let out = ProcessInfo.processInfo.environment["FARCOOLER_SPLIT_OUT"] else { return }
        let directory = URL(fileURLWithPath: out)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let cases: [(String, Int, Int, [String])] = [
            ("many-tasks", 120, 6, []), ("few-tasks", 4, 3, []), ("collapsed", 120, 6, ["tasks"]),
            ("collapsed-terminals", 120, 6, ["terminals", "worktrees"]),
        ]
        for (name, tasks, terminals, closed) in cases {
            for dark in [false, true] {
                let drawn = Drawn(
                    await Self.board(tasks: tasks, terminals: terminals, closed: closed).background(WorkspaceStyle.canvas),
                    height: 800, dark: dark)
                await drawn.settle()
                let png = drawn.pixels().representation(using: .png, properties: [:])
                try png?.write(to: directory.appendingPathComponent("split-\(name)-\(dark ? "dark" : "light").png"))
            }
        }
    }
}
