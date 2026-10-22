import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Text selection in the conversation (ov-693): a row on screen at rest
/// offers it, one that comes on screen mid-scroll is drawn without it, and
/// offers it once the scroll ends. Selected text copies.
@MainActor
@Suite(.serialized)
struct NativeSelectionTests {
    typealias T = NativeAgentTests

    static let terminal = "0199aaaa-0000-7000-8000-000000000693"

    /// A session of sixty real rows at its end, in a 720 by 900 window.
    static func session() async throws -> (NSWindow, NSScrollView) {
        let model = T.model(try T.terminal(id: terminal))
        let window = NativePolishTests.window(width: 720, height: 900, NativeAgentView(model: model, isFocused: true, showTerminal: {}))
        model.store.apply(try await model.store.ledger.page(T.page(RealSession.rows(60))))
        await T.settle(window, 400)  // capture-exempt: not a capture; the session drawn before its text is read
        let content = try #require(window.contentView)
        return (window, try #require(Fling.scrollView(in: content), "no scroll view"))
    }

    /// The views a selectable `Text` puts beside its drawing, on screen: a
    /// row off it may keep its own.
    static func selectable(in window: NSWindow) -> [NSView] {
        var found: [NSView] = []
        func walk(_ view: NSView) {
            if view.className.contains("TextInteraction"), !view.isHiddenOrHasHiddenAncestor, !view.visibleRect.isEmpty {
                found.append(view)
            }
            view.subviews.forEach(walk)
        }
        window.contentView.map(walk)
        return found
    }

    /// What selecting all of `view`'s text and copying it puts on a
    /// pasteboard of the test's own, never the reader's.
    static func copied(_ view: NSView) -> String? {
        view.perform(#selector(NSResponder.selectAll(_:)), with: nil)
        let board = NSPasteboard(name: NSPasteboard.Name("fc-selection-test-\(UUID().uuidString)"))
        defer { board.releaseGlobally() }
        board.clearContents()
        // What Copy and the Services menu ask a text view for.
        let wrote = (view as AnyObject).writeSelection?(to: board, types: [.string])
        #expect(wrote == true, "\(view.className) didn't write its selection")
        return board.string(forType: .string)
    }

    @Test("A row on screen at rest selects and copies its words")
    func restingRowsSelect() async throws {
        let (window, _) = try await Self.session()
        defer { window.close() }
        let views = Self.selectable(in: window)
        #expect(!views.isEmpty, "no text on screen offers selection")
        let words = views.compactMap(Self.copied)
        // The newest rows' words: answer 58's paragraphs, line 59.
        #expect(words.contains { $0.contains("of answer 58: the list is lazy") }, "copied: \(words)")
        #expect(words.contains("Line 59."), "copied: \(words)")
    }

    @Test("Rows that come on screen mid-scroll are drawn without selection, and offer it once the scroll ends")
    func scrollingRowsSelectOnceAtRest() async throws {
        let (window, scroll) = try await Self.session()
        defer { window.close() }
        LiveScroll.watch()
        // A live scroll, as a trackpad's: begun, moved a few screens, and
        // still in flight.
        NotificationCenter.default.post(name: NSScrollView.willStartLiveScrollNotification, object: scroll)
        #expect(LiveScroll.inFlight)
        let origin = scroll.contentView.bounds.origin
        scroll.contentView.scroll(to: NSPoint(x: origin.x, y: max(0, origin.y - 3_000)))
        scroll.reflectScrolledClipView(scroll.contentView)
        await T.settle(window, 300)  // capture-exempt: not a capture; the rows above drawn
        #expect(Self.selectable(in: window).isEmpty, "rows drawn mid-scroll offer selection: \(origin.y) > \(scroll.contentView.bounds.origin.y) \(Self.selectable(in: window).map { Self.copied($0) ?? "" })")

        NotificationCenter.default.post(name: NSScrollView.didEndLiveScrollNotification, object: scroll)
        #expect(!LiveScroll.inFlight)
        await T.settle(window, 300)  // capture-exempt: not a capture; the rows turn selection on
        let views = Self.selectable(in: window)
        #expect(!views.isEmpty, "the scroll ended and no row offers selection")
        let words = views.compactMap(Self.copied)
        #expect(words.contains { !$0.isEmpty }, "nothing copied")
    }
}
