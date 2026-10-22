import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The transcript's own scroller (ov-701): the system's `NSScroller`, its
/// knob placed from our heights rather than the lazy stack's guess, and
/// driven as the system's is.
///
/// Inside `NativeSelectionTests`, so neither runs beside the other: these
/// scroll live in a window on screen, which holds back the selection those
/// read (`LiveScroll.inFlight`).
extension NativeSelectionTests {
    @MainActor
    @Suite(.serialized)
    struct TranscriptScrollerTests {
        typealias T = NativeAgentTests

        init() throws {
            _ = try ConversationScrollTests()
        }

        static func session() async throws -> (NSWindow, NSScrollView, TranscriptScroller) {
            let (window, _, scroll) = try await ConversationScrollTests.session()
            let scroller = try #require(scroll.verticalScroller as? TranscriptScroller, "the transcript has the system's scroller")
            return (window, scroll, scroller)
        }

        /// The scroll view's own place for the clip, from the lazy stack's
        /// content size: what the system scroller would show.
        static func systemPlace(_ scroll: NSScrollView) -> Double {
            let doc = scroll.documentView?.frame.height ?? 0
            let insets = scroll.contentInsets
            let range = max(1, doc + insets.top + insets.bottom - scroll.contentView.bounds.height)
            return Double((scroll.contentView.bounds.minY + insets.top) / range)
        }

        /// The knob's top as a share of its track, as drawn.
        static func knob(_ scroller: NSScroller) -> Double {
            scroller.doubleValue * Double(1 - scroller.knobProportion)
        }

        /// A mouse event at `point` in the scroller.
        static func mouse(_ type: NSEvent.EventType, _ point: NSPoint, in scroller: NSScroller, flags: NSEvent.ModifierFlags = []) -> NSEvent {
            let window = scroller.window!
            return NSEvent.mouseEvent(
                with: type, location: scroller.convert(point, to: nil), modifierFlags: flags, timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: type == .leftMouseUp ? 0 : 1)!
        }

        /// Events for the scroller's tracking to take, in order, as the
        /// pointer would send them.
        static var queued: [NSEvent] = []

        static func queue(_ event: NSEvent) {
            queued.append(event)
            TranscriptScroller.nextEvent = { kinds in
                guard let i = queued.firstIndex(where: { kinds.contains(NSEvent.EventTypeMask(type: $0.type)) }) else { return nil }
                return queued.remove(at: i)
            }
        }

        /// Down along the track by `share` of the knob's travel, as a point in
        /// the scroller, for the knob's top.
        static func knobTop(_ scroller: NSScroller, at share: CGFloat) -> CGFloat {
            let slot = scroller.rect(for: .knobSlot)
            let travel = slot.height - scroller.rect(for: .knob).height
            return scroller.isFlipped ? slot.minY + travel * share : slot.maxY - travel * share
        }

        /// Scrolled up a long session a step at a time, with the stack laid out
        /// between steps, the knob moves with the scroll and nothing else, where
        /// the stack sizes its content again by thousands of points. Not a
        /// timing test: `theScrollerHoldsStill` flings it.
        @Test("Scrolled up a long session, the knob moves only with the scroll where the stack resizes")
        func theKnobHoldsThroughAResize() async throws {
            let (window, scroll, scroller) = try await Self.session()
            defer { window.close() }
            var worst = 0.0
            var systemWorst = 0.0
            var resized: CGFloat = 0
            var placeWas = Self.knob(scroller)
            var systemWas = Self.systemPlace(scroll)
            var docWas = scroll.documentView?.frame.height ?? 0
            // One two-finger scroll, 150 pt an event, as a trackpad's: the
            // stack keeps the rows on screen through its resize only for a
            // scroll it's told of, not for a clip moved under it.
            let step: CGFloat = 150
            var first = true
            var steps = 0
            while scroll.contentView.bounds.minY > -scroll.contentInsets.top + 1, steps < 1_000 {
                steps += 1
                let clip = scroll.contentView.bounds
                if let event = Fling.wheel(step, phase: (first ? 1 : 2, 0)) { scroll.scrollWheel(with: event) }
                first = false
                // Applied on the scroll view's next frame.
                for _ in 0..<25 where scroll.contentView.bounds.minY > clip.minY - 1 {
                    await T.settle(window, 20)  // capture-exempt: not a capture; the scroll applied and the stack laid out
                }
                let doc = scroll.documentView?.frame.height ?? 0
                let range = max(1, doc + scroll.contentInsets.top + scroll.contentInsets.bottom - clip.height)
                // A step moves the knob by its share of the track, and the
                // knob's own size by at most a point or two.
                let allowed = Double(step / range) * 1.5 + 0.002
                let place = Self.knob(scroller)
                let system = Self.systemPlace(scroll)
                worst = max(worst, abs(place - placeWas) - allowed)
                systemWorst = max(systemWorst, abs(system - systemWas) - allowed)
                resized = max(resized, abs(doc - docWas))
                placeWas = place
                systemWas = system
                docWas = doc
            }
            if let event = Fling.wheel(0, phase: (4, 0)) { scroll.scrollWheel(with: event) }
            // The fixture reaches the stack's resize, which would have moved
            // the system's knob: otherwise this checks nothing.
            #expect(resized > 2_000, "the stack never sized its content again (largest change \(Int(resized)) pt)")
            #expect(systemWorst > 0.05, "the content's own place moved at most \(systemWorst * 100)% beyond the scroll")
            #expect(worst <= 0.02, "the knob jumped \(String(format: "%.1f", worst * 100))% of its track beyond the scroll")
            #expect(scroller.doubleValue < 0.001, "at the first row the knob is at the top: \(scroller.doubleValue)")
        }

