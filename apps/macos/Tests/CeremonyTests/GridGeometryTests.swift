import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The board column on one grid (ov-83): every chevron, icon and text start
/// of every row type lands on a `ColumnGrid` column, and on the one the
/// owner's ruling names. (The old Fleet sidebar was on it too, until it went,
/// ov-178.)
///
/// Measured, not computed. The board is the real `TaskBoardView`, over a
/// board read through a stubbed CLI, each row reporting where its marks
/// landed through `gridMark(_:_:)`. ov-78's `SidebarColumnTests` read the
/// constants the rows were meant to lay out from, and passed while a
/// repository's name sat 12 pt off its column, pushed there by a chevron
/// cell drawn invisibly.
@MainActor
struct GridGeometryTests {
    struct Mark: CustomStringConvertible {
        let row: String
        let role: GridRole
        let x: CGFloat
        let width: CGFloat
        var midX: CGFloat { x + width / 2 }
        var description: String { "\(row).\(role.rawValue) at \(x), \(width) wide" }
    }

    final class Box {
        var marks: [Mark] = []
        func record(_ marks: [GridMark], _ proxy: GeometryProxy) {
            self.marks = marks.map { 
                let bounds = proxy[$0.bounds]
                return Mark(row: $0.row, role: $0.role, x: bounds.minX, width: bounds.width)
            }
        }
    }

    /// `content` with its marks switched on, read in its own coordinates.
    struct Probe<Content: View>: View {
        let box: Box
        let content: Content
        var body: some View {
            content
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(GridMarksKey.self) { marks in
                    GeometryReader { proxy in
                        let _ = box.record(marks, proxy)
                        Color.clear
                    }
                }
        }
    }

