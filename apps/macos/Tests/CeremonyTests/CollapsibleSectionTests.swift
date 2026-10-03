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
    /// shared component: the three navigator sections, Unread,
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
        let expected: Set<String> = Set(["orchestrator", "tasks", "summary", "worktrees", "worktrees.hidden"])
            .union(TaskBoardModel.order.map { "status.\($0.rawValue)" })
        #expect(expected.subtracting(seen.ids).isEmpty, "not through the shared section: \(expected.subtracting(seen.ids))")
    }

    /// No navigator file rolls its own disclosure: no chevron of its own,
    /// no expanded-or-collapsed value for VoiceOver. Only the shared
    /// component draws those.
    @Test("No navigator file draws a disclosure of its own")
    func noFileRollsItsOwn() throws {
        for file in ["TaskBoard.swift", "BoardSummaryStrip.swift", "BoardWorktreesSection.swift", "Navigator.swift"] {
            var text = try String(contentsOf: Self.sources.appendingPathComponent(file), encoding: .utf8)
            // TaskBoard.swift's task detail, from `struct TaskCard` on, isn't
            // the navigator.
            if let cut = text.range(of: "struct TaskCard: View") { text = String(text[..<cut.lowerBound]) }
            #expect(!text.contains("\"chevron.right\""), "\(file) draws its own disclosure chevron")
            #expect(!text.contains("\"Expanded\""), "\(file) says Expanded for itself")
        }
    }

    /// The disclosures in `text`, a Swift file's source: a system
    /// `DisclosureGroup` or `OutlineGroup`, a chevron turned by
    /// `rotationEffect` within a few lines of it, or a chevron swapped for
    /// another by a ternary. Comments aside.
    static func disclosures(in text: String) -> [String] {
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false).map { line -> String in
            // Code only: what a comment says about a disclosure isn't one.
            guard let cut = line.range(of: "//") else { return String(line) }
            return String(line[..<cut.lowerBound])
        }
        var found: [String] = []
        let system = try! Regex(#"\b(DisclosureGroup|OutlineGroup|disclosureGroupStyle)\b"#)
        let swap = try! Regex(#"\?\s*"chevron\.[a-z.]+"\s*:\s*"chevron\."#)
        for (index, line) in lines.enumerated() {
            if line.contains(system) { found.append("\(index + 1): a system disclosure") }
            if line.contains(swap) { found.append("\(index + 1): a chevron swapped for another") }
            if line.contains("\"chevron.") {
                let window = lines[index..<min(lines.count, index + 8)]
                if window.contains(where: { $0.contains(".rotationEffect") }) {
                    found.append("\(index + 1): a chevron of its own, turned")
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
        // The scan sees each kind, and not a comment.
        #expect(Self.disclosures(in: "DisclosureGroup(\"Details\") { Text(\"x\") }").count == 1)
        #expect(Self.disclosures(in: "Image(systemName: \"chevron.right\")\n    .font(.caption2)\n    .rotationEffect(.degrees(open ? 90 : 0))").count == 1)
        #expect(Self.disclosures(in: "Image(systemName: open ? \"chevron.down\" : \"chevron.right\")").count == 1)
        #expect(Self.disclosures(in: "// the DisclosureGroup it was").isEmpty)
        #expect(Self.disclosures(in: "Image(systemName: \"chevron.forward\")").isEmpty)
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