        @Test("At rest at the end the knob is at the bottom, sized to the window's share of the session")
        func theKnobStartsAtTheEnd() async throws {
            let (window, scroll, scroller) = try await Self.session()
            defer { window.close() }
            #expect(scroller.doubleValue > 0.999)
            #expect(scroller.knobProportion > 0.01 && scroller.knobProportion < 0.05, "\(scroller.knobProportion)")
            #expect(scroll.hasVerticalScroller)
        }

        /// Shown, as it is while the pointer is over it: an overlay scroller
        /// faded out takes no clicks.
        static func shown(_ style: NSScroller.Style, _ scroll: NSScrollView, _ window: NSWindow) async throws -> TranscriptScroller {
            scroll.scrollerStyle = style
            scroll.flashScrollers()
            await T.settle(window, 60)  // capture-exempt: not a capture; the scroller tiled and shown
            return try #require(scroll.verticalScroller as? TranscriptScroller)
        }

        @Test(
            "Dragging the knob scrolls in proportion, and the knob stays under the pointer",
            arguments: [NSScroller.Style.legacy, .overlay])
        func draggingTheKnobScrolls(style: NSScroller.Style) async throws {
            defer { TranscriptScroller.nextEvent = TranscriptScroller.systemNextEvent; Self.queued = [] }
            let (window, scroll, _) = try await Self.session()
            defer { window.close() }
            let scroller = try await Self.shown(style, scroll, window)
            var started = 0
            var ended = 0
            let center = NotificationCenter.default
            let a = center.addObserver(forName: NSScrollView.willStartLiveScrollNotification, object: scroll, queue: nil) { _ in started += 1 }
            let b = center.addObserver(forName: NSScrollView.didEndLiveScrollNotification, object: scroll, queue: nil) { _ in ended += 1 }
            defer { center.removeObserver(a); center.removeObserver(b) }
            let knob = scroller.rect(for: .knob)
            let grab = NSPoint(x: knob.midX, y: knob.midY)
            let offset = grab.y - knob.minY
            var places: [Double] = []
            for share in [0.5, 0.25] as [CGFloat] {
                let start = NSPoint(x: knob.midX, y: Self.knobTop(scroller, at: scroller.doubleValue) + offset)
                try #require(scroller.testPart(scroller.convert(start, to: nil)) == .knob, "no knob at \(start): \(scroller.rect(for: .knob))")
                let to = NSPoint(x: knob.midX, y: Self.knobTop(scroller, at: share) + offset)
                for step in 1...4 {
                    let y = start.y + (to.y - start.y) * CGFloat(step) / 4
                    Self.queue(Self.mouse(.leftMouseDragged, NSPoint(x: to.x, y: y), in: scroller))
                }
                Self.queue(Self.mouse(.leftMouseUp, to, in: scroller))
                scroller.mouseDown(with: Self.mouse(.leftMouseDown, start, in: scroller))
                await T.settle(window, 200)  // capture-exempt: not a capture; the rows at the new place laid out
                #expect(abs(scroller.doubleValue - Double(share)) < 0.01, "dragged to \(share), the knob is at \(scroller.doubleValue)")
                places.append(Self.systemPlace(scroll))
            }
            // The clip went where the knob was taken, near enough by the
            // content's own measure (which is the lazy stack's guess).
            #expect(abs(places[0] - 0.5) < 0.1, "at half the knob's travel, the clip is at \(places[0]) of the content")
            #expect(places[1] < places[0] - 0.1, "dragged up, the clip went from \(places[0]) to \(places[1])")
            #expect(started == 2 && ended == 2, "\(started) live scrolls began, \(ended) ended")
        }

