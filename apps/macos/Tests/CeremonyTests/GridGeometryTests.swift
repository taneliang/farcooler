import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The sidebar and the board column on one grid (ov-83): every chevron, icon
/// and text start of every row type lands on a `ColumnGrid` column, and on
/// the one the owner's ruling names.
///
/// Measured, not computed. The real row views are drawn, each reporting
/// where its marks landed through `gridMark(_:_:)`, at the depth
/// `ContentView.sidebarRows` gives it; and the board is the real
/// `TaskBoardView`, over a board read through a stubbed CLI. ov-78's
/// `SidebarColumnTests` read the constants the rows were meant to lay out
/// from, and passed while a repository's name sat 12 pt off its column,
/// pushed there by a chevron cell drawn invisibly.
@MainActor
struct GridGeometryTests {
    struct Mark: CustomStringConvertible {
        let row: String
        let role: GridRole
        let x: CGFloat
        var description: String { "\(row).\(role.rawValue) at \(x)" }
    }

    final class Box {
        var marks: [Mark] = []
        func record(_ marks: [GridMark], _ proxy: GeometryProxy) {
            self.marks = marks.map { Mark(row: $0.row, role: $0.role, x: proxy[$0.bounds].minX) }
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
        for mark in marks {
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

    // MARK: - The sidebar

    /// An agent with a feed and a running subagent, so the lines under its
    /// name are drawn too.
    private static let terminal: Terminal = {
        var terminal = Terminal(
            id: "t", short: "t", title: "claude", preset: "claude", state: "running", epoch: 0)
        terminal.line = "Reading the board"
        terminal.feed = ["Ran the tests"]
        terminal.subagents = ["explorer"]
        return terminal
    }()

    private static func worktree(_ id: String, workspace: String?, hidden: Bool = false) -> Worktree {
        Worktree(
            id: id, short: id, task: id, branch: "feat/\(id)", repository: "r", host: "",
            path: "/tmp/\(id)", state: hidden ? "hidden" : "active", terminals: [terminal],
            repositoryID: "repo", workspace: workspace)
    }

    /// One repository: Main with a worktree, an empty workspace, and an
    /// unclaimed worktree; every row type the sidebar draws, at the depths
    /// `sidebarRows` gives them.
    private static func rows() -> [SidebarEntry] {
        var fleet = Fleet(
            runtimeHealthy: true, livePanes: 0,
            worktrees: [worktree("w", workspace: "ws"), worktree("u", workspace: nil)],
            branchPrefix: nil)
        fleet.runnerWorkspaces[""] = [
            WorkspaceSummary(id: "ws", name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0, repository: "repo"),
            WorkspaceSummary(id: "e", name: "Empty", taskPrefix: "em", isMain: false, ordinal: 1, repository: "repo"),
        ]
        return ContentView.sidebarRows(fleet: fleet, open: { _ in true })
    }

    private struct Search: View {
        @State private var query = ""
        @FocusState private var focused: Bool
        var body: some View { SidebarSearchRow(query: $query, focused: $focused) }
    }

    private static func section(_ worktree: Worktree) -> WorktreeSection {
        WorktreeSection(
            worktree: worktree, isExpanded: true, selected: nil, onSelect: { _ in }, onToggle: {},
            onNewTerminal: {}, onHide: {}, onUnhide: {}, onRemove: {},
            onTerminalAction: { _, _ in })
    }

    @ViewBuilder
    private static func draw(_ entry: SidebarEntry) -> some View {
        switch entry.kind {
        case .repository:
            ProjectHeader(name: entry.project, count: 1, onToggleCollapse: {})
                .sidebarDepth(entry.depth)
        case .workspace(let name):
            WorkspaceRow(
                name: name, workspace: entry.workspace?.id ?? "", taskPrefix: "fc", seat: nil,
                implicit: false, count: 1, unread: false, isSelected: false, onSelect: {},
                actions: nil, isOpen: true)
                .sidebarDepth(entry.depth)
        case .worktree:
            section(entry.worktree!).sidebarDepth(entry.depth)
        case .noWorktrees:
            NoWorktreesRow().sidebarDepth(entry.depth)
        case .unclaimed:
            UnclaimedWorktrees(
                worktrees: entry.worktrees, isExpanded: true, onToggle: {},
                row: { section($0).sidebarDepth(1) })
                .sidebarDepth(entry.depth)
        case .hidden:
            EmptyView()
        }
    }

    @Test("Every sidebar row's chevron, icon and text is on a grid column")
    func theSidebarIsOnTheGrid() async {
        let rows = Self.rows()
        #expect(
            rows.map(\.kind) == [
                .repository, .workspace("Main"), .worktree("w"), .workspace("Empty"), .noWorktrees,
                .unclaimed(count: 1),
            ])
        let sidebar = VStack(alignment: .leading, spacing: 0) {
            SidebarTitleRow(busy: false) { EmptyView() }
            Search()
            NeedsYouRow(count: 2, isSelected: false, onSelect: {})
            ForEach(rows) { Self.draw($0) }
            HiddenWorktrees(
                project: "r", worktrees: [Self.worktree("h", workspace: nil, hidden: true)],
                isExpanded: true, onToggle: {}, onUnhide: { _ in })
            // A silent runner's header, which names the runner.
            ProjectHeader(name: "carl", count: 0)
        }
        let found = await marks(sidebar, width: 260, height: 1200)
        check(found, expect: [
            "title.text": ColumnGrid.a,
            "search.text": ColumnGrid.a,
            "needsYou.icon": ColumnGrid.a,
            "needsYou.text": ColumnGrid.b,
            // The shared section's chevron at A, the name at B (ov-101).
            "repository.chevron": ColumnGrid.a,
            "repository.text": ColumnGrid.b,
            "workspace.chevron": ColumnGrid.a,
            "workspace.icon": ColumnGrid.b,
            "workspace.text": ColumnGrid.c,
            "worktree.chevron": ColumnGrid.b,
            "worktree.icon": ColumnGrid.c,
            "worktree.text": ColumnGrid.d,
            "worktree.branch.text": ColumnGrid.d,
            "terminal.icon": ColumnGrid.d,
            "terminal.text": ColumnGrid.column(4),
            "terminal.step.text": ColumnGrid.column(4),
            "terminal.subagent.text": ColumnGrid.column(4),
            "noWorktrees.text": ColumnGrid.c,
            "group.chevron": ColumnGrid.a,
            "group.icon": ColumnGrid.b,
            "group.text": ColumnGrid.c,
            "hidden.text": ColumnGrid.d,
            "runner.icon": ColumnGrid.a,
            "runner.text": ColumnGrid.b,
        ])
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

    @Test("Every board row's chevron and text is on the board column's grid", arguments: [false, true])
    func theBoardIsOnTheGrid(collapsedSummary: Bool) async {
        let store = await Self.store()
        #expect(store.board.rows.count == 3)
        let defaults = UserDefaults(suiteName: "grid-\(UUID().uuidString)")!
        defaults.set(collapsedSummary, forKey: "board.summary.collapsed.\(store.hostKey).\(store.workspace.id)")
        let board = TaskBoardView(
            store: store, client: store.client, agents: .none, onGoTo: { _ in }, defaults: defaults)
        let found = await marks(board, width: WorkspaceColumns.navigatorDefault)
        var expect: [String: CGFloat] = [
            "header.text": ColumnGrid.b,
            // The filter's glyph and text on a row's icon and title columns
            // (ov-177).
            "filter.icon": ColumnGrid.b,
            "filter.text": ColumnGrid.c,
            "summary.chevron": ColumnGrid.a,
            "summary.text": ColumnGrid.b,
            "status.chevron": ColumnGrid.a,
            "tasks.chevron": ColumnGrid.a,
            "tasks.text": ColumnGrid.b,
            "status.text": ColumnGrid.b,
            "card.text": ColumnGrid.b,
        ]
        if !collapsedSummary {
            expect["summary.group.text"] = ColumnGrid.b
            expect["summary.key.text"] = ColumnGrid.b
        }
        check(found, expect: expect)
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
        ("SidebarViews.swift", nil), ("SidebarLayout.swift", nil), ("WorkspaceSidebar.swift", nil),
        ("BoardHeader.swift", nil), ("BoardSummaryStrip.swift", nil),
        ("TaskBoard.swift", "struct TaskCard: View"), ("TaskListSection.swift", nil), ("Navigator.swift", nil), ("BoardWorktreesSection.swift", nil),
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
