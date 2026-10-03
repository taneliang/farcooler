import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The navigator as the owner found it in ov-177, drawn: the real
/// `TaskBoardView` over a board read through a stubbed CLI, its filter typed
/// into, its keys pressed, and what it drew read back.
///
/// - Filtering narrows without motion: rows, sections and the selection jump
///   to where they now are, and the shared spring is for real changes.
/// - Filtering leaves out what doesn't match: a status with no match, Unread
///   with none; with nothing at all, one "No Results", never "You're all
///   caught up.".
/// - A task opened from Unread keeps its line there, in place, until the
///   selection moves on, and ↑ and ↓ go on from that line.
/// - Mark All as Read is a button on Unread's header, and the task selected
///   keeps its line through it.
@MainActor
@Suite(.serialized)
struct NavigatorFilterTests {
    // MARK: - The board

    /// Created a few minutes ago, so all three are unread and Unread's New
    /// lists them newest first: bil-3, bil-2, bil-1. The statuses list them
    /// To Do first: bil-1, bil-2, then In Progress's bil-3.
    static let tasks: [(id: String, key: String, title: String, status: String, ago: Int64)] = [
        ("t1", "bil-1", "Invoice PDF", "todo", 300),
        ("t2", "bil-2", "Refund flow", "todo", 200),
        ("t3", "bil-3", "Tax rates", "in_progress", 100),
    ]