        @Test(
            "A click in the track pages toward it; with jump-to-spot, the knob goes to the click",
            arguments: [NSScroller.Style.legacy, .overlay])
        func clickingTheTrack(style: NSScroller.Style) async throws {
            defer { TranscriptScroller.nextEvent = TranscriptScroller.systemNextEvent; Self.queued = [] }
            let (window, scroll, _) = try await Self.session()
            defer { window.close() }
            let scroller = try await Self.shown(style, scroll, window)
            // Off the end, where the knob is placed from the rows on screen.
            scroller.setAccessibilityValue(NSNumber(value: 0.7))
            await T.settle(window, 200)  // capture-exempt: not a capture; the rows there laid out
            scroll.flashScrollers()
            let knob = scroller.rect(for: .knob)
            let above = NSPoint(x: knob.midX, y: Self.knobTop(scroller, at: 0.3))
            let layout = try #require(scroller.layout)
            let was = try #require(layout.knob(scroll)).place
            try #require(scroller.testPart(scroller.convert(above, to: nil)) == .decrementPage, "no track at \(above)")
            TranscriptScroller.jumpsToClick = { false }
            defer { TranscriptScroller.jumpsToClick = TranscriptScroller.systemJumpsToClick }
            Self.queue(Self.mouse(.leftMouseUp, above, in: scroller))
            scroller.mouseDown(with: Self.mouse(.leftMouseDown, above, in: scroller))
            await T.settle(window, 100)  // capture-exempt: not a capture; the page laid out
            // A page up: the window's height less the context kept, by our
            // heights (the clip's own place moves again as the stack resizes
            // above, the rows holding still), give or take a row's estimate put
            // right as it came on screen.
            let page = scroll.contentView.bounds.height - scroll.verticalPageScroll
            let moved = was - (try #require(layout.knob(scroll)).place)
            #expect(abs(moved - page) < 12, "a page is \(page) pt; the view moved \(moved)")

            TranscriptScroller.jumpsToClick = { true }
            let middle = NSPoint(x: knob.midX, y: Self.knobTop(scroller, at: 0.5) + scroller.rect(for: .knob).height / 2)
            scroll.flashScrollers()
            try #require(scroller.testPart(scroller.convert(middle, to: nil)) == .decrementPage, "no track at \(middle)")
            Self.queue(Self.mouse(.leftMouseUp, middle, in: scroller))
            scroller.mouseDown(with: Self.mouse(.leftMouseDown, middle, in: scroller))
            await T.settle(window, 200)  // capture-exempt: not a capture; the rows there laid out
            #expect(abs(scroller.doubleValue - 0.5) < 0.01, "clicked at the middle, the knob is at \(scroller.doubleValue)")
            #expect(abs(Self.systemPlace(scroll) - 0.5) < 0.1)
        }

        @Test("Overlay or legacy, as Show Scroll Bars says, it is the transcript's scroller, placed the same")
        func bothScrollerStyles() async throws {
            let (window, scroll, scroller) = try await Self.session()
            defer { window.close() }
            let value = scroller.doubleValue
            for style in [NSScroller.Style.legacy, .overlay] {
                scroll.scrollerStyle = style
                await T.settle(window, 100)  // capture-exempt: not a capture; the scroll view tiled again
                let now = try #require(scroll.verticalScroller as? TranscriptScroller, "\(style.rawValue): the scroller was replaced")
                #expect(now.scrollerStyle == style)
                #expect(abs(now.frame.width - NSScroller.scrollerWidth(for: now.controlSize, scrollerStyle: style)) < 1, "\(style.rawValue): \(now.frame.width) pt wide")
                #expect(abs(now.doubleValue - value) < 0.002, "\(style.rawValue): the knob moved from \(value) to \(now.doubleValue)")
            }
            #expect(TranscriptScroller.isCompatibleWithOverlayScrollers)
        }

        /// The knob's opacity as the overlay scroller draws it: AppKit's
        /// own, read through its scroller part (`NSScrollerImp`), since an
        /// overlay knob faded out is still a scroller with a frame.
        static func knobAlpha(_ scroller: NSScroller) -> Double? {
            ((scroller.value(forKey: "scrollerImp") as? NSObject)?.value(forKey: "knobAlpha") as? NSNumber)?.doubleValue
        }

        @Test("A two-finger scroll shows the overlay knob, as it does the system's")
        func anOverlayScrollShowsTheKnob() async throws {
            let (window, scroll, scroller) = try await Self.session()
            defer { window.close() }
            scroll.scrollerStyle = .overlay
            await T.settle(window, 60)  // capture-exempt: not a capture; the scroller tiled
            for i in 0..<10 {
                if let event = Fling.wheel(60, phase: (i == 0 ? 1 : 2, 0)) { scroll.scrollWheel(with: event) }
                await T.settle(window, 20)  // capture-exempt: not a capture; each event's scroll applied
            }
            if let event = Fling.wheel(0, phase: (4, 0)) { scroll.scrollWheel(with: event) }
            let alpha = try #require(Self.knobAlpha(scroller), "AppKit's scroller part has no knobAlpha to read")
            #expect(alpha > 0.5, "mid-scroll, the overlay knob's opacity is \(alpha)")
        }

        @Test("VoiceOver reads the knob's place and can move it")
        func voiceOverReadsAndMoves() async throws {
            let (window, scroll, scroller) = try await Self.session()
            defer { window.close() }
            #expect(scroller.accessibilityRole() == .scrollBar)
            let read = try #require(scroller.accessibilityValue() as? NSNumber)
            #expect(abs(read.doubleValue - scroller.doubleValue) < 0.001)
            scroller.setAccessibilityValue(NSNumber(value: 0.5))
            await T.settle(window, 200)  // capture-exempt: not a capture; the rows there laid out
            #expect(abs(scroller.doubleValue - 0.5) < 0.01, "set to half, the knob is at \(scroller.doubleValue)")
            #expect(abs(Self.systemPlace(scroll) - 0.5) < 0.1)
            let now = try #require(scroller.accessibilityValue() as? NSNumber)
            #expect(abs(now.doubleValue - 0.5) < 0.01)
        }
    }
}

/// Opt-in (FARCOOLER_CAPTURE_OUT): the transcript's scroller over
/// `RealSession`, mid-scroll and at rest, light and dark, overlay and
/// legacy, for judging against the system's by eye (ov-701).
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct TranscriptScrollerCaptures {
    @Test func scroller() async throws {
        let out = URL(fileURLWithPath: try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"]))
            .appendingPathComponent("transcript-scroller")
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        _ = try ConversationScrollTests()
        for dark in [false, true] {
            for style in [NSScroller.Style.overlay, .legacy] {
                let (window, _, scroll) = try await ConversationScrollTests.session()
                defer { window.close() }
                window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                scroll.scrollerStyle = style
                let name = "\(dark ? "dark" : "light")-\(style == .overlay ? "overlay" : "legacy")"
                // A two-finger scroll up a few screens, the fingers still down.
                let start = scroll.contentView.bounds.minY
                for i in 0..<40 {
                    if let event = Fling.wheel(60, phase: (i == 0 ? 1 : 2, 0)) { scroll.scrollWheel(with: event) }
                    await NativeAgentTests.settle(window, 20)  // capture-exempt: not a capture; each event's scroll applied
                }
                try await Capture.keep(
                    window, to: out.appendingPathComponent("\(name)-mid-scroll.png"), expecting: "scrolled up mid-gesture",
                    until: { scroll.contentView.bounds.minY < start - 1_000 })
                if let event = Fling.wheel(0, phase: (4, 0)) { scroll.scrollWheel(with: event) }
                // At rest: an overlay scroller has faded.
                try await Task.sleep(for: .seconds(2))
                try await Capture.keep(
                    window, to: out.appendingPathComponent("\(name)-rest.png"), expecting: "at rest after the scroll",
                    until: { !LiveScroll.inFlight })
            }
        }
    }
}