    /// Draw `view` `width` wide, top-left, in an unshown window, and read
    /// back its marks.
    private func marks<V: View>(_ view: V, width: CGFloat, height: CGFloat = 900) async -> [Mark] {
        let box = Box()
        let root = Probe(box: box, content: view.frame(width: width, height: height, alignment: .topLeading))
        let host = NSHostingView(rootView: root)
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: width, height: height), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        defer { window.close() }
        for _ in 0..<10 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        return box.marks
    }

    /// Every mark on a column, each named one on its own, and every named
    /// one drawn.
    private func check(_ marks: [Mark], expect: [String: CGFloat]) {
        // A box starts past the grid and a glyph is placed by its center
        // (`checkTheLines`); everything else is on a column.
        for mark in marks where mark.role == .text {
            #expect(ColumnGrid.isColumn(mark.x), "\(mark) is between columns")
        }
        for (name, column) in expect.sorted(by: { $0.key < $1.key }) {
            let found = marks.filter { "\($0.row).\($0.role.rawValue)" == name }
            #expect(!found.isEmpty, "no \(name) was drawn")
            for mark in found {
                #expect(abs(mark.x - column) < 0.5, "\(mark), not at \(column)")
            }
        }
    }

    // MARK: - The board column

    /// A store read through a stubbed CLI: three tasks, all new in the last
    /// day, so the summary has groups to draw.
    private static func store() async -> TaskBoardStore {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let now = Int64(Date().timeIntervalSince1970 * 1000) - 60_000
        client.commandRunnerForTesting = { args in
            guard args.starts(with: ["task", "list"]) else { return (Data(), nil) }
            let tasks = [
                ("t1", "ov-81", "General polish", "done"),
                ("t2", "ov-1234", "Polish the sidebar", "needs_decision"),
                ("t3", "ov-9", "Coordinator", "todo"),
            ].map { id, key, title, status in
                #"{"id":"\#(id)","key":"\#(key)","title":"\#(title)","status":"\#(status)","status_since":\#(now),"created_at":\#(now),"updated_at":\#(now)}"#
            }
            return (Data(#"{"tasks":[\#(tasks.joined(separator: ","))]}"#.utf8), nil)
        }
        let store = TaskBoardStore(client: client, workspace: .implicit(repository: "r"))
        await store.readIfNeverRead()
        return store
    }

    /// The orchestrator's row, running, or with none and its Start menu.
    private static func orchestrator(running: Bool) -> NavigatorOrchestrator {
        running
            ? NavigatorOrchestrator(state: .working, agent: "claude", status: .working, nowDoing: "Reading the board")
            : NavigatorOrchestrator(state: .none, offers: [.start(.claude)])
    }

    /// ov-230: every box reaches `outset` past the edge, every chevron
    /// is centered in the glyph column, every word at the text column, and every glyph in
    /// a box is centered on the carets' x.
    private func checkTheLines(_ found: [Mark]) {
        let boxes = Set(found.filter { $0.role == .box }.map(\.x))
        let chevrons = Set(found.filter { $0.role == .chevron }.map(\.x))
        let words = Set(found.filter { $0.role == .text && $0.row != "summary.title" }.map(\.x))
        #expect(NavigatorGrid.outset == NavigatorGrid.edge / 2, "an outset of half the margin")
        #expect(boxes == [NavigatorGrid.edge - NavigatorGrid.outset], "boxes at \(boxes.sorted())")
        // The caret is marked on its glyph, which is centered in the glyph
        // column, so its left edge is not the margin; its center is
        // checked below.
        #expect(chevrons.allSatisfy { $0 > NavigatorGrid.edge }, "chevrons at \(chevrons.sorted())")
        #expect(words == [NavigatorGrid.text], "text starts at \(words.sorted())")
        let carets = Set(found.filter { $0.role == .chevron }.map(\.midX))
        #expect(carets == [NavigatorGrid.glyphCenter], "caret centers at \(carets.sorted())")
        // Every glyph is measured where it draws, and every one is there.
        let drawn = Set(found.filter { $0.role == .icon }.map(\.row))
        #expect(!carets.isEmpty, "no caret was drawn")
        #expect(drawn.isSuperset(of: ["filter", "orchestrator"]), "glyphs drawn: \(drawn.sorted())")
        for icon in found where icon.role == .icon {
            #expect(abs(icon.midX - NavigatorGrid.glyphCenter) < 0.5, "\(icon) isn't centered on the carets")
            for caret in found where caret.role == .chevron {
                #expect(abs(icon.midX - caret.midX) < 0.5, "\(icon) vs \(caret)")
            }
        }
    }

    /// The Terminals and Worktrees sections follow the same lines (ov-230):
    /// each row's box past the edge, its glyph over the carets, its name at B.
    @Test("Terminals and worktrees rows sit on the same lines", .disabled("ov-235: glyph bounds differ on CI's macOS runner"))
    func terminalsAndWorktreesAreOnTheGrid() async {
        let store = await Self.store()
        var terminals = ProjectTerminals(
            terminals: [Terminal(id: "t1", short: "t1", title: "proxy", preset: "zsh", state: "running", epoch: 0)],
            selected: "t1")
        terminals.onNew = {}
        let loose = Worktree(
            id: "w1", short: "w1", task: "ov-9", branch: "ov-9", repository: "r", host: "", path: "/tmp/w1",
            state: "active", terminals: [], repositoryID: "r", workspace: nil)
        var worktrees = BoardWorktrees(shown: [loose], selected: "w1", terminals: terminals)
        worktrees.onNew = {}
        let board = TaskBoardView(
            store: store, client: store.client, agents: .none, onGoTo: { _ in },
            defaults: UserDefaults(suiteName: "grid-\(UUID().uuidString)")!,
            worktrees: { _ in worktrees }, orchestrator: Self.orchestrator(running: true))
        let found = await marks(board, width: WorkspaceColumns.navigatorDefault, height: 1400)
        check(
            found,
            expect: [
                "projectTerminal.box": NavigatorGrid.boxEdge,
                "projectTerminal.text": NavigatorGrid.text, 
                "projectTerminalNew.text": NavigatorGrid.text, "boardWorktree.box": NavigatorGrid.boxEdge, "boardWorktree.text": NavigatorGrid.text,
            ])
        checkTheLines(found)
    }

    @Test(
        "Every board row's chevron and text is on the board column's grid",
        arguments: [(false, true), (true, false)])
    func theBoardIsOnTheGrid(collapsedSummary: Bool, orchestratorRunning: Bool) async {
        let store = await Self.store()
        #expect(store.board.rows.count == 3)
        let defaults = UserDefaults(suiteName: "grid-\(UUID().uuidString)")!
        defaults.set(collapsedSummary, forKey: "board.summary.collapsed.\(store.hostKey).\(store.workspace.id)")
        let board = TaskBoardView(
            store: store, client: store.client, agents: .none, onGoTo: { _ in }, defaults: defaults,
            orchestrator: Self.orchestrator(running: orchestratorRunning))
        let found = await marks(board, width: WorkspaceColumns.navigatorDefault)
        var expect: [String: CGFloat] = [
            // ov-177 round 2: the words of the filter and the orchestrator
            // on the text column every row's words start on. ov-230: their
            // boxes reach half a margin past the grid's edge, and their
            // glyphs sit in the glyph column, checked below against the
            // carets.
            "filter.box": NavigatorGrid.boxEdge,
            "filter.text": NavigatorGrid.text,
            "orchestrator.box": NavigatorGrid.boxEdge,
            "orchestrator.text": NavigatorGrid.text,
            "summary.text": ColumnGrid.b,
            "tasks.text": ColumnGrid.b,
            "status.text": ColumnGrid.b,
            "card.text": ColumnGrid.b,
        ]
        if !collapsedSummary {
            expect["summary.group.text"] = ColumnGrid.b
            expect["summary.key.text"] = ColumnGrid.b
        }
        check(found, expect: expect)
        checkTheLines(found)
        // The items' titles in one column after the widest key, "ov-1234":
        // past the key, and the same x for every item.
        let titles = Set(found.filter { $0.row == "summary.title" }.map(\.x))
        #expect(titles.count == (collapsedSummary ? 0 : 1), "titles at \(titles)")
        if let title = titles.first { #expect(title > ColumnGrid.c, "a title at \(title) overlaps its key") }
    }

    /// Closed, the strip is one row: its 24 pt line and 8 pt above and
    /// below, whatever it has to say.
    @Test("The collapsed strip is one row tall")
    func theCollapsedStripIsOneRowTall() async {
        let store = await Self.store()
        let defaults = UserDefaults(suiteName: "strip-\(UUID().uuidString)")!
        defaults.set(true, forKey: "board.summary.collapsed.\(store.hostKey).\(store.workspace.id)")
        let host = NSHostingController(rootView: BoardSummaryStrip(store: store, defaults: defaults))
        for width in [WorkspaceColumns.navigatorMinimum, WorkspaceColumns.navigatorDefault] {
            let height = host.sizeThatFits(in: CGSize(width: width, height: 400)).height
            #expect(height == ColumnGrid.rowHeight + 2 * ColumnGrid.rhythm, "\(height) tall at \(width)")
        }
    }

    /// Open, the strip has as much room under its last line as over its
    /// first, so it doesn't read as a scroll area cut off at the divider
    /// (owner, 2 Oct); closed, its one row is centered. Measured too: the
    /// open strip is its header row, its lines, and both insets.
    @Test("The strip's room above its text equals the room below")
    func theStripIsAFinishedBlock() async {
        for collapsed in [false, true] {
            let room = BoardSummaryStrip.visibleInsets(collapsed: collapsed)
            #expect(room.top == room.bottom, "collapsed \(collapsed): \(room)")
        }
        let store = await Self.store()
        let defaults = UserDefaults(suiteName: "strip-open-\(UUID().uuidString)")!
        let host = NSHostingController(rootView: BoardSummaryStrip(store: store, defaults: defaults))
        let height = host.sizeThatFits(in: CGSize(width: WorkspaceColumns.navigatorDefault, height: 800)).height
        // Everything else in it is on the 8 pt rhythm, so what's over is
        // the bottom inset's: drawn, not just declared.
        let bottom = BoardSummaryStrip.insets(collapsed: false).bottom
        #expect(
            height.truncatingRemainder(dividingBy: ColumnGrid.rhythm)
                == bottom.truncatingRemainder(dividingBy: ColumnGrid.rhythm), "\(height) tall")
    }

    // MARK: - No stray offsets

    /// The row files, and how much of each holds rows: TaskBoard.swift's
    /// task detail, from `struct TaskCard` on, is another lane's.
    private static let rowFiles: [(name: String, until: String?)] = [
        ("SidebarViews.swift", nil), ("SidebarLayout.swift", nil),
        ("BoardSummaryStrip.swift", nil),
        ("TaskBoard.swift", "struct TaskCard: View"), ("TaskListSection.swift", nil), ("Navigator.swift", nil), ("BoardWorktreesSection.swift", nil),
        ("ProjectTerminalsSection.swift", nil),
    ]

    /// Every numeric horizontal padding or x offset in `source`, with its
    /// line number, that no `// grid-exempt:` comment explains.
    static func strays(in source: String) -> [(line: Int, text: String)] {
        let pattern = try! Regex(
            #"\.padding\((\.(leading|trailing|horizontal),\s*)?-?[0-9]|\.offset\(x:\s*-?[0-9]"#)
        return source.split(separator: "\n", omittingEmptySubsequences: false).enumerated()
            .filter { $0.element.contains(pattern) && !$0.element.contains("grid-exempt:") }
            .map { ($0.offset + 1, $0.element.trimmingCharacters(in: .whitespaces)) }
    }

    /// No row sets a horizontal inset in a number of its own: every one is a
    /// grid constant, or says on its line why it isn't a column.
    @Test("The row files hold no bare horizontal paddings or offsets")
    func noStrayOffsets() throws {
        let sources = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/FarCooler")
        for file in Self.rowFiles {
            var text = try String(contentsOf: sources.appendingPathComponent(file.name), encoding: .utf8)
            if let until = file.until, let end = text.range(of: until) {
                text = String(text[..<end.lowerBound])
            }
            for stray in Self.strays(in: text) {
                Issue.record("\(file.name):\(stray.line) sets a bare inset: \(stray.text)")
            }
        }
    }

    /// The scan finds what it is for, and lets an explained one through.
    @Test func theScanCatchesABareInset() {
        #expect(Self.strays(in: "x\n    .padding(.leading, 5)\n").map(\.line) == [2])
        #expect(Self.strays(in: ".padding(12)").count == 1)
        #expect(Self.strays(in: ".offset(x: -3)").count == 1)
        #expect(Self.strays(in: ".padding(.leading, SidebarGrid.gap)").isEmpty)
        #expect(Self.strays(in: ".padding(.vertical, 4)").isEmpty)
        #expect(Self.strays(in: ".padding(14)  // grid-exempt: a popover").isEmpty)
    }

    @Test func theCollapsedStripsLineReads() {
        #expect(BoardSummaryStrip.collapsedLine(count: 2) == "2 unread")
        #expect(BoardSummaryStrip.collapsedLine(count: 0) == "Nothing unread")
    }
}
