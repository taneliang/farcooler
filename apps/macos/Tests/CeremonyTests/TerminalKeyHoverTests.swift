import AgentKit
import AppKit
import Foundation
import Testing

@testable import Far_Cooler

/// A task key's hovercard in a terminal (ov-299): output fed through the real
/// emulator, the pointer moved by real mouse events into the view's own
/// handler, and the key found by the link hit-testing ⌘-click uses.
@MainActor
struct TerminalKeyHoverTests {
    /// A pane in an offscreen window showing `text`, whose linker knows ov-190
    /// (with a card) and ov-7 (linked, but its card was never built).
    static func pane(_ text: String) -> (view: TerminalRenderView, window: NSWindow) {
        let window = NSWindow(
            contentRect: CGRect(x: -10_000, y: -10_000, width: 900, height: 300), styleMask: [.titled],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let view = TerminalRenderView()
        view.frame = CGRect(x: 0, y: 0, width: 900, height: 300)
        window.contentView?.addSubview(view)
        view.layoutSubtreeIfNeeded()
        view.setPaneGrid(PaneGrid(columns: 50, rows: 6))
        view.feed(Array(text.utf8))
        let row = TaskRow(id: "t190", key: "ov-190", title: "Fix the login", status: .inReview, statusSince: .now)
        let board = TaskBoardModel(columns: [TaskBoardColumn(status: .inReview, rows: [row])])
        let targets = ["ov-190": "t190", "ov-7": "t7"].map {
            ($0.key, TaskKeyTarget(runner: "", workspace: "w", task: $0.value, key: $0.key))
        }
        view.taskKeyLinker = TaskKeyLinker(
            index: TaskKeyIndex(runner: "", prefixes: ["ov"], targets: Dictionary(uniqueKeysWithValues: targets)),
            cards: TaskKeyCards(runner: "", boards: ["w": board])
        ) { _ in }
        view.keyHover.delay = .zero
        return (view, window)
    }

    /// A plain move over cell (`row`, `column`): no ⌘, as a person reading.
    static func move(_ view: TerminalRenderView, row: Int, column: Int) throws {
        let cell = TerminalMetrics.cell(Preferences.shared.terminalFont())
        let pad = TerminalMetrics.padding
        let point = view.convert(
            CGPoint(
                x: pad.left + (CGFloat(column) + 0.5) * cell.width, y: pad.top + (CGFloat(row) + 0.5) * cell.height),
            to: nil)
        let event = try #require(
            NSEvent.mouseEvent(
                with: .mouseMoved, location: point, modifierFlags: [], timestamp: 0,
                windowNumber: view.window?.windowNumber ?? 0, context: nil, eventNumber: 0, clickCount: 0, pressure: 0))
        view.mouseMoved(with: event)
    }

    /// Let a zero-delay wait run out, however loaded the machine: until the
    /// pane has no card waiting, or two seconds.
    static func settle(_ view: TerminalRenderView) async {
        for _ in 0..<200 {
            await Task.yield()
            if !view.keyHover.isWaiting { return }
            try? await Task.sleep(for: .milliseconds(10))
        }
    }

    @Test("Resting on a key in output shows its card, from any of its cells, and moving off closes it")
    func hoverShowsTheCard() async throws {
        let (view, window) = Self.pane("\u{1b}[1mdone\u{1b}[0m in ov-190.\r\nnext")
        defer { window.close() }
        // "done in " is eight cells; the key is columns 8...13.
        for column in [8, 13] {
            try Self.move(view, row: 0, column: column)
            await Self.settle(view)
            #expect(view.keyHover.shown?.key == "ov-190", "column \(column)")
            #expect(view.keyHover.shown?.title == "Fix the login")
        }
        try Self.move(view, row: 0, column: 14)
        await Self.settle(view)
        #expect(view.keyHover.shown == nil, "the full stop after it isn't the key")
        try Self.move(view, row: 1, column: 1)
        await Self.settle(view)
        #expect(view.keyHover.shown == nil)
    }

    @Test("Leaving the pane closes the card")
    func exitCloses() async throws {
        let (view, window) = Self.pane("ov-190")
        defer { window.close() }
        try Self.move(view, row: 0, column: 2)
        await Self.settle(view)
        #expect(view.keyHover.shown != nil)
        view.mouseExited(with: try #require(NSEvent.enterExitEvent(
            with: .mouseExited, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
            eventNumber: 0, trackingNumber: 0, userData: nil)))
        #expect(view.keyHover.shown == nil)
    }

    @Test("A key without a card, an unknown key and a key inside a URL show nothing")
    func nothingElse() async throws {
        let (view, window) = Self.pane("ov-7 ov-191 https://x.dev/ov-190")
        defer { window.close() }
        for column in [1, 7, 28] {
            try Self.move(view, row: 0, column: column)
            await Self.settle(view)
            #expect(view.keyHover.shown == nil, "column \(column)")
        }
    }

    @Test("The card waits for the hover delay, and moving off first cancels the wait")
    func waitsForTheDelay() async throws {
        let (view, window) = Self.pane("ov-190 x")
        defer { window.close() }
        // Longer than any loaded machine takes to settle, so "not yet" holds.
        view.keyHover.delay = .seconds(60)
        try Self.move(view, row: 0, column: 2)
        await Self.settle(view)
        #expect(view.keyHover.shown == nil, "not before the delay")
        #expect(view.keyHover.isWaiting)
        try Self.move(view, row: 0, column: 7)
        #expect(!view.keyHover.isWaiting, "moving off cancels the wait")
        #expect(view.keyHover.key == nil)
        view.keyHover.delay = .zero
        try Self.move(view, row: 0, column: 2)
        await Self.settle(view)
        #expect(view.keyHover.shown?.key == "ov-190")
    }

    @Test("The window's linker gives every key the card of what its boards read")
    func macLinkerCarriesCards() {
        let cache = TaskKeyCardCache()
        let linker = TaskKeyLinker.mac(host: "h", workspaces: [], stores: [TaskBoardStore](), cards: cache) { _, _, _ in }
        #expect(linker.cards.runner == "h")
        #expect(cache.builds == 1)
        _ = TaskKeyLinker.mac(host: "h", workspaces: [], stores: [TaskBoardStore](), cards: cache) { _, _, _ in }
        #expect(cache.builds == 1, "the same reads, the same cards")
    }
}