    static func store(defaults: UserDefaults) async -> TaskBoardStore {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        client.commandRunnerForTesting = { args in
            guard args.starts(with: ["task", "list"]) else { return (Data(), nil) }
            let rows = Self.tasks.map { t in
                let at = now - t.ago * 1000
                return #"{"id":"\#(t.id)","key":"\#(t.key)","title":"\#(t.title)","status":"\#(t.status)","status_since":\#(at),"created_at":\#(at),"updated_at":\#(at)}"#
            }
            return (Data(#"{"tasks":[\#(rows.joined(separator: ","))]}"#.utf8), nil)
        }
        let store = TaskBoardStore(
            client: client, workspace: .implicit(repository: "r"), readStore: DefaultsBoardReads(defaults))
        await store.readIfNeverRead()
        return store
    }

    /// What the window would hand the board: the task selected, and the
    /// row it lights.
    @MainActor
    final class Level: ObservableObject {
        @Published var selected: String?
        var current: NavigatorItem? { selected.map(NavigatorItem.task) }
    }

    final class Seen { var sections: Set<String> = [] }

    struct Hosted: View {
        @ObservedObject var level: Level
        let store: TaskBoardStore
        let defaults: UserDefaults
        let seen: Seen
        let slowdown: Double
        let onStep: (NavigatorItem) -> Void

        var body: some View {
            TaskBoardView(
                store: store, client: store.client, agents: .none, onGoTo: { _ in }, defaults: defaults,
                selected: level.selected, hasKeyboard: true, current: level.current, onStep: onStep)
            .environment(\.boardMotionSlowdown, slowdown)
            .frame(width: Harness.width, height: Harness.height, alignment: .topLeading)
            .background(WorkspaceStyle.canvas)
            .onPreferenceChange(CollapsibleSectionsKey.self) { ids in MainActor.assumeIsolated { seen.sections = ids } }
        }
    }

    /// A window that can take the keyboard, so typing and arrows reach it.
    final class KeyWindow: NSWindow {
        override var canBecomeKey: Bool { true }
        override var canBecomeMain: Bool { true }
    }

    @MainActor
    final class Harness {
        static let width: CGFloat = 300
        static let height: CGFloat = 900
        let level = Level()
        let seen = Seen()
        let store: TaskBoardStore
        let defaults: UserDefaults
        let host: NSHostingView<Hosted>
        let window: KeyWindow
        /// The rows ↑ and ↓ sent the window to, in order.
        var stepped: [NavigatorItem] = []

        init(slowdown: Double = 1) async {
            defaults = UserDefaults(suiteName: "ov177-\(UUID().uuidString)")!
            store = await NavigatorFilterTests.store(defaults: defaults)
            var onStep: (NavigatorItem) -> Void = { _ in }
            host = NSHostingView(
                rootView: Hosted(
                    level: level, store: store, defaults: defaults, seen: seen, slowdown: slowdown,
                    onStep: { onStep($0) }))
            window = KeyWindow(
                contentRect: NSRect(x: -4000, y: -4000, width: Self.width, height: Self.height),
                styleMask: [.borderless], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = host
            window.makeKeyAndOrderFront(nil)
            onStep = { [unowned self] item in
                self.stepped.append(item)
                Task { @MainActor in await self.select(item.taskID) }
            }
        }

        /// The window selects `id`, and a moment later its record's read
        /// lands, as opening it does (`TaskBoardStore.read`, after a `task
        /// show`).
        func select(_ id: String?) async {
            level.selected = id
            await settle(2)
            if let id, let row = store.board.rows.first(where: { $0.id == id }) { store.markRead(row) }
        }

        func settle(_ frames: Int = 10) async {
            for _ in 0..<frames {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }

        /// Every accessibility identifier drawn.
        var identifiers: Set<String> {
            var found: Set<String> = []
            func walk(_ element: Any, _ depth: Int) {
                guard depth < 80, let node = element as? NSAccessibilityProtocol else { return }
                if let id = node.accessibilityIdentifier(), !id.isEmpty { found.insert(id) }
                for child in node.accessibilityChildren() ?? [] { walk(child, depth + 1) }
            }
            walk(host, 0)
            return found
        }

        /// Every accessibility label drawn.
        var labels: [String] {
            var found: [String] = []
            func walk(_ element: Any, _ depth: Int) {
                guard depth < 80, let node = element as? NSAccessibilityProtocol else { return }
                if let label = node.accessibilityLabel(), !label.isEmpty { found.append(label) }
                if let value = node.accessibilityValue() as? String, !value.isEmpty { found.append(value) }
                for child in node.accessibilityChildren() ?? [] { walk(child, depth + 1) }
            }
            walk(host, 0)
            return found
        }

        /// Press the element `identifier` names, as VoiceOver or a click does.
        @discardableResult
        func press(_ identifier: String) -> Bool {
            func walk(_ element: Any, _ depth: Int) -> Bool {
                guard depth < 80, let node = element as? NSAccessibilityProtocol else { return false }
                if node.accessibilityIdentifier() == identifier { return node.accessibilityPerformPress() }
                return (node.accessibilityChildren() ?? []).contains { walk($0, depth + 1) }
            }
            return walk(host, 0)
        }

        /// The navigator's filter field.
        var field: NSTextField? {
            func find(_ view: NSView) -> NSTextField? {
                if let field = view as? NSTextField, field.isEditable, field.placeholderString == "Filter" {
                    return field
                }
                for sub in view.subviews { if let found = find(sub) { return found } }
                return nil
            }
            return find(host)
        }

        /// Type `text` into the filter, as the keyboard would: into its
        /// field editor, replacing what's there.
        func type(_ text: String) {
            guard let field else {
                Issue.record("no filter field drawn")
                return
            }
            window.makeFirstResponder(field)
            guard let editor = field.currentEditor() as? NSTextView else {
                Issue.record("the filter field took no editor")
                return
            }
            editor.selectAll(nil)
            editor.insertText(text, replacementRange: editor.selectedRange())
        }

        /// ↓ (1) or ↑ (-1), through the app as the keyboard sends it, so the
        /// navigator's monitor hears it. The list has the keyboard first.
        func arrow(_ by: Int) {
            window.makeFirstResponder(nil)
            let down = by > 0
            for type in [NSEvent.EventType.keyDown, .keyUp] {
                let event = NSEvent.keyEvent(
                    with: type, location: .zero, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                    windowNumber: window.windowNumber, context: nil,
                    characters: String(Character(UnicodeScalar(down ? 0xF701 : 0xF700)!)),
                    charactersIgnoringModifiers: String(Character(UnicodeScalar(down ? 0xF701 : 0xF700)!)),
                    isARepeat: false, keyCode: down ? 125 : 126)!
                NSApp.sendEvent(event)
            }
        }

        /// The list below the filter field, as drawn now, its pixels.
        func listPixels() -> Data {
            let bounds = host.bounds
            let rep = host.bitmapImageRepForCachingDisplay(in: bounds)!
            host.cacheDisplay(in: bounds, to: rep)
            // From under the header, its divider and the field, down: the
            // field's caret blinks.
            let fieldFoot = ColumnHeader.height + 1 + ColumnGrid.rhythm + ColumnGrid.rowHeight + 2
            let top = Int(fieldFoot * CGFloat(rep.pixelsHigh) / bounds.height)
            let bytes = rep.bytesPerRow
            let data = Data(bytes: rep.bitmapData!, count: bytes * rep.pixelsHigh)
            return data.subdata(in: (top * bytes)..<data.count)
        }

        func close() {
            window.close()
        }
    }

    // MARK: - Filtering

    /// Typing doesn't move the list (owner, ov-177: "the list animates,
    /// including selection states, which is weird"). On a spring slowed
    /// twentyfold, a narrowing that moved would still be moving 0.15 s and
    /// 0.6 s after the keystroke, and those two frames would differ. (Fails
    /// with `unanimatedWhenFiltering` taken off the list: the rows, the
    /// sections and the selected row's highlight slide.)
    @Test("The filter narrows the list without moving it")
    func filteringDoesNotAnimate() async {
        let harness = await Harness(slowdown: 20)
        defer { harness.close() }
        harness.level.selected = "t2"
        await harness.settle(20)
        harness.type("refund")
        try? await Task.sleep(for: .milliseconds(150))
        harness.host.layoutSubtreeIfNeeded()
        let soon = harness.listPixels()
        try? await Task.sleep(for: .milliseconds(450))
        harness.host.layoutSubtreeIfNeeded()
        let later = harness.listPixels()
        #expect(harness.identifiers.contains("board-row-bil-2"))
        #expect(!harness.identifiers.contains("board-row-bil-1"), "the filter never took")
        #expect(soon == later, "the list was still moving after the filter changed")
    }

    /// A status with no match isn't drawn, nor Unread with none; a status
    /// with one is, open. (Fails on ov-104's navigator, which kept every
    /// status as a header reading 0 and Unread saying "You're all caught
    /// up.".)
    @Test("While filtering, only the sections with a match are drawn")
    func onlyMatchingSectionsAreDrawn() async {
        let harness = await Harness()
        defer { harness.close() }
        await harness.settle()
        #expect(harness.seen.sections.contains("status.backlog"), "unfiltered, every status is drawn")
        harness.store.markAllRead()
        harness.type("tax")
        await harness.settle()
        let sections = harness.seen.sections
        #expect(sections.contains("tasks"))
        #expect(sections.contains("status.in_progress"))
        for status in TaskBoardModel.order where status != .inProgress {
            #expect(!sections.contains("status.\(status.rawValue)"), "\(status) has no match, and was drawn")
        }
        // Everything was read: Unread has no match, and isn't drawn.
        #expect(!sections.contains("summary"))
        #expect(!harness.identifiers.contains("board-summary-empty"))
        #expect(harness.identifiers.contains("board-row-bil-3"))
    }

    /// Nothing matches: one "No Results for …", and nothing else, least of
    /// all "You're all caught up.". (Fails on ov-104's navigator.)
    @Test("With nothing matching, the navigator says No Results once")
    func noResults() async {
        let harness = await Harness()
        defer { harness.close() }
        await harness.settle()
        #expect(!harness.identifiers.contains("navigator-no-results"))
        harness.type("testso")
        await harness.settle()
        let ids = harness.identifiers
        #expect(ids.contains("navigator-no-results"))
        #expect(!ids.contains("board-summary-empty"), "said it was all caught up while filtering")
        #expect(harness.seen.sections.isEmpty, "drew \(harness.seen.sections) with nothing matching")
        #expect(harness.labels.contains { $0.contains("No Results for") && $0.contains("testso") })
        // Cleared, it's all back, and Unread is Unread again.
        harness.type("")
        await harness.settle()
        #expect(!harness.identifiers.contains("navigator-no-results"))
        #expect(harness.seen.sections.contains("summary"))
    }

    // MARK: - Unread keeps the selected task's line

    /// Opened from Unread, a task's line stays there, lit, until the
    /// selection moves to another task, and then it leaves. (Fails on
    /// ov-104's navigator, where opening it read it and took the line away
    /// at once.)
    @Test("An opened Unread line stays while its task is selected")
    func anOpenedLineStays() async {
        let harness = await Harness()
        defer { harness.close() }
        await harness.settle()
        #expect(harness.identifiers.contains("board-summary-item-t2/created"))
        // Chosen from Unread: the window selects it, and its read lands.
        #expect(harness.press("board-summary-item-t2/created"), "couldn't press the line")
        await harness.select("t2")
        await harness.settle()
        #expect(!harness.store.reads.isUnread("t2", at: Date().addingTimeInterval(-200)), "it was never read")
        #expect(harness.identifiers.contains("board-summary-item-t2/created"), "the line left while selected")
        // Another task: the line leaves.
        await harness.select("t1")
        await harness.settle(40)
        #expect(!harness.identifiers.contains("board-summary-item-t2/created"), "the line stayed after the selection moved")
        // Nothing selected: t1's leaves too.
        await harness.select(nil)
        await harness.settle(40)
        #expect(!harness.identifiers.contains("board-summary-item-t1/created"))
    }

    /// ↓ walks Unread's lines in place, newest first, before the statuses:
    /// each opens its task, which keeps its line until the next ↓ moves on.
    /// (Fails on ov-104's navigator, whose walk was the statuses' rows only:
    /// its first ↓ went to bil-1, the first To Do, not bil-3.)
    @Test("↑ and ↓ go on from the Unread line the selection is on")
    func arrowsWalkUnread() async {
        let harness = await Harness()
        defer { harness.close() }
        await harness.settle()
        harness.arrow(1)
        await harness.settle()
        #expect(harness.stepped.last?.taskID == "t3")
        #expect(harness.identifiers.contains("board-summary-item-t3/created"), "the line left while selected")
        harness.arrow(1)
        await harness.settle(40)
        #expect(harness.stepped.map(\.taskID) == ["t3", "t2"])
        #expect(!harness.identifiers.contains("board-summary-item-t3/created"))
        #expect(harness.identifiers.contains("board-summary-item-t2/created"))
        // On from the line it's on, not from its status's row: Unread's
        // next, bil-1, where To Do's row would have gone on to bil-3.
        harness.arrow(1)
        await harness.settle(40)
        #expect(harness.stepped.map(\.taskID) == ["t3", "t2", "t1"])
    }

    // MARK: - Mark All as Read

    /// The header's button reads everything, but the task selected keeps
    /// its line until the selection moves on. (Fails on ov-104's Unread,
    /// which had no button.)
    @Test("Mark All as Read clears Unread but for the task selected")
    func markAllRead() async {
        let harness = await Harness()
        defer { harness.close() }
        await harness.select("t2")
        await harness.settle()
        #expect(harness.identifiers.contains("board-mark-all-read"))
        #expect(harness.press("board-mark-all-read"), "no Mark All as Read on Unread's header")
        await harness.settle(40)
        let ids = harness.identifiers
        #expect(!ids.contains("board-summary-item-t1/created"))
        #expect(!ids.contains("board-summary-item-t3/created"))
        #expect(ids.contains("board-summary-item-t2/created"), "the selected task's line went with the rest")
        await harness.select(nil)
        await harness.settle(40)
        #expect(harness.identifiers.contains("board-summary-empty"))
        #expect(!harness.identifiers.contains("board-mark-all-read"), "offered with nothing to read")
    }
}
