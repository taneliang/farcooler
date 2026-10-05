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

    @Test("Many terminals share the room evenly with Tasks while both want more")
    func manyTerminalsShareEvenly() {
        let panes = [Pane(id: "tasks", fills: true, content: 5000), Pane(id: "terminals", content: 2000)]
        let heights = NavigatorSplit.viewports(panes, room: 900)
        #expect(heights["terminals"] == 450 && heights["tasks"] == 450, "\(heights)")
    }

    /// ov-292, the owner: Terminals scrolled with 3 rows while Worktrees was
    /// closed and room was free.
    @Test("Three terminals with Worktrees closed take their rows' height and don't scroll, in any window that holds them")
    func fewTerminalsDoNotScroll() {
        let panes = [
            Pane(id: "tasks", fills: true, content: 5000), Pane(id: "terminals", content: 96),
            Pane(id: "worktrees", expanded: false, content: 300),
        ]
        for room: CGFloat in [300, 500, 900, 2000] {
            let heights = NavigatorSplit.viewports(panes, room: room)
            #expect(heights["terminals"] == 96, "\(room): \(heights)")
            #expect(heights["worktrees"] == nil)
            #expect(abs((heights["tasks"] ?? 0) + 96 - room) < 0.01, "Tasks takes the rest at \(room): \(heights)")
        }
    }

    @Test("Many rows in every section: each is held to an even share, each scrolls, and nothing is over the room")
    func everySectionScrolls() {
        let panes = [
            Pane(id: "tasks", fills: true, content: 5000), Pane(id: "terminals", content: 2000),
            Pane(id: "worktrees", content: 2000),
        ]
        let heights = NavigatorSplit.viewports(panes, room: 600)
        for pane in panes {
            #expect(abs((heights[pane.id] ?? 0) - 200) < 0.01 && (heights[pane.id] ?? 0) < pane.content, "\(heights)")
        }
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

    @Test("A chosen height holds, short of Tasks' floor")
    func aChosenHeightHolds() {
        let panes = [Pane(id: "tasks", fills: true, content: 5000), Pane(id: "terminals", content: 2000)]
        let chosen = NavigatorSplit.viewports(panes, room: 900, chosen: ["terminals": 500])
        #expect(chosen["terminals"] == 500 && chosen["tasks"] == 400, "\(chosen)")
        let greedy = NavigatorSplit.viewports(panes, room: 900, chosen: ["terminals": 880])
        #expect(greedy["tasks"] == NavigatorSplit.fillMinimum, "\(greedy)")
        #expect(greedy["terminals"] == 900 - NavigatorSplit.fillMinimum, "\(greedy)")
    }

    /// Too short for every floor: the panes squeeze in proportion, none
    /// below nothing and none past the room, so the navigator never scrolls
    /// as one (ov-292) and every header stays on screen.
    @Test("A window too short for every floor squeezes the panes, never past the room")
    func aShortWindow() {
        let panes = [
            Pane(id: "tasks", fills: true, content: 5000), Pane(id: "terminals", content: 2000),
            Pane(id: "worktrees", content: 2000), Pane(id: "few", content: 30),
        ]
        let floors = NavigatorSplit.fillMinimum + 2 * NavigatorSplit.minimum + 30
        for room: CGFloat in [200, 50, 0, -40] {
            let heights = NavigatorSplit.viewports(panes, room: room)
            let total = heights.values.reduce(0, +)
            #expect(total <= max(room, 0) + 0.01, "\(room): \(heights)")
            #expect(heights.values.allSatisfy { $0 >= 0 })
            if room > 0, room < floors {
                #expect(abs(total - room) < 0.01, "the squeezed panes fill the room: \(heights)")
                #expect(heights["tasks"]! > heights["few"]!, "in proportion to their floors: \(heights)")
            }
        }
        let roomy = NavigatorSplit.viewports(panes, room: floors)
        #expect(roomy["tasks"] == NavigatorSplit.fillMinimum && roomy["few"] == 30, "\(roomy)")
    }

    static let three = [
        Pane(id: "tasks", fills: true, content: 5000), Pane(id: "terminals", content: 400),
        Pane(id: "worktrees", content: 300),
    ]

    /// What a drag keeps is what's then drawn, for every distance, in
    /// either direction, past either limit.
    @Test("A drag keeps exactly what it draws, and the rule follows the pointer to its limit")
    func aDragKeepsWhatItDraws() {
        let room: CGFloat = 700
        let start = NavigatorSplit.viewports(Self.three, room: room)
        for rule in [1, 2] {
            let limits = NavigatorSplit.resize(rule: rule, panes: Self.three, heights: start, room: room)
            #expect(limits.up > 0 && limits.down > 0, "rule \(rule): \(limits)")
            for dy: CGFloat in [-5000, -limits.up, -37, 0, 23, limits.down, 5000] {
                let chosen = NavigatorSplit.dragged(
                    rule: rule, panes: Self.three, heights: start, room: room, by: dy, chosen: [:])
                let drawn = NavigatorSplit.viewports(Self.three, room: room, chosen: chosen)
                for (id, height) in chosen {
                    #expect(abs(drawn[id]! - height) < 0.001, "rule \(rule) by \(dy): \(id) kept \(height), drew \(drawn[id]!)")
                }
                // The rule moved as far as the pointer, held at its limits.
                let moved = min(max(dy, -limits.up), limits.down)
                let below = Self.three[rule].id
                #expect(abs((start[below]! - drawn[below]!) - moved) < 0.001, "rule \(rule) by \(dy)")
            }
        }
    }

    /// The rule between Terminals and Worktrees trades between those two,
    /// as Xcode's does: Tasks, and the rule over Terminals, stay put.
    @Test("The lower rule trades between the two panes beside it")
    func theLowerRuleTradesWithItsNeighbors() {
        let room: CGFloat = 700
        let start = NavigatorSplit.viewports(Self.three, room: room)
        for dy: CGFloat in [-60, 40] {
            let chosen = NavigatorSplit.dragged(
                rule: 2, panes: Self.three, heights: start, room: room, by: dy, chosen: [:])
            let drawn = NavigatorSplit.viewports(Self.three, room: room, chosen: chosen)
            #expect(abs(drawn["tasks"]! - start["tasks"]!) < 0.001, "\(dy): \(start) → \(drawn)")
            #expect(abs(drawn["terminals"]! + drawn["worktrees"]! - start["terminals"]! - start["worktrees"]!) < 0.001)
            #expect(drawn["worktrees"]! == start["worktrees"]! - dy, "\(dy): \(drawn)")
        }
    }

    @Test("A rule over a closed pane resizes the pane over it; a rule with nothing to resize doesn't move")
    func rulesBesideClosedPanes() {
        var panes = Self.three
        panes[2].expanded = false
        let heights: [String: CGFloat] = ["tasks": 400, "terminals": 200]
        let lower = NavigatorSplit.resize(rule: 2, panes: panes, heights: heights, room: 700)
        #expect(lower.above == "terminals" && lower.below == nil && lower.canMove, "\(lower)")
        panes[1].expanded = false
        let inert = NavigatorSplit.resize(rule: 2, panes: panes, heights: ["tasks": 700], room: 700)
        #expect(!inert.canMove && inert.name == nil, "\(inert)")
    }

    @Test("What the window keeps reads back as written")
    func theCodecRoundTrips() {
        let chosen: [String: CGFloat] = ["terminals": 120, "worktrees": 80.4]
        let kept = NavigatorSplit.encode(chosen)
        #expect(kept == "terminals=120,worktrees=80")
        #expect(NavigatorSplit.decode(kept) == ["terminals": 120, "worktrees": 80])
        #expect(NavigatorSplit.decode("junk,terminals=x,worktrees=-3,tasks=10") == ["tasks": 10])
    }

    /// Restored with the window, so read as input: nothing it says may trap
    /// (`Int(inf)` does) or ask for a pane past any screen.
    @Test("A kept height that isn't a sane number is dropped or held")
    func keptHeightsAreDistrusted() {
        let kept = "a=inf,b=nan,c=-inf,d=1e400,e=1e300,f=12000,g=80"
        let chosen = NavigatorSplit.decode(kept)
        #expect(chosen == ["e": NavigatorSplit.tallest, "f": NavigatorSplit.tallest, "g": 80], "\(chosen)")
        // Whatever reaches it, encode writes only finite, held heights.
        let written = NavigatorSplit.encode(["a": .infinity, "b": .nan, "c": 1e300, "d": -5, "e": 40])
        #expect(written == "c=10000,d=0,e=40", "\(written)")
    }

    // MARK: - Drawn

    /// A board of `count` tasks to do, and `terminals` terminals.
    static func board(
        tasks count: Int, terminals: Int = 2, kept: Binding<String>? = nil, closed: [String] = []
    ) async -> TaskBoardView {
        view(await store(tasks: count), terminals: terminals, kept: kept, closed: closed)
    }

    /// A board's store: `count` tasks, every one read.
    static func store(tasks count: Int) async -> TaskBoardStore {
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
        return store
    }

    /// `store`'s navigator, with `terminals` terminals and two worktrees,
    /// `current` selected.
    static func view(
        _ store: TaskBoardStore, terminals: Int = 2, kept: Binding<String>? = nil, closed: [String] = [],
        defaults: UserDefaults = UserDefaults(suiteName: "split-\(UUID().uuidString)")!,
        current: NavigatorItem? = nil, onStep: ((NavigatorItem) -> Void)? = nil
    ) -> TaskBoardView {
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
        for id in closed { defaults.set(true, forKey: TaskBoardView.closedKey(id, store: store)) }
        return TaskBoardView(
            store: store, client: store.client, agents: .none, onGoTo: { _ in }, defaults: defaults,
            hasKeyboard: current != nil, worktrees: { _ in loose },
            orchestrator: NavigatorOrchestrator(state: .working, agent: "claude", status: .working, nowDoing: "Reading"),
            current: current, onStep: onStep, split: kept)
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

        /// The scroll views that hold a pane's: the split's own.
        func outerScrollViews() -> [NSScrollView] {
            var found: [NSScrollView] = []
            func holds(_ view: NSView) -> Bool { view.subviews.contains { $0 is NSScrollView || holds($0) } }
            func walk(_ view: NSView) {
                if let scroll = view as? NSScrollView, holds(scroll) { found.append(scroll) }
                view.subviews.forEach(walk)
            }
            walk(host)
            return found
        }

        /// Draw `view` in its place, as the window does when what it hands
        /// the navigator changes.
        func show(_ view: some View) {
            let size = window.frame.size
            host.rootView = NavigatorRhythmTests.Probe(
                box: box, content: AnyView(view.frame(width: size.width, height: size.height)))
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

        /// The panes' scroll views, top to bottom: not the one the whole
        /// split scrolls in, in a window too short for it, which holds them.
        func scrollViews() -> [NSScrollView] {
            var found: [NSScrollView] = []
            func walk(_ view: NSView) {
                if let scroll = view as? NSScrollView, !holdsAScrollView(scroll) { found.append(scroll) }
                view.subviews.forEach(walk)
            }
            func holdsAScrollView(_ view: NSView) -> Bool {
                view.subviews.contains { $0 is NSScrollView || holdsAScrollView($0) }
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

    /// The dividers under the orchestrator and between the panes, top to
    /// bottom.
    private func dividers(_ drawn: Drawn) -> [CGRect] {
        drawn.box.probes.filter { $0.0 == "navigator-divider" }.map(\.1).sorted { $0.minY < $1.minY }
    }

    /// ov-258, the owner, 4 October: "the sidebar horizontal dividers seem to
    /// have margin around them ... the scroll content should probably bump up
    /// against the lines." The scroll view of each pane over a rule ends where
    /// the rule's line starts, so what scrolls runs up to it, and the room the
    /// rhythm gives it is an inset at the foot of the content.
    @Test("Each pane's scroll view meets the rule under it, with its rhythm as an inset inside")
    func panesMeetTheirRules() async throws {
        let drawn = Drawn(await Self.board(tasks: 120, terminals: 30), height: 800)
        await drawn.settle()
        let rules = dividers(drawn)
        #expect(rules.count == 3, "\(rules.count) dividers: the orchestrator's and two rules")
        guard rules.count == 3 else { return }
        for (pane, rule) in [("tasks", rules[1]), ("terminals", rules[2])] {
            let viewport = try #require(drawn.probe("navigator-pane-\(pane)"), "no \(pane) pane")
            #expect(abs(viewport.maxY - rule.minY) <= 0.5, "\(pane) ends at \(viewport.maxY), its rule starts at \(rule.minY)")
        }
        #expect(NavigatorSplit.ruleSlot == WorkspaceColumns.divider, "a rule's slot is its line")
        #expect(NavigatorSplit.paneInset == NavigatorRhythm.rule && NavigatorSplit.headerInset == NavigatorRhythm.rule)
    }

    /// At rest, the last row of a pane that shows all its rows is
    /// `NavigatorRhythm.rule` (and its own air) from the rule: the inset,
    /// inside the scroll view. Scrolled, a row runs under the line's edge.
    @Test("A pane's last row rests a rhythm from its rule, and scrolled rows run up to it")
    func theInsetIsInsideTheScroll() async throws {
        let rest = Drawn(await Self.board(tasks: 3, terminals: 2), height: 800)
        await rest.settle()
        let rules = dividers(rest)
        let new = try #require(rest.box.marks.first { $0.0.row == "projectTerminalNew" && $0.0.role == .text }?.1)
        let pane = try #require(rest.probe("navigator-pane-terminals"))
        let under = try #require(rules.last)
        #expect(
            abs((under.minY - new.maxY) - (NavigatorRhythm.rule + NavigatorRhythm.air)) <= 1,
            "New Terminal ends \(under.minY - new.maxY) from its rule")
        #expect(abs(pane.maxY - under.minY) <= 0.5, "the pane \(pane) and its rule \(under)")
        // Scrolled: a pane whose rows are taller than it has one straddling
        // its foot, cut by the scroll view's edge at the rule.
        let long = Drawn(await Self.board(tasks: 120, terminals: 30), height: 800)
        await long.settle()
        let scrolls = long.scrollViews()
        guard scrolls.count >= 2 else { Issue.record("\(scrolls.count) scroll views"); return }
        let clip = scrolls[1].contentView
        clip.scroll(to: NSPoint(x: 0, y: clip.bounds.origin.y + 100))
        scrolls[1].reflectScrolledClipView(clip)
        await long.settle()
        let viewport = try #require(long.probe("navigator-pane-terminals"))
        let rule = try #require(dividers(long).last)
        let cut = long.boxes("projectTerminal").filter { $0.minY < viewport.maxY && $0.maxY > viewport.maxY }
        #expect(!cut.isEmpty, "no row runs up to the rule at \(rule.minY)")
    }

    final class Kept { var value = "" }

    /// What the window does with the heights: keeps them in state it
    /// observes (`@State` in `ContentView`, kept in the window's record), so a change draws again.
    /// `sink` sees each value kept.
    struct Keeping<V: View>: View {
        @State var kept: String
        let sink: Kept
        let make: (Binding<String>) -> V
        init(_ sink: Kept, make: @escaping (Binding<String>) -> V) {
            _kept = State(initialValue: sink.value)
            self.sink = sink
            self.make = make
        }
        var body: some View {
            make($kept).onChange(of: kept, initial: true) { _, now in sink.value = now }
        }
    }

    /// The rule dragged: its own drag handlers, called as its
    /// `DragGesture` calls them, ten points at a time to 90 pt up, then
    /// released. (An unshown window takes no real mouse, and a test sends
    /// none.)
    @Test("Dragging a rule resizes its pane, and the window keeps what's drawn")
    func aDragIsKept() async throws {
        let kept = Kept()
        let store = await Self.store(tasks: 120)
        let drawn = Drawn(Keeping(kept) { Self.view(store, terminals: 30, kept: $0) }, height: 800)
        await drawn.settle()
        let was = try #require(drawn.probe("navigator-pane-terminals")).height
        for step in 1...9 {
            let rule = try #require(drawn.box.rules.first { $0.rule.index == 1 }).rule
            rule.onDrag(-CGFloat(step) * 10)
            await drawn.settle()
        }
        try #require(drawn.box.rules.first { $0.rule.index == 1 }).rule.onDragEnd()
        await drawn.settle()
        let now = try #require(drawn.probe("navigator-pane-terminals")).height
        #expect(abs(now - (was + 90)) < 1, "dragged 90 up: \(was) → \(now)")
        // What's kept is what's drawn, to the point.
        let chosen = try #require(NavigatorSplit.decode(kept.value)["terminals"], "kept \(kept.value)")
        #expect(abs(chosen - now) <= 0.5, "kept \(chosen), drew \(now)")
        // Dragged on past where Tasks keeps its four rows: the rule stops
        // there, and that's what's kept, not where the pointer went.
        let rule = try #require(drawn.box.rules.first { $0.rule.index == 1 }).rule
        rule.onDrag(-2000)
        await drawn.settle()
        rule.onDragEnd()
        let tasks = try #require(drawn.probe("navigator-pane-tasks")).height
        let held = try #require(drawn.probe("navigator-pane-terminals")).height
        #expect(abs(tasks - NavigatorSplit.fillMinimum) < 1, "Tasks \(tasks)")
        #expect(abs((NavigatorSplit.decode(kept.value)["terminals"] ?? 0) - held) <= 0.5, "kept \(kept.value), drew \(held)")
        // A window drawn again from what it kept, as a restored one is.
        let again = Drawn(Keeping(kept) { Self.view(store, terminals: 30, kept: $0) }, height: 800)
        await again.settle()
        let redrawn = try #require(again.probe("navigator-pane-terminals")).height
        #expect(abs(redrawn - held) < 1, "kept \(kept.value): \(redrawn), not \(held)")
    }

    /// The rule as VoiceOver reads it, named for the pane it resizes, its
    /// height its value, adjusted a row at a time; and ↑ and ↓ on it, with
    /// the keyboard, the same steps.
    @Test("A rule is adjustable by VoiceOver and by the keyboard")
    func rulesAreAdjustable() async throws {
        let store = await Self.store(tasks: 120)
        let drawn = Drawn(Keeping(Kept()) { Self.view(store, terminals: 30, kept: $0) }, height: 800)
        await drawn.settle()
        func rule() throws -> NavigatorSplitRule { try #require(drawn.box.rules.first { $0.rule.index == 1 }).rule }
        func height() throws -> CGFloat { try #require(drawn.probe("navigator-pane-terminals")).height }
        let was = try height()
        #expect(try rule().label == "Resize Terminals")
        // Points as the value says them, whole: within one of the drawn height
        // (heights are fractional shares now, and the value is rounded).
        let value = try rule().value
        let said = Int(value.split(separator: " ").first ?? "") ?? -1
        #expect(abs(CGFloat(said) - was) <= 1, "\(value) vs \(was)")
        #expect(try rule().canMove)
        try rule().adjust(.increment)
        await drawn.settle()
        let up = try height()
        #expect(abs(up - (was + NavigatorSplit.row)) < 1, "VoiceOver up: \(was) → \(up)")
        try rule().adjust(.decrement)
        await drawn.settle()
        #expect(abs(try height() - was) < 1)
        #expect(try rule().press(.upArrow) == .handled)
        await drawn.settle()
        let keyed = try height()
        #expect(abs(keyed - (was + NavigatorSplit.row)) < 1, "↑: \(was) → \(keyed)")
        #expect(try rule().press(.downArrow) == .handled)
        await drawn.settle()
        #expect(abs(try height() - was) < 1)
        #expect(try rule().press(.leftArrow) == .ignored)
        // The lower rule names the pane under it.
        #expect(drawn.box.rules.first { $0.rule.index == 2 }?.rule.label == "Resize Worktrees")
    }

    final class Steps { var items: [NavigatorItem] = [] }

    /// A rule had the keyboard, then a row was selected without the rule
    /// hearing it lost focus (a click on a task): ↓ steps through the rows
    /// again, rather than resizing.
    @Test("Selecting a row takes the arrows back from a rule")
    func selectingARowTakesTheArrowsBack() async throws {
        let store = await Self.store(tasks: 120)
        let defaults = UserDefaults(suiteName: "split-\(UUID().uuidString)")!
        let tasks = BoardKeys.rows(
            store.board, collapsed: BoardForm.collapsed(host: store.hostKey, workspace: store.workspace.id, from: defaults),
            reads: store.reads, now: Date())
        let steps = Steps()
        func view(_ current: NavigatorItem) -> TaskBoardView {
            Self.view(store, terminals: 3, defaults: defaults, current: current, onStep: { steps.items.append($0) })
        }
        let drawn = Drawn(view(.task(tasks[0])), height: 800)
        await drawn.settle()
        let rule = try #require(drawn.box.rules.first { $0.rule.index == 1 }).rule
        rule.onFocus(true)
        let arrows = try #require(drawn.box.arrows.first)
        #expect(arrows.arrow(1) == nil, "the rule has the keyboard; the list should stand aside")
        #expect(steps.items.isEmpty)
        // The window selects another task, as a click on its row does.
        drawn.show(view(.task(tasks[3])))
        await drawn.settle()
        let after = try #require(drawn.box.arrows.first)
        #expect(after.arrow(1) == .handled, "↓ after selecting a row didn't step")
        #expect(steps.items == [.task(tasks[4])], "stepped to \(steps.items)")
    }

    /// At the window's least height, 400 pt, and under it, every pane is
    /// drawn inside the window and none over another: the panes squeeze, and
    /// nothing around them scrolls (ov-292).
    @Test("A short window squeezes the panes inside it, none overlapping", arguments: [400, 300, 200] as [CGFloat])
    func aShortWindowIsDrawnWhole(height: CGFloat) async throws {
        let drawn = Drawn(await Self.board(tasks: 120, terminals: 30), height: height)
        await drawn.settle()
        var bottom: CGFloat = -.infinity
        for id in ["tasks", "terminals", "worktrees"] {
            let pane = try #require(drawn.probe("navigator-pane-\(id)"), "no \(id) pane")
            #expect(pane.minY >= bottom - 0.5, "\(height): \(id) at \(pane.minY) overlaps what's over it, to \(bottom)")
            #expect(pane.minY < height, "\(height): \(id)'s header is off the window")
            bottom = pane.maxY
        }
        #expect(bottom <= height + 0.5, "\(height): the panes reach \(bottom)")
        #expect(drawn.outerScrollViews().isEmpty, "something scrolls the sections as one")
    }

    /// ov-292: one level of scrolling. Each open section has its one scroll
    /// view, and nothing scrolls around them.
    @Test("No scroll view holds the sections, and each section has exactly one", arguments: [800, 300] as [CGFloat])
    func oneScrollViewPerSection(height: CGFloat) async throws {
        let drawn = Drawn(await Self.board(tasks: 120, terminals: 30), height: height)
        await drawn.settle()
        #expect(drawn.outerScrollViews().isEmpty, "a scroll view holds another")
        #expect(drawn.scrollViews().count == 3, "\(drawn.scrollViews().count) scroll views for 3 sections")
    }

    /// The owner's case in a real window: 3 terminals, Worktrees closed.
    @Test("In a window, three terminals with Worktrees closed show every row with nothing to scroll")
    func threeTerminalsDoNotScroll() async throws {
        let drawn = Drawn(await Self.board(tasks: 120, terminals: 3, closed: ["worktrees"]), height: 700)
        await drawn.settle()
        let pane = try #require(drawn.probe("navigator-pane-terminals"))
        let scrolls = drawn.scrollViews()
        #expect(scrolls.count == 2, "\(scrolls.count) scroll views: Tasks and Terminals")
        let scroll = try #require(scrolls.last)
        let content = scroll.documentView?.frame.height ?? 0
        #expect(abs(pane.height - content) <= 0.5, "the pane is \(pane.height) tall for \(content) of rows")
        #expect(content <= scroll.contentView.bounds.height + 0.5, "Terminals scrolls")
    }

    /// ↓ from the last task: the selection crosses into Terminals' first
    /// row, and Terminals' pane scrolls it into sight; down to its last,
    /// the same, Tasks' pane left where it was.
    @Test("Arrowing from Tasks into Terminals crosses the panes and keeps the row in sight")
    func arrowsCrossThePanes() async throws {
        let store = await Self.store(tasks: 120)
        let defaults = UserDefaults(suiteName: "split-\(UUID().uuidString)")!
        let terminals = (0..<30).map { "term\($0)" }
        let tasks = BoardKeys.rows(
            store.board, collapsed: BoardForm.collapsed(host: store.hostKey, workspace: store.workspace.id, from: defaults),
            reads: store.reads, keeping: nil, showingMore: [], filtering: false, now: Date())
        let items = Navigator.items(orchestrator: true, tasks: tasks, terminals: terminals, worktrees: ["w0", "w1"])
        let last = try #require(tasks.last)
        let drawn = Drawn(Self.view(store, terminals: 30, defaults: defaults, current: .task(last)), height: 800)
        await drawn.settle()
        // Selected where it was drawn, then ↓ onto the next row.
        var current: NavigatorItem = .task(last)
        func step(_ by: Int) async {
            current = Navigator.step(from: current, by: by, in: items)!
            drawn.show(Self.view(store, terminals: 30, defaults: defaults, current: current))
            await drawn.settle()
        }
        await step(1)
        #expect(current == .terminal("term0"), "↓ from the last task went to \(current)")
        for _ in 0..<29 { await step(1) }
        #expect(current == .terminal("term29"))
        let pane = try #require(drawn.probe("navigator-pane-terminals"))
        let row = try #require(drawn.box.probes.first { $0.0 == "navigator-terminal-term29" }?.1, "no row probe")
        #expect(pane.insetBy(dx: 0, dy: -0.5).contains(CGPoint(x: pane.midX, y: row.midY)), "row at \(row), pane \(pane)")
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
