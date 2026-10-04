import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The navigator's vertical rhythm (ov-243): the gap over and under every
/// level, measured on the real `TaskBoardView` drawn offscreen, from the
/// frames its rows, titles and rules really land at.
///
/// A gap is measured between what the eye reads as the two things: a
/// title's line, a row's text (its box less the row's own padding), a box,
/// a rule. The owner's screenshot of 3 October found Tasks → Unread as wide
/// as the box → Tasks gap, Unread → Finished narrow, and the gap over Needs
/// You or Review wider than any.
@MainActor
struct NavigatorRhythmTests {
    /// What a line is, for the rhythm.
    enum Kind: String {
        case box, divider, section, group, subgroup, row, line
    }

    struct Line: CustomStringConvertible {
        let kind: Kind
        let name: String
        let top: CGFloat
        let bottom: CGFloat
        var description: String { "\(kind.rawValue) \(name) \(top)–\(bottom)" }
    }

    final class Box {
        var marks: [(GridMark, CGRect)] = []
        var probes: [(String, CGRect)] = []
        /// The split's rules, with their handlers (`NavigatorRuleReport`).
        var rules: [NavigatorRuleReport] = []
        /// The list's arrows (`NavigatorArrowsReport`).
        var arrows: [NavigatorArrowsReport] = []
    }

    struct Probe<Content: View>: View {
        let box: Box
        let content: Content
        var body: some View {
            content
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(GridMarksKey.self) { marks in
                    GeometryReader { proxy in
                        let _ = box.marks = marks.map { ($0, proxy[$0.bounds]) }
                        Color.clear
                    }
                }
                .overlayPreferenceValue(ProbedViewsKey.self) { probes in
                    GeometryReader { proxy in
                        let _ = box.probes = probes.map { ($0.id, proxy[$0.bounds]) }
                        Color.clear
                    }
                }
                .overlayPreferenceValue(NavigatorRulesKey.self) { rules in
                    let _ = box.rules = rules
                    Color.clear
                }
                .overlayPreferenceValue(NavigatorArrowsKey.self) { arrows in
                    let _ = box.arrows = arrows
                    Color.clear
                }
        }
    }

    // MARK: - Boards

    /// A board read through a stubbed CLI: `done` tasks finished in the last
    /// hour, so Unread's Finished is long, `review` waiting, and a few of
    /// every other status.
    static func store(done: Int, review: Int, others: Bool, read: Bool = false) async -> TaskBoardStore {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let now = Int64(Date().timeIntervalSince1970 * 1000) - 60_000
        var tasks: [(String, String)] = []
        for n in 0..<done { tasks.append(("Finish the sidebar rhythm, round \(n), with a title long enough to wrap", "done")) }
        for n in 0..<review { tasks.append(("Review the toolbar pass \(n)", n % 2 == 0 ? "needs_decision" : "in_review")) }
        if others {
            tasks += [
                ("Coordinator", "in_progress"), ("Glance specimens", "in_progress"),
                ("Scope the relay", "todo"), ("Watch app", "backlog"),
            ]
        }
        let rows = tasks.enumerated().map { i, task in
            let (title, status) = task
            return #"{"id":"t\#(i)","key":"ov-\#(100 + i)","title":"\#(title)","status":"\#(status)","status_since":\#(now - Int64(i) * 60_000),"created_at":\#(now - 86_400_000),"updated_at":\#(now)}"#
        }
        client.commandRunnerForTesting = { args in
            guard args.starts(with: ["task", "list"]) else { return (Data(), nil) }
            return (Data(#"{"tasks":[\#(rows.joined(separator: ","))]}"#.utf8), nil)
        }
        let store = TaskBoardStore(client: client, workspace: .implicit(repository: "r"))
        await store.readIfNeverRead()
        if read { for row in store.board.rows { store.markRead(row) } }
        return store
    }

    static func board(_ store: TaskBoardStore, terminals: Int = 2, worktrees: Int = 2) -> TaskBoardView {
        var shell = ProjectTerminals(
            terminals: (0..<terminals).map {
                Terminal(id: "term\($0)", short: "term\($0)", title: "proxy \($0)", preset: "zsh", state: "running", epoch: 0)
            })
        shell.onNew = {}
        var loose = BoardWorktrees(
            shown: (0..<worktrees).map {
                Worktree(
                    id: "w\($0)", short: "w\($0)", task: "spike-\($0)", branch: "spike-\($0)", repository: "r", host: "",
                    path: "/tmp/w\($0)", state: "active", terminals: [], repositoryID: "r", workspace: nil)
            }, terminals: shell)
        loose.onNew = {}
        return TaskBoardView(
            store: store, client: store.client, agents: .none, onGoTo: { _ in },
            defaults: UserDefaults(suiteName: "rhythm-\(UUID().uuidString)")!,
            worktrees: { _ in loose },
            orchestrator: NavigatorOrchestrator(
                state: .working, agent: "claude", status: .working, nowDoing: "Reading the board"))
    }

    // MARK: - Measuring

    /// Draw `view` in an unshown window and read where every line landed,
    /// top to bottom; with `capture`, write it as a PNG there too.
    static func lines<V: View>(
        _ view: V, width: CGFloat = WorkspaceColumns.navigatorDefault, height: CGFloat = 2400,
        capture: URL? = nil, dark: Bool = false
    ) async -> [Line] {
        let box = Box()
        let root = Probe(box: box, content: view.frame(width: width, height: height, alignment: .topLeading))
        let host = NSHostingView(rootView: root)
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: width, height: height), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        window.contentView = host
        defer { window.close() }
        await settle(host) { "\(box.marks.map(\.1)) \(box.probes.map(\.1))" }
        if let capture, let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds) {
            host.cacheDisplay(in: host.bounds, to: rep)
            try? rep.representation(using: .png, properties: [:])?.write(to: capture)
        }
        return classify(marks: box.marks, probes: box.probes)
    }

