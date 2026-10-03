import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Mark as Read asks first (ov-210, the owner: "too easy to hit by accident,
/// especially the keyboard shortcut").
///
/// - Every way in asks, and reads nothing until the answer is yes: Unread's
///   header button, filtered or not, its VoiceOver action, its context menu,
///   and Board ▸ Mark All as Read (⇧⌘K).
/// - The alert says what and how many, in Apple's words, and Return can't
///   confirm it; Cancel and Esc read nothing.
@MainActor
@Suite(.serialized)
struct MarkReadConfirmationTests {
    typealias Harness = NavigatorFilterTests.Harness

    /// Unread's three lines, one per task, as the harness's board has them.
    static let lines = ["board-summary-item-t1/created", "board-summary-item-t2/created", "board-summary-item-t3/created"]

    /// Whether every line of the board's Unread is still drawn.
    static func allUnread(_ harness: Harness) -> Bool {
        Set(lines).isSubset(of: harness.identifiers)
    }

    // MARK: - Every way in asks

    /// The header's button asks, reads nothing on Cancel, and reads all on
    /// Mark as Read. (Fails with `markRead` reading straight away, as it did
    /// before ov-210.)
    @Test("Unread's header button asks before it reads")
    func headerButtonAsks() async {
        let harness = await Harness()
        defer { harness.close() }
        harness.marks.answer = nil
        await harness.settle()
        harness.hover("section-header-summary")
        await harness.settle()
        #expect(harness.press("board-mark-all-read"), "no Mark All as Read on Unread's header")
        await harness.settle(20)
        #expect(harness.marks.asked == [MarkReadRequest(filtering: false, tasks: 3)])
        #expect(Self.allUnread(harness), "read before the person answered")
        harness.marks.reply(false)
        await harness.settle(20)
        #expect(Self.allUnread(harness), "read on Cancel")
        harness.hover("section-header-summary")
        await harness.settle()
        harness.press("board-mark-all-read")
        await harness.settle(5)
        harness.marks.reply(true)
        await harness.settle(40)
        #expect(harness.identifiers.contains("board-summary-empty"), "Mark as Read read nothing")
    }

    /// Filtered, it asks "Mark These as Read?" about the one task listed.
    /// (Fails with the filtered path reading straight away.)
    @Test("Filtered, Mark These as Read asks about the tasks listed")
    func filteredButtonAsks() async {
        let harness = await Harness()
        defer { harness.close() }
        harness.marks.answer = nil
        await harness.settle()
        harness.type("refund")
        await harness.settle()
        harness.hover("section-header-summary")
        await harness.settle()
        #expect(harness.press("board-mark-all-read"), "no button on the filtered strip")
        await harness.settle(20)
        #expect(harness.marks.asked == [MarkReadRequest(filtering: true, tasks: 1)])
        #expect(harness.marks.asked.first?.title == "Mark These as Read?")
        #expect(harness.identifiers.contains("board-summary-item-t2/created"), "read before the person answered")
        harness.marks.reply(false)
        harness.type("")
        await harness.settle(40)
        #expect(Self.allUnread(harness), "read on Cancel")
    }

    /// The header's VoiceOver action is Mark All as Read, and it asks too.
    /// (Fails with the header offering no action, and with the action
    /// reading straight away.)
    @Test("Unread's VoiceOver action asks before it reads")
    func voiceOverActionAsks() async throws {
        let harness = await Harness()
        defer { harness.close() }
        harness.marks.answer = nil
        await harness.settle()
        let header = try #require(Self.element("section-summary", in: harness.host), "no Unread header for VoiceOver")
        let action = try #require(
            header.accessibilityCustomActions()?.first { $0.name == "Mark All as Read" },
            "Unread's header offers no Mark All as Read to VoiceOver")
        Self.perform(action)
        await harness.settle(20)
        #expect(harness.marks.asked == [MarkReadRequest(filtering: false, tasks: 3)])
        #expect(Self.allUnread(harness), "read before the person answered")
        harness.marks.reply(true)
        await harness.settle(40)
        #expect(harness.identifiers.contains("board-summary-empty"), "Mark as Read read nothing")
    }

    /// Unread's context menu asks too. (Fails with the menu item reading
    /// straight away.)
    @Test("Unread's context menu asks before it reads")
    func contextMenuAsks() async throws {
        let harness = await Harness()
        defer { harness.close() }
        harness.marks.answer = nil
        await harness.settle()
        let chosen = Self.choose("Mark All as Read", inContextMenuOf: "section-header-summary", harness)
        #expect(chosen, "no Mark All as Read in Unread's context menu")
        await harness.settle(20)
        #expect(harness.marks.asked == [MarkReadRequest(filtering: false, tasks: 3)])
        #expect(Self.allUnread(harness), "read before the person answered")
        harness.marks.reply(false)
        await harness.settle(20)
        #expect(Self.allUnread(harness), "read on Cancel")
    }

    /// Board ▸ Mark All as Read (⇧⌘K) asks about every task Unread lists,
    /// filtered or not. The window's command reaches the store only through
    /// `askToMarkAllRead`: `markAllRead` takes a `MarkReadGrant`, which
    /// only a confirmation hands out.
    @Test("⇧⌘K asks before it reads")
    func shortcutAsks() async {
        let harness = await Harness()
        defer { harness.close() }
        harness.marks.answer = nil
        await harness.settle()
        harness.type("refund")
        await harness.settle()
        harness.store.askToMarkAllRead(harness.marks.confirmation)
        await harness.settle(5)
        #expect(harness.marks.asked == [MarkReadRequest(filtering: false, tasks: 3)])
        harness.marks.reply(false)
        harness.type("")
        await harness.settle(40)
        #expect(Self.allUnread(harness), "read on Cancel")
        harness.store.askToMarkAllRead(harness.marks.confirmation)
        harness.marks.reply(true)
        await harness.settle(40)
        #expect(harness.identifiers.contains("board-summary-empty"), "Mark as Read read nothing")
    }

    /// With nothing unread there's nothing to ask about.
    @Test("Nothing unread, nothing asked")
    func nothingToAsk() async {
        let marks = AskedToMarkRead()
        let performed = Flag()
        marks.confirmation.confirm(MarkReadRequest(filtering: false, tasks: 0)) { _ in performed.value = true }
        #expect(marks.asked.isEmpty)
        #expect(!performed.value)
    }

    // MARK: - The alert

    /// Apple's words: the question, the count, Cancel to the left of Mark
    /// as Read, and no default button, so Return straight after ⇧⌘K can't
    /// confirm. (Fails on NSAlert's own defaults, which give Mark as Read
    /// Return.)
    @Test("The alert's words and keys")
    func alertCopy() {
        let all = MarkReadConfirmation.makeAlert(MarkReadRequest(filtering: false, tasks: 68))
        #expect(all.messageText == "Mark All as Read?")
        #expect(all.informativeText == "68 tasks will be marked as read.")
        #expect(all.buttons.map(\.title) == ["Mark as Read", "Cancel"])
        #expect(all.buttons[0].keyEquivalent == "", "Return confirms")
        #expect(all.buttons[1].keyEquivalent == "\u{1b}", "Esc doesn't cancel")
        let one = MarkReadConfirmation.makeAlert(MarkReadRequest(filtering: true, tasks: 1))
        #expect(one.messageText == "Mark These as Read?")
        #expect(one.informativeText == "1 task will be marked as read.")
    }

    /// On the window, as the app shows it: Esc and Cancel close it reading
    /// nothing, Return does nothing, and Mark as Read reads. (Fails with
    /// Mark as Read keeping Return, or Cancel losing Esc.)
    @Test("Esc and Cancel read nothing; Return doesn't confirm")
    func alertKeys() async throws {
        let harness = await Harness()
        defer { harness.close() }
        await harness.settle()
        let window = NSWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: 480, height: 300), styleMask: [.titled],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.orderFront(nil)
        defer { window.close() }
        let shown = Shown()
        let confirmation = MarkReadConfirmation { request, done in
            shown.alert = MarkReadConfirmation.present(request, on: window, done: done)
        }

        for key in [Key.escape, Key.cancel] {
            harness.store.askToMarkAllRead(confirmation)
            await harness.settle(5)
            let sheet = try #require(shown.alert?.window, "no alert shown")
            if key == .escape {
                // Return first: with no default button, nothing happens.
                #expect(!sheet.performKeyEquivalent(with: Self.key("\r", code: 36, in: sheet)), "Return answered")
                await harness.settle(5)
                #expect(window.attachedSheet != nil, "Return closed the alert")
                #expect(sheet.performKeyEquivalent(with: Self.key("\u{1b}", code: 53, in: sheet)), "Esc did nothing")
            } else {
                shown.alert?.buttons[1].performClick(nil)
            }
            await harness.settle(20)
            #expect(window.attachedSheet == nil, "\(key) left the alert up")
            #expect(Self.allUnread(harness), "\(key) read")
        }

        harness.store.askToMarkAllRead(confirmation)
        await harness.settle(5)
        shown.alert?.buttons[0].performClick(nil)
        await harness.settle(40)
        #expect(harness.identifiers.contains("board-summary-empty"), "Mark as Read read nothing")
    }

    // MARK: - Helpers

    enum Key { case escape, cancel }

    @MainActor final class Shown { var alert: NSAlert? }
    @MainActor final class Flag { var value = false }

    static func key(_ characters: String, code: UInt16, in window: NSWindow) -> NSEvent {
        NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: window.windowNumber, context: nil, characters: characters,
            charactersIgnoringModifiers: characters, isARepeat: false, keyCode: code)!
    }

    /// The accessibility element whose identifier is `id`, under `root`.
    static func element(_ id: String, in root: NSAccessibilityProtocol) -> NSAccessibilityProtocol? {
        if root.accessibilityIdentifier() == id { return root }
        for child in root.accessibilityChildren() ?? [] {
            if let child = child as? NSAccessibilityProtocol, let found = element(id, in: child) { return found }
        }
        return nil
    }

    /// Perform `action` as VoiceOver's Actions rotor would.
    static func perform(_ action: NSAccessibilityCustomAction) {
        if let handler = action.handler {
            _ = handler()
        } else if let target = action.target as? NSObject, let selector = action.selector {
            target.perform(selector, with: action)
        }
    }

    /// Choose `title` from the context menu of the view `identifier` names,
    /// as a right click would. False when there's no such item.
    static func choose(_ title: String, inContextMenuOf identifier: String, _ harness: Harness) -> Bool {
        guard let frame = harness.seen.views[identifier] else { return false }
        let at = NSPoint(x: frame.midX, y: Harness.height - frame.midY)
        let click = NSEvent.mouseEvent(
            with: .rightMouseDown, location: at, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: harness.window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
        let target = harness.host.hitTest(at) ?? harness.host
        if let menu = target.menu(for: click) ?? harness.host.menu(for: click) {
            let index = menu.indexOfItem(withTitle: title)
            guard index >= 0 else { return false }
            menu.performActionForItem(at: index)
            return true
        }
        // Drawn by SwiftUI on the click itself: choose the item as the menu
        // opens, and close it.
        let found = Flag()
        let observer = NotificationCenter.default.addObserver(
            forName: NSMenu.didBeginTrackingNotification, object: nil, queue: nil
        ) { note in
            guard let menu = note.object as? NSMenu else { return }
            MainActor.assumeIsolated {
                let index = menu.indexOfItem(withTitle: title)
                RunLoop.main.perform(inModes: [.common, .eventTracking]) {
                    MainActor.assumeIsolated {
                        if index >= 0 {
                            found.value = true
                            menu.performActionForItem(at: index)
                        }
                        menu.cancelTracking()
                    }
                }
            }
        }
        defer { NotificationCenter.default.removeObserver(observer) }
        harness.window.sendEvent(click)
        return found.value
    }
}

/// Mark as Read's questions, and the person's answers: yes straight away by
/// default; with `answer` nil, held until `reply`.
@MainActor
final class AskedToMarkRead {
    var asked: [MarkReadRequest] = []
    var answer: Bool? = true
    private var waiting: [@MainActor (Bool) -> Void] = []

    var confirmation: MarkReadConfirmation {
        MarkReadConfirmation { [unowned self] request, done in
            asked.append(request)
            if let answer { done(answer) } else { waiting.append(done) }
        }
    }

    /// Answer every question still open.
    func reply(_ yes: Bool) {
        let open = waiting
        waiting = []
        for done in open { done(yes) }
    }
}

extension MarkReadConfirmation {
    /// Yes, every time: for tests about what Mark All as Read reads.
    static var granting: MarkReadConfirmation { MarkReadConfirmation { _, done in done(true) } }
}
