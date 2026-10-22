import AgentKit
import AppKit
import SwiftUI
import TestWindows
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
    static func session(probed frames: NativePolishTests.Frames = .init()) async throws -> (NSWindow, NSScrollView) {
        let model = T.model(try T.terminal(id: terminal))
        let view = NativeAgentView(model: model, isFocused: true, showTerminal: {})
        let window = NativePolishTests.window(width: 720, height: 900, NativePolishTests.Framed(frames: frames, content: view))
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

    @Test("Copying a list at rest keeps its bullets and numbers")
    func listsCopyWithTheirMarkers() async throws {
        let text = "Steps:\n\n- first point\n- second point\n\n1. one thing\n2. another thing"
        let window = NativePolishTests.window(width: 720, height: 600, MarkdownText(text: text))
        defer { window.close() }
        await T.settle(window, 300)  // capture-exempt: not a capture; the list drawn before its text is read
        // Read top to bottom, a row's marker then its words, as a copy of the
        // list would put them.
        let views = Self.selectable(in: window).sorted {
            let (a, b) = ($0.convert($0.bounds, to: nil), $1.convert($1.bounds, to: nil))
            return abs(a.midY - b.midY) > 4 ? a.midY > b.midY : a.minX < b.minX
        }
        let words = views.compactMap(Self.copied)
        // Each marker is a `Text` of its own beside its item's words, so it
        // must offer selection itself or a copied list loses it (ov-700).
        #expect(words == ["Steps:", "•", "first point", "•", "second point", "1.", "one thing", "2.", "another thing"], "copied in order: \(words)")
    }

    @Test("The caption over clipped output copies, at rest")
    func theTruncationCaptionCopies() async throws {
        let long = (1...450).map { "line \($0)" }.joined(separator: "\n")
        let window = NativePolishTests.window(width: 720, height: 900, DetailBox(text: long))
        defer { window.close() }
        await T.settle(window, 300)  // capture-exempt: not a capture; the box drawn before its text is read
        let words = Self.selectable(in: window).compactMap(Self.copied)
        #expect(words.contains { $0.hasPrefix("Showing the last") }, "caption not selectable: \(words)")
    }

    @Test("A live scroll in a pane that's put away holds no row's selection back")
    func aHiddenPanesScrollIsOver() async throws {
        let (window, scroll) = try await Self.session()
        defer { window.close() }
        LiveScroll.watch()
        let other = NSWindow(contentRect: NSRect(x: -9000, y: -9000, width: 400, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
        other.isReleasedWhenClosed = false
        defer { other.close() }
        let pane = NSView(frame: NSRect(x: 0, y: 0, width: 400, height: 400))
        let stuck = NSScrollView(frame: NSRect(x: 0, y: 0, width: 200, height: 200))
        pane.addSubview(stuck)
        other.contentView = pane
        TestWindow.show(other)
        // Begun and never ended, in a pane that's on screen and then hidden.
        NotificationCenter.default.post(name: NSScrollView.willStartLiveScrollNotification, object: stuck)
        defer { NotificationCenter.default.post(name: NSScrollView.didEndLiveScrollNotification, object: stuck) }
        #expect(LiveScroll.inFlight, "a scroll in a visible pane is in flight")
        pane.isHidden = true
        #expect(stuck.isHiddenOrHasHiddenAncestor)
        #expect(other.isVisible, "the window stays up: only the pane is put away")
        #expect(!LiveScroll.inFlight, "a hidden pane's scroll still holds selection back")

        let origin = scroll.contentView.bounds.origin
        scroll.contentView.scroll(to: NSPoint(x: origin.x, y: max(0, origin.y - 3_000)))
        scroll.reflectScrolledClipView(scroll.contentView)
        await T.settle(window, 300)  // capture-exempt: not a capture; the rows above drawn
        #expect(!Self.selectable(in: window).isEmpty, "rows drawn at rest offer no selection")
    }

    /// Where each row on screen is drawn, by id.
    static func shown(_ frames: NativePolishTests.Frames) -> [String: CGRect] {
        frames.views.filter { $0.key.hasPrefix("native-row-") && $0.value.maxY > 0 && $0.value.minY < 900 }
    }

    @Test("Rows that come on screen mid-scroll are drawn without selection, and offer it once the scroll ends, staying where they are")
    func scrollingRowsSelectOnceAtRest() async throws {
        let frames = NativePolishTests.Frames()
        let (window, scroll) = try await Self.session(probed: frames)
        defer { window.close() }
        LiveScroll.watch()
        // A live scroll, as a trackpad's: begun, moved a few screens, and
        // still in flight. Ended whatever happens, so no other suite's rows
        // come up without selection.
        NotificationCenter.default.post(name: NSScrollView.willStartLiveScrollNotification, object: scroll)
        defer { NotificationCenter.default.post(name: NSScrollView.didEndLiveScrollNotification, object: scroll) }
        #expect(LiveScroll.inFlight)
        let origin = scroll.contentView.bounds.origin
        scroll.contentView.scroll(to: NSPoint(x: origin.x, y: max(0, origin.y - 3_000)))
        scroll.reflectScrolledClipView(scroll.contentView)
        await T.settle(window, 300)  // capture-exempt: not a capture; the rows above drawn
        #expect(Self.selectable(in: window).isEmpty, "rows drawn mid-scroll offer selection: \(origin.y) > \(scroll.contentView.bounds.origin.y) \(Self.selectable(in: window).map { Self.copied($0) ?? "" })")

        // Answers with lists, code and a table among them.
        let before = Self.shown(frames)
        #expect(before.keys.contains { ["5", "15", "25", "35", "45"].contains($0.split(separator: ":").last.map(String.init) ?? "") }, "no answer with a list, code and a table on screen: \(before.keys.sorted())")

        NotificationCenter.default.post(name: NSScrollView.didEndLiveScrollNotification, object: scroll)
        #expect(!LiveScroll.inFlight)
        await T.settle(window, 300)  // capture-exempt: not a capture; the rows turn selection on
        let views = Self.selectable(in: window)
        #expect(!views.isEmpty, "the scroll ended and no row offers selection")
        let words = views.compactMap(Self.copied)
        #expect(words.contains { !$0.isEmpty }, "nothing copied")
        // Selection coming on moves nothing: no row on screen changes its
        // size or its place as the reader stops to read.
        let after = Self.shown(frames)
        #expect(after == before, "rows moved as selection came on: \(before.filter { after[$0.key] != $0.value }.map { "\($0.key) \($0.value) > \(after[$0.key].map { "\($0)" } ?? "gone")" })")
    }

    @Test("A live scroll in a window that's gone holds no row's selection back")
    func aClosedWindowsScrollIsOver() async throws {
        let (window, scroll) = try await Self.session()
        defer { window.close() }
        LiveScroll.watch()
        let other = NSWindow(contentRect: NSRect(x: -9000, y: -9000, width: 200, height: 200), styleMask: [.titled], backing: .buffered, defer: false)
        other.isReleasedWhenClosed = false
        let stuck = NSScrollView(frame: NSRect(x: 0, y: 0, width: 200, height: 200))
        other.contentView = stuck
        TestWindow.show(other)
        // Begun and never ended, as a scroll can be when its pane goes.
        NotificationCenter.default.post(name: NSScrollView.willStartLiveScrollNotification, object: stuck)
        defer { NotificationCenter.default.post(name: NSScrollView.didEndLiveScrollNotification, object: stuck) }
        #expect(LiveScroll.inFlight, "a scroll on screen is in flight")
        other.close()
        #expect(!LiveScroll.inFlight, "a closed window's scroll still holds selection back")

        // So the rows a scroll here brings on screen at rest select.
        let origin = scroll.contentView.bounds.origin
        scroll.contentView.scroll(to: NSPoint(x: origin.x, y: max(0, origin.y - 3_000)))
        scroll.reflectScrolledClipView(scroll.contentView)
        await T.settle(window, 300)  // capture-exempt: not a capture; the rows above drawn
        #expect(!Self.selectable(in: window).isEmpty, "rows drawn at rest offer no selection")
    }
}
