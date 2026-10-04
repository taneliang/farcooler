import AgentKit
import AppKit
import Foundation
import Testing

@testable import Far_Cooler

/// Task keys in terminal output (ov-215): a key an agent prints opens its
/// task with ⌘-click, the way a URL there opens, by the rule the Markdown
/// links follow (`test/fixtures/task-key-links.json`).
@MainActor
struct TerminalTaskKeyTests {
    final class Seen {
        var tasks: [String] = []
        var urls: [URL] = []
    }

    /// A pane showing `text` on its first row, whose linker knows ov-190 and
    /// ov-7 and records what it opens.
    static func pane(_ text: String, seen: Seen) -> TerminalRenderView {
        let view = TerminalRenderView()
        view.frame = CGRect(x: 0, y: 0, width: 900, height: 300)
        view.layoutSubtreeIfNeeded()
        view.setPaneGrid(PaneGrid(columns: 50, rows: 6))
        view.feed(Array(text.utf8))
        let targets = ["ov-190": "t190", "ov-7": "t7"].map {
            ($0.key, TaskKeyTarget(runner: "", workspace: "w", task: $0.value, key: $0.key))
        }
        view.taskKeyLinker = TaskKeyLinker(
            index: TaskKeyIndex(runner: "", prefixes: ["ov"], targets: Dictionary(uniqueKeysWithValues: targets))
        ) { seen.tasks.append($0.task) }
        return view
    }

    /// A ⌘ event over cell (`row`, `column`), sent to the view itself, so no
    /// event reaches the system.
    static func event(
        _ type: NSEvent.EventType, over view: TerminalRenderView, row: Int, column: Int
    ) throws -> NSEvent {
        let cell = TerminalMetrics.cell(Preferences.shared.terminalFont())
        let pad = TerminalMetrics.padding
        let point = view.convert(
            CGPoint(
                x: pad.left + (CGFloat(column) + 0.5) * cell.width, y: pad.top + (CGFloat(row) + 0.5) * cell.height),
            to: nil)
        return try #require(
            NSEvent.mouseEvent(
                with: type, location: point, modifierFlags: [.command], timestamp: 0, windowNumber: 0, context: nil,
                eventNumber: 0, clickCount: 1, pressure: 1))
    }

    @Test("A ⌘-click on a known key opens its task, from any cell of it")
    func clickOpensTheTask() throws {
        let seen = Seen()
        let view = Self.pane("done in ov-190.", seen: seen)
        // "done in " is eight cells; the key is columns 8...13.
        for column in [8, 11, 13] {
            view.mouseDown(with: try Self.event(.leftMouseDown, over: view, row: 0, column: column))
        }
        #expect(seen.tasks == ["t190", "t190", "t190"])
    }

    @Test("Unknown keys, versions and keys in URLs don't link, and a URL still opens as a URL")
    func nothingElseLinks() throws {
        let seen = Seen()
        let view = Self.pane("ov-191 ov-1.2 https://x.dev/ov-190 utf-8", seen: seen)
        for column in [1, 8, 20, 36] {
            #expect(view.link(atRow: 0, column: column)?.url.hasPrefix("farcooler://") != true, "column \(column)")
        }
        #expect(view.link(atRow: 0, column: 1) == nil)
        #expect(view.link(atRow: 0, column: 8) == nil)
        // The URL is the URL's: handed to the opener, never to the linker.
        view.openLink(atRow: 0, column: 20) { seen.urls.append($0) }
        #expect(seen.urls.map(\.absoluteString) == ["https://x.dev/ov-190"])
        #expect(seen.tasks.isEmpty)
    }

    @Test("A task key never reaches the system opener")
    func neverTheSystem() throws {
        let seen = Seen()
        let view = Self.pane("see ov-7", seen: seen)
        #expect(view.openLink(atRow: 0, column: 5) { seen.urls.append($0) })
        #expect(seen.urls.isEmpty, "farcooler:// goes to the app, not to whichever channel claimed the scheme")
        #expect(seen.tasks == ["t7"])
    }

    @Test("Holding ⌘ over a key underlines it as a link, and letting go clears it")
    func hoverMarksTheKey() throws {
        let seen = Seen()
        let view = Self.pane("ov-7 here", seen: seen)
        view.mouseMoved(with: try Self.event(.mouseMoved, over: view, row: 0, column: 2))
        #expect(view.hoveredLinkForTesting == TaskKeyLinks.url(runner: "", key: "ov-7")?.absoluteString)
        view.mouseMoved(with: try Self.event(.mouseMoved, over: view, row: 0, column: 7))
        #expect(view.hoveredLinkForTesting == nil)
    }
}
