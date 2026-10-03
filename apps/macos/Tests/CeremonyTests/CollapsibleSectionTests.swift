import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Every collapsible section in the navigator is the one shared
/// `CollapsibleSection` (ov-92, owner: "make board UI components reusable so
/// that you can ensure that they look and feel consistent"): drawn through
/// it, by the ids it registers, and with no disclosure of its own left in
/// the navigator's files, nor anywhere else in the app (ov-101).
@MainActor
struct CollapsibleSectionTests {
    private static let sources = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .appendingPathComponent("Sources/FarCooler")

    private static func store() async -> TaskBoardStore {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let now = Int64(Date().timeIntervalSince1970 * 1000) - 60_000
        client.commandRunnerForTesting = { args in
            guard args.starts(with: ["task", "list"]) else { return (Data(), nil) }
            let tasks = [("t1", "bil-1", "Invoice PDF", "in_progress"), ("t2", "bil-2", "Refunds", "todo")]
                .map { id, key, title, status in
                    #"{"id":"\#(id)","key":"\#(key)","title":"\#(title)","status":"\#(status)","status_since":\#(now),"created_at":\#(now),"updated_at":\#(now)}"#
                }
            return (Data(#"{"tasks":[\#(tasks.joined(separator: ","))]}"#.utf8), nil)
        }
        let store = TaskBoardStore(client: client, workspace: .implicit(repository: "r"))
        await store.readIfNeverRead()
        return store
    }

    final class Seen { var ids: Set<String> = [] }

    /// Drawn, the navigator registers every section it has through the
    /// shared component: Tasks and Worktrees, Unread,
    /// every task status, and Hidden under Worktrees. (Fails for any of
    /// them drawn its own way, as all but Worktrees were before.)
    @Test("Every navigator section is drawn through CollapsibleSection")
    func everySectionIsShared() async {
        let store = await Self.store()
        func worktree(_ id: String, state: String = "active") -> Worktree {
            Worktree(
                id: id, short: id, task: id, branch: id, repository: "shop", host: "", path: "/tmp/\(id)",
                state: state, terminals: [])
        }
        let seen = Seen()
        let board = TaskBoardView(
            store: store, client: store.client, agents: .none, onGoTo: { _ in },
            defaults: UserDefaults(suiteName: "sections-\(UUID().uuidString)")!,
            worktrees: { _ in
                BoardWorktrees(shown: [worktree("scratch")], hidden: [worktree("old", state: "hidden")], onNew: {})
            },
            orchestrator: NavigatorOrchestrator(state: .idle, agent: "claude", nowDoing: nil))
        let root = board
            .frame(width: 300, height: 3000)
            .onPreferenceChange(CollapsibleSectionsKey.self) { ids in MainActor.assumeIsolated { seen.ids = ids } }
        let host = NSHostingView(rootView: root)
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 300, height: 3000), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        for _ in 0..<10 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        window.close()
        let expected: Set<String> = Set(["tasks", "summary", "worktrees", "worktrees.hidden"])
            .union(TaskBoardModel.order.map { "status.\($0.rawValue)" })
        #expect(expected.subtracting(seen.ids).isEmpty, "not through the shared section: \(expected.subtracting(seen.ids))")
        // The orchestrator is a row, not a section: there's only ever one
        // (ov-177).
        #expect(!seen.ids.contains("orchestrator"))
    }

    /// No navigator file rolls its own disclosure: no chevron of its own,
    /// no expanded-or-collapsed value for VoiceOver. Only the shared
    /// component draws those.
    @Test("No navigator file draws a disclosure of its own")
    func noFileRollsItsOwn() throws {
        for file in ["TaskBoard.swift", "TaskListSection.swift", "BoardSummaryStrip.swift", "BoardWorktreesSection.swift", "Navigator.swift"] {
            var text = try String(contentsOf: Self.sources.appendingPathComponent(file), encoding: .utf8)
            // TaskBoard.swift's task detail, from `struct TaskCard` on, isn't
            // the navigator.
            if let cut = text.range(of: "struct TaskCard: View") { text = String(text[..<cut.lowerBound]) }
            #expect(!text.contains("\"chevron.right\""), "\(file) draws its own disclosure chevron")
            #expect(!text.contains("\"Expanded\""), "\(file) says Expanded for itself")
        }
    }

    /// The disclosures in `text`, a Swift file's source:
    /// - a system `DisclosureGroup` or `OutlineGroup`;
    /// - a disclosure glyph (a chevron or a triangle), or an image named by
    ///   a variable, turned by `rotationEffect` within eight lines;
    /// - a glyph swapped for another, by a ternary or an `if`/`else`;
    /// - a Show/Hide button toggling something without a chevron at all.
    ///
    /// Comments aside, and a line saying `not a disclosure` (a sort arrow
    /// that turns, say) is excused.
    static func disclosures(in text: String) -> [String] {
        let raw = text.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        let lines = raw.map { line -> String in
            if line.contains("not a disclosure") { return "" }
            // Code only: what a comment says about a disclosure isn't one.
            guard let cut = line.range(of: "//") else { return line }
            return String(line[..<cut.lowerBound])
        }
        var found: [String] = []
        let system = try! Regex(#"\b(DisclosureGroup|OutlineGroup|disclosureGroupStyle)\b"#)
        let glyph = try! Regex(#""(chevron\.(right|down|forward)|arrowtriangle\.|triangle)"#)
        let named = try! Regex(#"systemName:\s*[A-Za-z_(]"#)
        let swap = try! Regex(#"\?\s*"(chevron|arrowtriangle|triangle)[a-z.]*"\s*:\s*"(chevron|arrowtriangle|triangle)"#)
        let words = try! Regex(#"Button\([^"]*\?\s*("Show\b[^"]*"\s*:\s*"Hide|"Hide\b[^"]*"\s*:\s*"Show)\b"#)
        for (index, line) in lines.enumerated() {
            let next = lines[index..<min(lines.count, index + 8)]
            if line.contains(system) { found.append("\(index + 1): a system disclosure") }
            if line.contains(swap) { found.append("\(index + 1): a glyph swapped for another") }
            if line.contains(words) { found.append("\(index + 1): Show and Hide words for a disclosure") }
            if line.contains(glyph) || line.contains(named) {
                if next.contains(where: { $0.contains(".rotationEffect") }) {
                    found.append("\(index + 1): a disclosure glyph of its own, turned")
                }
                if line.contains(glyph),
                    let other = next.firstIndex(where: { $0.contains("} else {") }),
                    lines[other..<min(lines.count, other + 4)].contains(where: { $0.contains(glyph) })
                {
                    found.append("\(index + 1): a glyph swapped for another by if/else")
                }
            }
        }
        return found
    }

    /// No file in the app draws a disclosure of its own (ov-101): every one
    /// is `CollapsibleSection` or `DisclosureButton`, and only
    /// `Components/CollapsibleSection.swift` turns a chevron.
    @Test("No file in the app rolls its own disclosure")
    func noBespokeDisclosures() throws {
        let files = FileManager.default.enumerator(at: Self.sources, includingPropertiesForKeys: nil)!
            .compactMap { $0 as? URL }
            .filter { $0.pathExtension == "swift" && $0.lastPathComponent != "CollapsibleSection.swift" }
        #expect(files.count > 50, "the scan found the sources")
        for file in files {
            for hit in Self.disclosures(in: try String(contentsOf: file, encoding: .utf8)) {
                Issue.record("\(file.lastPathComponent):\(hit); use CollapsibleSection or DisclosureButton")
            }
        }
    }

    /// The scan sees each kind, and passes what isn't one.
    @Test("The disclosure scan catches each kind and excuses the rest")
    func scanCatches() {
        let caught = [
            "DisclosureGroup(\"Details\") { Text(\"x\") }",
            "Image(systemName: \"chevron.right\")\n    .font(.caption2)\n    .rotationEffect(.degrees(open ? 90 : 0))",
            "Image(systemName: \"arrowtriangle.right.fill\")\n    .rotationEffect(.degrees(open ? 90 : 0))",
            "Image(systemName: glyph)\n    .rotationEffect(.degrees(open ? 90 : 0))",
            "Image(systemName: open ? \"chevron.down\" : \"chevron.right\")",
            "if open {\n    Image(systemName: \"chevron.down\")\n} else {\n    Image(systemName: \"chevron.right\")\n}",
            "Button(open ? \"Hide\" : \"Show\") { open.toggle() }",
        ]
        for source in caught { #expect(Self.disclosures(in: source).count == 1, "missed: \(source)") }
        let passed = [
            "// the DisclosureGroup it was",
            "Image(systemName: \"chevron.forward\")",
            "Image(systemName: \"arrow.up\")  // not a disclosure: a sort order\n    .rotationEffect(.degrees(up ? 0 : 180))",
            ".accessibilityAction(named: open ? \"Hide Worktrees\" : \"Show Worktrees\") {}",
            "Button(agent ? \"Show as Terminal\" : \"Show as Chat\", action: flip)",
        ]
        for source in passed { #expect(Self.disclosures(in: source).isEmpty, "wrongly caught: \(source)") }
    }

    /// The clip is on only while a section closes: opened again before the
    /// close settles, it's off at once, and that close settling later
    /// doesn't touch it (ov-101 review, M3).
    @Test("A section is clipped only while it closes")
    func clipOnlyWhileClosing() {
        var clip = SectionClip()
        #expect(!clip.closing)
        let first = clip.close()
        #expect(clip.closing)
        clip.settle(first)
        #expect(!clip.closing)
        // Closed, then opened within the settle: unclipped at once.
        let second = clip.close()
        clip.open()
        #expect(!clip.closing)
        // Closed again; the first close's late settle leaves it clipped,
        // its own settle clears it.
        let third = clip.close()
        clip.settle(second)
        #expect(clip.closing)
        clip.settle(third)
        #expect(!clip.closing)
    }

    /// Under Reduce Motion every collapsible cross-fades instead of
    /// springing, and the section reaches motion only through `BoardMotion`.
    @Test("Under Reduce Motion a section cross-fades, and moves only through BoardMotion")
    func reduceMotion() throws {
        #expect(BoardMotion.list(reduceMotion: true) == .easeInOut(duration: 0.2))
        #expect(BoardMotion.list(reduceMotion: false) == WorkspaceMotion.spring)
        let source = try String(
            contentsOf: Self.sources.appendingPathComponent("Components/CollapsibleSection.swift"), encoding: .utf8)
        #expect(!source.contains("withAnimation("), "a section animates other than through BoardMotion")
        #expect(!source.contains("WorkspaceMotion.spring"), "a section springs whatever Reduce Motion says")
        #expect(source.contains("BoardMotion.toggle(reduceMotion: reduceMotion"))
        #expect(source.contains("contentTransition(reduceMotion: reduceMotion"))
    }

    /// The header answers the keyboard and VoiceOver the same way
    /// everywhere: the state it reads from defaults when it keeps its own.
    @Test("A keyed section reads its stored state, open by default")
    func keyedState() {
        let defaults = UserDefaults(suiteName: "keyed-\(UUID().uuidString)")!
        typealias Section = CollapsibleSection<SectionTitle, EmptyView, EmptyView>
        #expect(Section.read(key: "k", defaults: defaults, default: true))
        defaults.set(false, forKey: "k")
        #expect(!Section.read(key: "k", defaults: defaults, default: true))
        #expect(Section.gridRow("status.todo") == "status")
    }
}
