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
        var y: CGFloat = 0
        var height: CGFloat = 0
        var midX: CGFloat { x + width / 2 }
        var midY: CGFloat { y + height / 2 }
        var description: String { "\(row).\(role.rawValue) at \(x), \(width) wide" }
    }

    final class Box {
        var marks: [Mark] = []
        func record(_ marks: [GridMark], _ proxy: GeometryProxy) {
            self.marks = marks.map { 
                let bounds = proxy[$0.bounds]
                return Mark(
                    row: $0.row, role: $0.role, x: bounds.minX, width: bounds.width, y: bounds.minY,
                    height: bounds.height)
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
        await NavigatorRhythmTests.settle(host) { "\(box.marks.map(\.description))" }
        return box.marks
    }

    /// Every mark on a column, each named one on its own, and every named
    /// one drawn.
    private func check(_ marks: [Mark], expect: [String: CGFloat]) {
        // A box starts past the grid and a glyph is placed by its center
        // (`checkTheLines`); everything else is on a column.
        // The navigator's text column (ov-255) is past the glyph column and
        // its gap, so it is the one place text is not on a `ColumnGrid` column;
        // a task's title follows its key, whose column is the widest key's.
        for mark in marks where mark.role == .text && mark.row != "summary.title" && abs(mark.x - NavigatorGrid.text) >= 0.5 {
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
    static func store() async -> TaskBoardStore {
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
    static func orchestrator(running: Bool) -> NavigatorOrchestrator {
        running
            ? NavigatorOrchestrator(state: .working, agent: "claude", status: .working, nowDoing: "Reading the board")
            : NavigatorOrchestrator(state: .none, offers: [.start(.claude)])
    }

    /// ov-260: the orchestrator's pulse dot (ov-229's Core Animation dot, drawn
    /// only in a live window) occupies the icon's frame: the same center, on
    /// the glyph column's x and on the title's first line. Each state is
    /// measured in the same environment as the others, not against constants.
    @Test("The orchestrator's pulse dot is centered where its icon is")
    func theOrchestratorDotIsWhereTheIconIs() async {
        func row(_ state: OrchestratorRow.State, status: Status?) async -> [Mark] {
            let model = NavigatorOrchestrator(state: state, agent: "claude", status: status, nowDoing: "Reading the board")
            let view = OrchestratorRowView(model: model, inProgress: 1, selected: false, keyed: false)
                .environment(\.inLiveWindow, true)
            return await marks(view, width: WorkspaceColumns.navigatorDefault, height: 120)
        }
        let icon = await row(.idle, status: nil).first { $0.row == "orchestrator" && $0.role == .icon }
        let dot = await row(.working, status: .working).filter { $0.row == "orchestratorDot" }
        let title = await row(.working, status: .working).first { $0.role == .text }
        #expect(dot.count == 1, "the dot's marks: \(dot)")
        guard let icon, let dot = dot.first, let title else { Issue.record("a mark wasn't drawn"); return }
        #expect(abs(dot.midX - icon.midX) <= 0.5, "dot \(dot) vs icon \(icon): x centers \(dot.midX), \(icon.midX)")
        #expect(abs(dot.midY - icon.midY) <= 0.5, "dot \(dot) vs icon \(icon): y centers \(dot.midY), \(icon.midY)")
        // The row alone has no margin, so the glyph column's center is half
        // a cell in; in the board it's `glyphCenter` (`checkTheLines`).
        #expect(abs(dot.midX - NavigatorGrid.mark / 2) <= 0.5, "dot \(dot) isn't centered in the glyph column")
        #expect(abs(dot.midY - title.midY) <= 0.5, "dot \(dot) vs title \(title): y centers \(dot.midY), \(title.midY)")
        // The needs-you and unread dots take the icon's place too (ov-260 review).
        for state in [OrchestratorRow.State.needsYou, .unread] {
            let news = await row(state, status: nil).first { $0.row == "orchestratorDot" }
            guard let news else { Issue.record("no mark for \(state)"); continue }
            #expect(abs(news.midX - icon.midX) <= 0.5, "\(state) \(news) vs icon \(icon): x centers")
            #expect(abs(news.midY - icon.midY) <= 0.5, "\(state) \(news) vs icon \(icon): y centers \(news.midY), \(icon.midY)")
        }
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
        // At 1x (CI's headless runner) a glyph's drawn bounds snap to a whole
        // point, so an odd-width glyph centered on 25 draws from 19 to 32 and
        // its center reads 25.5: half a point off, exactly. That is the
        // tolerance (ov-235). A `.leading` mutation moves a glyph (18 - width)
        // / 2, at least 2.5 for the 13-wide ones, so it still turns this red.
        let snap: CGFloat = 0.5
        // ov-255: the gap between a glyph's right edge and the text after
        // it, as Finder's and Xcode's sidebars leave it (6 to 8 pt after a
        // 16 pt icon). The widest glyph here is 14 pt, so at least 7 less a
        // snap; reverting `gap` to 0 leaves 2 pt and fails.
        #expect(NavigatorGrid.gap >= 5, "the gap constant is \(NavigatorGrid.gap)")
        #expect(NavigatorGrid.text == NavigatorGrid.edge + NavigatorGrid.mark + NavigatorGrid.gap)
        for icon in found where icon.role == .icon {
            let gap = NavigatorGrid.text - (icon.x + icon.width)
            #expect(gap >= 7 - snap, "\(icon) ends \(gap) pt short of the text column, under 7")
        }
        for icon in found where icon.role == .icon {
            #expect(abs(icon.midX - NavigatorGrid.glyphCenter) <= snap, "\(icon) isn't centered on the carets: icon.midX \(icon.midX), glyphCenter \(NavigatorGrid.glyphCenter), glyph width \(icon.width)")
            for caret in found where caret.role == .chevron {
                #expect(abs(icon.midX - caret.midX) <= snap, "\(icon) vs \(caret): icon.midX \(icon.midX), caret.midX \(caret.midX), glyphCenter \(NavigatorGrid.glyphCenter), icon width \(icon.width), caret width \(caret.width)")
            }
        }
    }

    /// The Terminals and Worktrees sections follow the same lines (ov-230):
    /// each row's box past the edge, its glyph over the carets, its name at B.
    @Test("Terminals and worktrees rows sit on the same lines")
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

    /// ov-257: the owner, 4 October: "these circles should probably be
    /// aligned with the numbers as well?" Every count (a section's, a
    /// group's) and every status ring (a terminal's, a worktree's) ends on
    /// the trailing column: the content's right edge less
    /// `NavigatorGrid.trailingInset`. Right edges, so a 7 pt ring and a 12 pt
    /// "3" read as one column.
    ///
    /// The tolerance is 0.5 pt: at 1x (CI's headless runner) a glyph's drawn
    /// bounds snap to a whole point. The row it replaced stood a step (18 pt)
    /// inside the counts, 36 times the tolerance.
    @Test("Status rings and counts end on one trailing column")
    func ringsAndCountsShareTheTrailingColumn() async {
        let store = await Self.store()
        let width = WorkspaceColumns.navigatorDefault
        let found = await marks(NavigatorTrailingSpecimenTests.board(store), width: width, height: 1600)
        let trailing = found.filter { $0.role == .trailing }
        let column = width - NavigatorGrid.edge - NavigatorGrid.trailingInset
        for name in ["count", "groupCount", "projectTerminal", "boardWorktree"] {
            let drawn = trailing.filter { $0.row == name }
            #expect(!drawn.isEmpty, "no \(name) trailing mark was drawn")
            for mark in drawn {
                #expect(
                    abs(mark.x + mark.width - column) <= 0.5,
                    "\(mark) ends at \(mark.x + mark.width), not the trailing column at \(column)")
            }
        }
        // And against each other, not just a constant: the rings' right
        // edges are the counts'.
        let counts = Set(trailing.filter { $0.row == "count" }.map { ($0.x + $0.width).rounded() })
        let rings = trailing.filter { $0.row == "projectTerminal" || $0.row == "boardWorktree" }
        #expect(counts.count == 1, "section counts end at \(counts.sorted())")
        for ring in rings {
            #expect(counts.allSatisfy { abs($0 - (ring.x + ring.width)) <= 1 }, "\(ring) vs counts \(counts)")
        }
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
            "summary.text": NavigatorGrid.text,
            "tasks.text": NavigatorGrid.text,
            "status.text": NavigatorGrid.text,
            "card.text": NavigatorGrid.text,
        ]
        if !collapsedSummary {
            expect["summary.group.text"] = NavigatorGrid.text
            expect["summary.key.text"] = NavigatorGrid.text
        }
        check(found, expect: expect)
        checkTheLines(found)
        // The items' titles in one column after the widest key, "ov-1234":
        // past the key, and the same x for every item.
        let titles = Set(found.filter { $0.row == "summary.title" }.map(\.x))
        #expect(titles.count == (collapsedSummary ? 0 : 1), "titles at \(titles)")
        if let title = titles.first { #expect(title > ColumnGrid.c, "a title at \(title) overlaps its key") }
    }

    /// Closed, the strip is one slot: its line and `NavigatorRhythm.air`
    /// over and under it, and no room of its own (ov-243: the gap around
    /// it is the Tasks section's group gap, `NavigatorRhythmTests`).
    @Test("The collapsed strip is one slot tall")
    func theCollapsedStripIsOneSlotTall() async {
        let store = await Self.store()
        let defaults = UserDefaults(suiteName: "strip-\(UUID().uuidString)")!
        defaults.set(true, forKey: "board.summary.collapsed.\(store.hostKey).\(store.workspace.id)")
        let host = NSHostingController(rootView: BoardSummaryStrip(store: store, defaults: defaults))
        // The line measured here, in the same environment, not a constant.
        let line = NSHostingController(
            rootView: Text(BoardSummaryStrip.collapsedLine(count: 3)).font(.system(size: WorkspaceStyle.PaneText.body)))
        let lineHeight = line.sizeThatFits(in: CGSize(width: 400, height: 400)).height
        for width in [WorkspaceColumns.navigatorMinimum, WorkspaceColumns.navigatorDefault] {
            let height = host.sizeThatFits(in: CGSize(width: width, height: 400)).height
            #expect(
                abs(height - (lineHeight + 2 * NavigatorRhythm.air)) < 0.5,
                "\(height) tall at \(width), its line \(lineHeight)")
        }
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