    /// Lay `host` out until what `reading` reads has held still for five
    /// passes in a row: the navigator's panes take a pass or two to measure
    /// their rows (`NavigatorSplitView`), more on a busy machine running the
    /// whole suite at once. At least ten passes, at most 150 (3 s).
    static func settle(_ host: NSView, reading: () -> String) async {
        var last = "", still = 0
        for pass in 0..<150 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
            let now = reading()
            still = now == last ? still + 1 : 0
            last = now
            if pass >= 10, still >= 5 { return }
        }
    }

    /// A row's text sits this far inside its box (`NavigatorRowStyle`).
    static let rowInset = ColumnGrid.rhythm / 2

    static func classify(marks: [(GridMark, CGRect)], probes: [(String, CGRect)]) -> [Line] {
        var lines: [Line] = []
        func add(_ kind: Kind, _ name: String, _ rect: CGRect, inset: CGFloat = 0) {
            lines.append(Line(kind: kind, name: name, top: rect.minY + inset, bottom: rect.maxY - inset))
        }
        for (mark, rect) in marks {
            switch (mark.row, mark.role) {
            case ("filter", .box): add(.box, "filter", rect)
            case ("orchestrator", .box): add(.box, "orchestrator", rect, inset: rowInset)
            case ("tasks", .text), ("terminals", .text), ("worktrees", .text): add(.section, mark.row, rect)
            case ("summary", .text), ("status", .text): add(.group, mark.row, rect)
            case ("summary.key", .box), ("card", .box), ("projectTerminal", .box), ("boardWorktree", .box):
                add(.row, mark.row, rect, inset: rowInset)
            case ("projectTerminalNew", .text): add(.line, "New Terminal", rect)
            case ("summary.empty", .text): add(.line, "Nothing new", rect)
            default: break
            }
        }
        for (id, rect) in probes {
            switch id {
            case "navigator-divider": add(.divider, id, rect)
            case "subgroup-title": add(.subgroup, id, rect)
            case "summary-more": add(.line, "and N more", rect, inset: NavigatorRhythm.air)
            case "board-new-worktree-line": add(.line, "New Worktree", rect, inset: NavigatorRhythm.air)
            default: break
            }
        }
        // Each once, top to bottom: a mark can be reported by more than one
        // view on the same rect.
        var seen = Set<String>()
        return lines.sorted { ($0.top, $0.bottom) < ($1.top, $1.bottom) }.filter {
            seen.insert("\($0.kind) \(Int(($0.top * 4).rounded())) \(Int(($0.bottom * 4).rounded()))").inserted
        }
    }

    /// Each gap, from one line's foot to the next one's top, named by both.
    static func gaps(_ lines: [Line]) -> [(Line, Line, CGFloat)] {
        zip(lines, lines.dropFirst()).map { ($0, $1, $1.top - $0.bottom) }
    }

    static func report(_ lines: [Line]) -> String {
        gaps(lines).map { a, b, gap in
            "\(a.kind.rawValue) \(a.name) → \(b.kind.rawValue) \(b.name): \(String(format: "%.1f", gap))"
        }.joined(separator: "\n")
    }

    // MARK: - The rhythm

    /// A header's level: a section's over a group's over a subgroup's.
    static func rank(_ kind: Kind) -> Int? {
        switch kind {
        case .section: 3
        case .group: 2
        case .subgroup: 1
        default: nil
        }
    }

    /// The gap `NavigatorRhythm` sets between `a` and the line after it, `b`.
    static func expected(_ a: Line, _ b: Line) -> CGFloat {
        let r = NavigatorRhythm.self
        if a.kind == .box && b.kind == .box { return r.band + r.air }
        if a.kind == .divider || b.kind == .divider { return r.visible(r.rule) - r.air }
        guard let level = rank(b.kind) else { return r.visible(r.row) }
        // A header's first child sits as close to it as a row to a row.
        if let over = rank(a.kind), over > level { return r.visible(r.row) }
        switch b.kind {
        case .section: return r.visible(r.section)
        case .group: return r.visible(r.group)
        default: return r.visible(r.subgroup)
        }
    }

    /// Every gap down the navigator is the one its levels name, within a
    /// point: a text's line box is drawn to whole pixels, so at 1x (CI's
    /// runner) it can land half a point either way at both ends. The
    /// smallest step between two levels is 4 pt.
    static func checkRhythm(_ lines: [Line], _ name: String) {
        #expect(lines.count > 8, "\(name): only \(lines)")
        for (a, b, gap) in gaps(lines) {
            let want = expected(a, b)
            #expect(abs(gap - want) <= 1, "\(name): \(a) → \(b) is \(gap) apart, not \(want)")
        }
    }

    @Test("Every gap in a long navigator is its level's", arguments: ["many", "empty"])
    func theRhythmHolds(_ board: String) async {
        let many = board == "many"
        // "empty" is a board with one task, read: Unread says "Nothing new". A
        // board with no task at all draws its own state (ov-205).
        let store = await Self.store(done: many ? 32 : 1, review: many ? 4 : 0, others: many, read: !many)
        let found = await Self.lines(Self.board(store), height: many ? 1800 : 700)
        let kinds = Set(found.map(\.kind))
        // Every level is drawn, so every rule above was checked.
        let drawn: Set<Kind> = many
            ? [.box, .divider, .section, .group, .subgroup, .row, .line] : [.box, .divider, .section, .group, .row, .line]
        #expect(kinds == drawn, "\(board): drew \(kinds.map(\.rawValue).sorted())")
        Self.checkRhythm(found, board)
    }

    /// The table's own shape: more room over a higher header than over a
    /// lower one, and less under any header than over it.
    @Test("A higher level has more room over it, and every header is closest to its own")
    func theTableIsAHierarchy() {
        let r = NavigatorRhythm.self
        let above = [r.subgroup, r.group, r.section].map(r.visible)
        #expect(above == above.sorted() && Set(above).count == 3, "\(above)")
        #expect(above.allSatisfy { $0 > r.visible(r.row) }, "a header as close to the rows over it as to its own")
        #expect(2 * (r.visible(r.rule) - r.air) == r.visible(r.section), "a rule off the middle of its section gap")
        // The table in the spec, pinned: rows 8 apart, headers 12, 16, 24.
        #expect(r.visible(r.row) == 8 && above == [12, 16, 24], "\(r.visible(r.row)), \(above)")
    }

    // MARK: - No bare vertical numbers

    /// The navigator's files, and how much of each is the navigator's:
    /// TaskBoard.swift's task detail, from `struct TaskCard` on, isn't.
    static let navigatorFiles: [(name: String, until: String?)] = [
        ("SidebarLayout.swift", nil), ("BoardSummaryStrip.swift", nil),
        ("TaskBoard.swift", "struct TaskCard: View"), ("TaskListSection.swift", nil), ("Navigator.swift", nil),
        ("BoardWorktreesSection.swift", nil), ("ProjectTerminalsSection.swift", nil),
        ("Components/CompactTaskRow.swift", nil), ("Components/CollapsibleSection.swift", nil),
        ("NavigatorSplit.swift", nil),
    ]

    /// Every vertical gap in `source` (a stack's spacing, a top, bottom,
    /// vertical or all-round padding, a least height) set by anything but a
    /// `NavigatorRhythm` name or 0, with its line, that no
    /// `// rhythm-exempt:` explains. An `HStack`'s spacing and a `Spacer`'s
    /// length are left alone: in a row they're horizontal.
    static func strays(in source: String) -> [(line: Int, text: String)] {
        let named = #"(?!\s*-?(?:NavigatorRhythm\.|0(?![.\d])))"#
        let pattern = try! Regex(
            #"(?:\b(?:VStack|LazyVStack)\([^)]*spacing:"# + named + #"|\.padding\(\.(?:top|bottom|vertical),"#
                + named + #"|\.padding\((?!\.)"# + named + #"|\.frame\([^)]*minHeight:"# + named + ")")
        return source.split(separator: "\n", omittingEmptySubsequences: false).enumerated()
            .filter { $0.element.contains(pattern) && !$0.element.contains("rhythm-exempt:") }
            .map { ($0.offset + 1, $0.element.trimmingCharacters(in: .whitespaces)) }
    }

    @Test("The navigator's files space lines by the rhythm's names, never by a number")
    func noBareVerticalNumbers() throws {
        let sources = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/FarCooler")
        for file in Self.navigatorFiles {
            var text = try String(contentsOf: sources.appendingPathComponent(file.name), encoding: .utf8)
            if let until = file.until, let end = text.range(of: until) { text = String(text[..<end.lowerBound]) }
            for stray in Self.strays(in: text) {
                Issue.record("\(file.name):\(stray.line) spaces by a number: \(stray.text)")
            }
        }
    }

    @Test func theScanCatchesABareGap() {
        #expect(Self.strays(in: "x\n    .padding(.top, 5)\n").map(\.line) == [2])
        #expect(Self.strays(in: "VStack(alignment: .leading, spacing: 2) {").count == 1)
        #expect(Self.strays(in: "LazyVStack(spacing: 0.5) {").count == 1)
        #expect(Self.strays(in: "VStack(spacing: ColumnGrid.rhythm / 2) {").count == 1)
        #expect(Self.strays(in: ".padding(.vertical, ColumnGrid.rhythm)").count == 1)
        #expect(Self.strays(in: ".padding(12)").count == 1)
        #expect(Self.strays(in: ".frame(maxWidth: .infinity, minHeight: 6 * ColumnGrid.rhythm)").count == 1)
        #expect(Self.strays(in: "VStack(spacing: 0) {").isEmpty)
        #expect(Self.strays(in: "HStack(spacing: 6) {").isEmpty)
        #expect(Self.strays(in: ".padding(.horizontal, NavigatorGrid.edge)").isEmpty)
        #expect(Self.strays(in: ".padding(.vertical, NavigatorRhythm.air)").isEmpty)
        #expect(Self.strays(in: ".padding(.top, -NavigatorRhythm.air)").isEmpty)
        #expect(Self.strays(in: ".padding(.top, 3)  // rhythm-exempt: a badge").isEmpty)
    }

    // MARK: - Captures

    /// Write the captures and the measured gaps, when asked to
    /// (`FARCOOLER_RHYTHM_OUT`). Not a check: the checks are below.
    @Test("Write the navigator's rhythm captures")
    func writeCaptures() async throws {
        guard let out = ProcessInfo.processInfo.environment["FARCOOLER_RHYTHM_OUT"] else { return }
        let directory = URL(fileURLWithPath: out)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let label = ProcessInfo.processInfo.environment["FARCOOLER_RHYTHM_LABEL"] ?? "capture"
        var text = ""
        for (name, done, review, others) in [("many", 32, 4, true), ("empty", 1, 0, false)] {
            for dark in [false, true] {
                let store = await Self.store(done: done, review: review, others: others, read: name == "empty")
                let found = await Self.lines(
                    Self.board(store), height: name == "many" ? 1800 : 700,
                    capture: directory.appendingPathComponent("\(label)-\(name)-\(dark ? "dark" : "light").png"),
                    dark: dark)
                if !dark { text += "## \(label) \(name)\n\(Self.report(found))\n\n" }
            }
        }
        try text.write(to: directory.appendingPathComponent("\(label)-gaps.txt"), atomically: true, encoding: .utf8)
    }
}
