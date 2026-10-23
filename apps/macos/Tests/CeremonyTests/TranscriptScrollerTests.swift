import AgentKit
import AppKit
import SwiftUI
import TestWindows
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

        static func session(width: CGFloat = 720, rows: Int = RealSession.count, resizable: Bool = false) async throws
            -> (NSWindow, NSScrollView, NSScroller)
        {
            let (window, _, scroll) = try await ConversationScrollTests.session(width: width, rows: rows, resizable: resizable)
            let scroller = try #require(scroll.verticalScroller)
            try #require(TranscriptScroller.driving(scroll) != nil, "the transcript's scroller isn't driven")
            #expect(type(of: scroller) == NSScroller.self, "the scroller is AppKit's own: \(type(of: scroller))")
            return (window, scroll, scroller)
        }

        /// The scroll view's own place for the clip, from the lazy stack's
        /// content size: what the system scroller would show.
        static func systemPlace(_ scroll: NSScrollView) -> Double {
            Double((scroll.contentView.bounds.minY + scroll.contentInsets.top) / range(scroll))
        }

        /// The clip's travel through the content as the stack sizes it.
        static func range(_ scroll: NSScrollView) -> CGFloat {
            let doc = scroll.documentView?.frame.height ?? 0
            return max(1, doc + scroll.contentInsets.top + scroll.contentInsets.bottom - scroll.contentView.bounds.height)
        }

        /// The knob's top as a share of its track, as drawn.
        static func knob(_ scroller: NSScroller) -> Double {
            scroller.doubleValue * Double(1 - scroller.knobProportion)
        }

        /// Down along the track by `share` of the knob's travel, as a point
        /// in the scroller, for the knob's top.
        static func knobTop(_ scroller: NSScroller, at share: CGFloat) -> CGFloat {
            let slot = scroller.rect(for: .knobSlot)
            let travel = slot.height - scroller.rect(for: .knob).height
            return scroller.isFlipped ? slot.minY + travel * share : slot.maxY - travel * share
        }

        /// Shown, as it is while the pointer is over it: an overlay
        /// scroller faded out takes no clicks.
        static func shown(_ style: NSScroller.Style, _ scroll: NSScrollView, _ window: NSWindow) async throws -> NSScroller {
            scroll.scrollerStyle = style
            scroll.flashScrollers()
            await T.settle(window, 60)  // capture-exempt: not a capture; the scroller tiled and shown
            let scroller = try #require(scroll.verticalScroller)
            try #require(scroller.target === TranscriptScroller.driving(scroll), "\(style.rawValue): the scroller isn't driven")
            return scroller
        }

        /// Where AppKit's knob tracking leaves the scroller when the pointer
        /// is at `value`: its value set, and its action sent. (AppKit's own
        /// tracking can't run in a test: its event loop outside `NSApp.run`
        /// ends the test process.)
        static func track(_ scroller: NSScroller, to value: Double) {
            scroller.doubleValue = value
            _ = scroller.sendAction(scroller.action, to: scroller.target)
        }

        /// Off the end, where rows on screen place the knob: VoiceOver's
        /// value set, and the aim let go.
        static func park(_ scroller: NSScroller, at value: Double, _ window: NSWindow) async {
            scroller.setAccessibilityValue(NSNumber(value: value))
            await T.settle(window, 600)  // capture-exempt: not a capture; the rows there laid out and the aim let go
        }

        /// A two-finger scroll up of `steps` events of `step` points, the
        /// stack laid out after each; `each` after each event.
        static func scrollUp(
            _ scroll: NSScrollView, _ window: NSWindow, by step: CGFloat, steps: Int = 1_000, each: () -> Void = {}
        ) async {
            var done = 0
            while scroll.contentView.bounds.minY > -scroll.contentInsets.top + 1, done < steps {
                let clip = scroll.contentView.bounds
                if let event = Fling.wheel(step, phase: (done == 0 ? 1 : 2, 0)) { scroll.scrollWheel(with: event) }
                done += 1
                // Applied on the scroll view's next frame; read once a frame
                // has been committed after it, as the reader sees it.
                for _ in 0..<25 where scroll.contentView.bounds.minY > clip.minY - 1 {
                    await T.settle(window, 20)  // capture-exempt: not a capture; the scroll applied and the stack laid out
                }
                await T.settle(window, 20)  // capture-exempt: not a capture; a frame committed after the scroll
                each()
            }
            if let event = Fling.wheel(0, phase: (4, 0)) { scroll.scrollWheel(with: event) }
            await T.settle(window, 100)  // capture-exempt: not a capture; the gesture's end
        }

        /// Scrolled up a long session a step at a time, with the stack laid
        /// out between steps, the knob moves with the scroll and nothing
        /// else, where the stack sizes its content again by thousands of
        /// points: in the default window, and in a wide one, where prose and
        /// the bubble keep their reading measures. Thinking rows and task
        /// lists, as narrow as their words, are among the rows. Not a timing
        /// test: `theScrollerHoldsStill` flings it.
        @Test(
            "Scrolled up a long session, the knob moves only with the scroll where the stack resizes",
            arguments: [CGFloat(720), 1_280])
        func theKnobHoldsThroughAResize(width: CGFloat) async throws {
            let (window, scroll, scroller) = try await Self.session(width: width)
            defer { window.close() }
            var worst = 0.0
            var systemWorst = 0.0
            var resized: CGFloat = 0
            var placeWas = Self.knob(scroller)
            var systemWas = Self.systemPlace(scroll)
            var docWas = scroll.documentView?.frame.height ?? 0
            let step: CGFloat = 150
            await Self.scrollUp(scroll, window, by: step) {
                let doc = scroll.documentView?.frame.height ?? 0
                // A step moves the knob by its share of the track, and the
                // knob's own size by at most a point or two.
                let allowed = Double(step / Self.range(scroll)) * 1.5 + 0.002
                let place = Self.knob(scroller)
                let system = Self.systemPlace(scroll)
                worst = max(worst, abs(place - placeWas) - allowed)
                systemWorst = max(systemWorst, abs(system - systemWas) - allowed)
                resized = max(resized, abs(doc - docWas))
                placeWas = place
                systemWas = system
                docWas = doc
            }
            // The fixture reaches the stack's resize, which would have moved
            // the system's knob: otherwise this checks nothing.
            #expect(resized > 2_000, "the stack never sized its content again (largest change \(Int(resized)) pt)")
            #expect(systemWorst > 0.05, "the content's own place moved at most \(systemWorst * 100)% beyond the scroll")
            #expect(worst <= 0.02, "at \(Int(width)) pt the knob jumped \(String(format: "%.1f", worst * 100))% of its track beyond the scroll")
            #expect(scroller.doubleValue < 0.001, "at the first row the knob is at the top: \(scroller.doubleValue)")
        }

        /// At the end, before any row above is built, the knob is sized to
        /// the session's height as it turns out once every row has been
        /// laid out (read at the top): within 4%, where the stack's guess is
        /// 18% over.
        @Test("At the end the knob is sized to the session's laid-out height, not the stack's guess", arguments: [CGFloat(720), 1_280])
        func theKnobIsSizedToTheTrueHeight(width: CGFloat) async throws {
            let (window, scroll, scroller) = try await Self.session(width: width)
            defer { window.close() }
            #expect(scroller.doubleValue > 0.999, "the session opens at its end")
            let ours = scroller.knobProportion
            let clip = scroll.contentView.bounds.height
            let insets = scroll.contentInsets.top + scroll.contentInsets.bottom
            let guessed = clip / ((scroll.documentView?.frame.height ?? 0) + insets)
            await Self.scrollUp(scroll, window, by: 900)
            #expect(scroller.doubleValue < 0.001, "the scroll never reached the first row")
            let truth = scroller.knobProportion
            #expect(abs(guessed - truth) / truth > 0.1, "the stack's guess was already within 10% of the truth: \(guessed) for \(truth)")
            #expect(abs(ours - truth) / truth < 0.04, "at \(Int(width)) pt the knob was \(ours) of the track, the session's share \(truth)")
        }

        @Test("A reply growing at the end keeps the view at the end, the knob at the bottom and shrinking with it")
        func aGrowingReplyStaysAtTheEnd() async throws {
            let (window, model, scroll) = try await ConversationScrollTests.session()
            defer { window.close() }
            let scroller = try #require(scroll.verticalScroller)
            let doc = scroll.documentView?.frame.height ?? 0
            let stream = ConversationScrollTests.streams(model)
            var sizes: [CGFloat] = [scroller.knobProportion]
            for tick in stride(from: 0, through: 120, by: 10) {
                stream(tick)
                await T.settle(window, 60)  // capture-exempt: not a capture; the reply's words laid out
                #expect(Self.systemPlace(scroll) > 0.999, "tick \(tick): the view left the end")
                #expect(scroller.doubleValue > 0.999, "tick \(tick): the knob left the bottom: \(scroller.doubleValue)")
                sizes.append(scroller.knobProportion)
            }
            let grew = (scroll.documentView?.frame.height ?? 0) - doc
            #expect(grew > 100, "the reply never grew")
            // The knob is the window's share of a longer session: smaller by
            // about what the reply added.
            let first = try #require(sizes.first), last = try #require(sizes.last)
            let expected = first * (1 - grew / (scroll.contentView.bounds.height / first))
            #expect(abs(last - expected) < (first - expected) * 0.3, "the knob went from \(first) to \(last) of the track, for \(expected)")
        }

        @Test("Show More on a row on screen grows it in place; the knob shrinks and moves by its share")
        func showMoreHoldsTheKnob() async throws {
            let frames = NativePolishTests.Frames()
            let (window, _, scroll) = try await ConversationScrollTests.session(probed: frames)
            defer { window.close() }
            let scroller = try #require(scroll.verticalScroller)
            let height = scroll.contentView.bounds.height
            // Up the session until a folded report is on screen.
            var tries = 0
            while !(frames.views["native-show-more"].map { $0.minY > 60 && $0.maxY < height - 120 } ?? false), tries < 40 {
                tries += 1
                await Self.scrollUp(scroll, window, by: 80, steps: 4)
            }
            try #require(frames.views["native-show-more"] != nil, "no Show More came on screen")
            let doc = scroll.documentView?.frame.height ?? 0
            let was = scroller.doubleValue
            let sizeWas = scroller.knobProportion
            let top = TranscriptScroller.driving(scroll)?.layout?.top(scroll)?.id
            #expect(NativePolishTests.click("native-show-more", frames: frames, in: window))
            await T.settle(window, 300)  // capture-exempt: not a capture; the row opened
            let grew = (scroll.documentView?.frame.height ?? 0) - doc
            #expect(grew > 20, "Show More opened nothing (\(grew) pt)")
            #expect(TranscriptScroller.driving(scroll)?.layout?.top(scroll)?.id == top, "the reader's row changed")
            // Taller below the clip's top: the same place is a smaller share
            // of a longer travel, and the knob is a smaller share too.
            let range = Self.range(scroll)
            #expect(abs(scroller.doubleValue - was) <= Double(grew / range) + 0.002, "the knob went from \(was) to \(scroller.doubleValue) for \(grew) pt")
            let expected = sizeWas * (1 - grew / (height / sizeWas))
            #expect(abs(scroller.knobProportion - expected) < (sizeWas - expected) * 0.3,
                    "the knob's size went from \(sizeWas) to \(scroller.knobProportion), for \(expected)")
        }

        @Test("A reply growing at the end while the reader is up the session leaves them where they are")
        func aReplyBelowLeavesTheReader() async throws {
            let (window, model, scroll) = try await ConversationScrollTests.session()
            defer { window.close() }
            let scroller = try #require(scroll.verticalScroller)
            await Self.park(scroller, at: 0.5, window)
            let top = TranscriptScroller.driving(scroll)?.layout?.top(scroll)
            let was = scroller.doubleValue
            let place = try #require(TranscriptScroller.driving(scroll)?.layout?.knob(scroll)).place
            let doc = scroll.documentView?.frame.height ?? 0
            let stream = ConversationScrollTests.streams(model)
            for tick in stride(from: 0, through: 60, by: 10) {
                stream(tick)
                await T.settle(window, 60)  // capture-exempt: not a capture; the reply's words laid out
            }
            #expect(top != nil)
            #expect(TranscriptScroller.driving(scroll)?.layout?.top(scroll)?.id == top?.id, "the reader's row changed")
            // Longer below: the same place, a smaller share of the travel.
            let now = try #require(TranscriptScroller.driving(scroll)?.layout?.knob(scroll)).place
            #expect(abs(now - place) < 2, "the reader's place went from \(place) to \(now) pt")
            #expect(scroller.doubleValue < was, "the knob went from \(was) to \(scroller.doubleValue) as the reply grew below")
            _ = doc
        }

        /// Resized narrower and back a step at a time, as a window's edge
        /// is dragged, with every row laid out at the wide width first: the
        /// knob moves little, and the reader stays on their row.
        @Test("Resizing the window mid-session keeps the reader on their row and the knob within 2%")
        func aResizeHoldsTheReader() async throws {
            let (window, scroll, scroller) = try await Self.session(width: 1_300, resizable: true)
            defer { window.close() }
            // Every row laid out at the wide width first, so the narrow one
            // has to scale their heights (text wraps to more lines there).
            await Self.scrollUp(scroll, window, by: 900)
            await Self.park(scroller, at: 0.5, window)
            // A third of the way down the reader's row, not at its edge.
            let layout = try #require(TranscriptScroller.driving(scroll)?.layout)
            let reading = try #require(layout.reading(scroll))
            let y = try #require(layout.origin(for: .init(id: reading.id, share: 0.3), in: scroll))
            scroll.contentView.scroll(to: NSPoint(x: 0, y: y))
            scroll.reflectScrolledClipView(scroll.contentView)
            await T.settle(window, 200)  // capture-exempt: not a capture; the rows there laid out
            let top = try #require(TranscriptScroller.driving(scroll)?.layout?.top(scroll)?.id)
            let start = scroller.doubleValue
            var worst = 0.0
            // Down to a narrow tile, where prose and the bubble wrap to more
            // lines than they did wide.
            for width in Array(stride(from: CGFloat(1_280), through: 420, by: -20)) + Array(stride(from: CGFloat(440), through: 900, by: 20)) + [420] {
                window.setContentSize(NSSize(width: width, height: 900))
                window.contentView?.layoutSubtreeIfNeeded()
                window.displayIfNeeded()
                await T.settle(window, 60)  // capture-exempt: not a capture; the rows laid out at the new width
                worst = max(worst, abs(scroller.doubleValue - start))
            }
            // The knob holds while the width changes, then goes where the
            // reader is: still within 2% of where it started.
            await T.settle(window, 500)  // capture-exempt: not a capture; the last width laid out and held
            worst = max(worst, abs(scroller.doubleValue - start))
            #expect(abs((TranscriptScroller.driving(scroll)?.layout?.knob(scroll)?.value ?? 0) - scroller.doubleValue) < 0.001, "the knob was left held")
            #expect(worst <= 0.02, "the knob moved \(String(format: "%.1f", worst * 100))% of its track")
            let rows = TranscriptScroller.driving(scroll)?.layout.map { layout in Set(layout.items.map(\.id)) } ?? []
            let now = try #require(TranscriptScroller.driving(scroll)?.layout?.top(scroll)?.id)
            #expect(now == top, "the reader went from \(top) to \(now)")
            #expect(rows.contains(now))
            // Its size, with the rows laid out wide scaled to the narrow
            // width, is the session's share as it turns out once every row
            // is laid out narrow again.
            let size = scroller.knobProportion
            await Self.scrollUp(scroll, window, by: 900)
            let truth = scroller.knobProportion
            #expect(abs(size - truth) / truth < 0.04, "narrowed, the knob was \(size) of the track; laid out, the session's share is \(truth)")
        }

        /// At the end, with a reply streaming, the window narrowed and
        /// widened: the stack's bottom anchor keeps the reader at the end,
        /// and nothing of ours pulls them off it to the row that was at the
        /// top.
        @Test("Resizing at the end while a reply streams keeps the reader at the end")
        func aResizeAtTheEndStaysAtTheEnd() async throws {
            let (window, model, scroll) = try await ConversationScrollTests.session(resizable: true)
            defer { window.close() }
            let scroller = try #require(scroll.verticalScroller)
            let stream = ConversationScrollTests.streams(model)
            var tick = 0
            for width in Array(stride(from: CGFloat(700), through: 420, by: -20)) + Array(stride(from: CGFloat(440), through: 900, by: 40)) {
                window.setContentSize(NSSize(width: width, height: 900))
                stream(tick)
                tick += 10
                await T.settle(window, 40)  // capture-exempt: not a capture; the rows laid out at the new width
            }
            for _ in 0..<6 {
                stream(tick)
                tick += 10
                await T.settle(window, 100)  // capture-exempt: not a capture; the width held and the reply grew
                #expect(Self.systemPlace(scroll) > 0.999, "the reader left the end: \(Self.systemPlace(scroll))")
                #expect(scroller.doubleValue > 0.999, "the knob left the bottom: \(scroller.doubleValue)")
            }
        }

        /// A two-finger scroll begun just after a click in the track jumped
        /// the knob: the reader's scroll wins at once. The jump is let go on
        /// the gesture's first event, so the clip isn't put back on it.
        @Test("A scroll begun right after a track jump drops the jump at once")
        func aScrollAfterAJumpIsTheReaders() async throws {
            let (window, scroll, scroller) = try await Self.session()
            defer { window.close() }
            let driver = try #require(TranscriptScroller.driving(scroll))
            // Into rows the stack hasn't built, where the jump takes frames
            // to settle.
            Self.track(scroller, to: 0.3)
            try #require(driver.aimed != nil, "the jump settled before the scroll could begin")
            if let event = Fling.wheel(100, phase: (1, 0)) { scroll.scrollWheel(with: event) }
            #expect(driver.aimed == nil, "the jump is still held while the reader scrolls")
            await Self.scrollUp(scroll, window, by: 100, steps: 10)
            await T.settle(window, 400)  // capture-exempt: not a capture; long past the jump's settling
            #expect(driver.aimed == nil)
            #expect(!LiveScroll.inFlight)
        }

        /// A resize's hold begun just before a two-finger scroll (the legacy
        /// scroller appearing narrows the clip): the reader's scroll drops
        /// the putting back of where they were.
        @Test("A scroll begun while the width settles isn't undone when it settles")
        func aScrollDuringAResizeHoldIsTheReaders() async throws {
            let (window, scroll, scroller) = try await Self.session()
            defer { window.close() }
            await Self.park(scroller, at: 0.5, window)
            let driver = try #require(TranscriptScroller.driving(scroll))
            scroll.scrollerStyle = scroll.scrollerStyle == .overlay ? .legacy : .overlay
            await T.settle(window, 20)  // capture-exempt: not a capture; the clip narrowed and the hold begun
            try #require(driver.restorePending, "the width's hold didn't begin")
            if let event = Fling.wheel(100, phase: (1, 0)) { scroll.scrollWheel(with: event) }
            #expect(!driver.restorePending, "the reader scrolled, and would still be put back")
            await Self.scrollUp(scroll, window, by: 500, steps: 10)
            await T.settle(window, 800)  // capture-exempt: not a capture; long past the width's hold
            #expect(!driver.restorePending)
            #expect(!LiveScroll.inFlight)
        }

        /// A mouse wheel's tick (no phase), as a mouse sends it.
        static func tick(_ points: Int32, in window: NSWindow) -> NSEvent? {
            guard let cg = CGEvent(scrollWheelEvent2Source: nil, units: .line, wheelCount: 1, wheel1: points, wheel2: 0, wheel3: 0) else { return nil }
            cg.setIntegerValueField(.mouseEventWindowUnderMousePointer, value: Int64(window.windowNumber))
            cg.setIntegerValueField(.mouseEventWindowUnderMousePointerThatCanHandleThisEvent, value: Int64(window.windowNumber))
            return NSEvent(cgEvent: cg)
        }

        /// A jump, then the reader's own event arrives before it settles:
        /// a mouse wheel's tick, or a press in the window off the scroller.
        /// AppKit begins no live scroll for either, so ours ends.
        @Test("A wheel tick or a click elsewhere after a jump ends the jump's live scroll", arguments: ["wheel", "click"])
        func aReaderEventEndsTheJump(kind: String) async throws {
            let (window, scroll, scroller) = try await Self.session()
            defer { window.close() }
            let driver = try #require(TranscriptScroller.driving(scroll))
            Self.track(scroller, to: 0.3)
            try #require(driver.aimed != nil, "the jump settled at once")
            #expect(LiveScroll.inFlight)
            let event: NSEvent
            if kind == "wheel" {
                event = try #require(Self.tick(3, in: window))
                try #require(event.phase.isEmpty && event.momentumPhase.isEmpty, "the tick has a phase")
            } else {
                event = try #require(NSEvent.mouseEvent(
                    with: .leftMouseDown, location: NSPoint(x: 100, y: 100), modifierFlags: [], timestamp: 0,
                    windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1))
            }
            // As the event monitor hands it over, for the window AppKit
            // sends it to.
            driver.reader(event, in: window)
            #expect(driver.aimed == nil, "the jump is still held")
            #expect(!LiveScroll.inFlight, "\(kind): the jump's live scroll was left open")
            await T.settle(window, 200)  // capture-exempt: not a capture; a few frames after
            #expect(!LiveScroll.inFlight)
        }

        /// AppKit's own tracking, as the scroller reports it to the action:
        /// the knob held and dragged, then let go; a click in the track.
        @Test("The knob tracked and a click in the track reach the scroller's own branches")
        func appKitsTrackingBranches() async throws {
            let (window, scroll, scroller) = try await Self.session()
            defer { window.close() }
            defer {
                TranscriptScroller.tracking = { NSApp.currentEvent.map { [.leftMouseDown, .leftMouseDragged, .periodic].contains($0.type) } ?? false }
                TranscriptScroller.part = { $0.hitPart }
                TranscriptScroller.held = { NSEvent.pressedMouseButtons & 1 != 0 }
            }
            let driver = try #require(TranscriptScroller.driving(scroll))
            var started = 0, ended = 0, moved = 0
            let center = NotificationCenter.default
            let a = center.addObserver(forName: NSScrollView.willStartLiveScrollNotification, object: scroll, queue: nil) { _ in started += 1 }
            let b = center.addObserver(forName: NSScrollView.didEndLiveScrollNotification, object: scroll, queue: nil) { _ in ended += 1 }
            let c = center.addObserver(forName: NSScrollView.didLiveScrollNotification, object: scroll, queue: nil) { _ in moved += 1 }
            defer { center.removeObserver(a); center.removeObserver(b); center.removeObserver(c) }
            var pressed = true
            TranscriptScroller.tracking = { true }
            TranscriptScroller.part = { _ in .knob }
            TranscriptScroller.held = { pressed }
            for value in [0.8, 0.65, 0.5] { Self.track(scroller, to: value) }
            // Held: the jump stays, however long, until the pointer lets go.
            await T.settle(window, 600)  // capture-exempt: not a capture; longer than an aim settles
            #expect(driver.aimed == 0.5, "the knob let go while held: \(String(describing: driver.aimed))")
            #expect(ended == 0)
            pressed = false
            await T.settle(window, 600)  // capture-exempt: not a capture; the aim let go
            #expect(abs(scroller.doubleValue - 0.5) < 0.01, "dragged to 0.5, the knob is at \(scroller.doubleValue)")
            #expect(started == 1 && ended == 1 && moved >= 3, "\(started) began, \(moved) moved, \(ended) ended")

            let layout = try #require(driver.layout)
            let was = try #require(layout.knob(scroll)).place
            TranscriptScroller.part = { _ in .decrementPage }
            _ = scroller.sendAction(scroller.action, to: scroller.target)
            await T.settle(window, 100)  // capture-exempt: not a capture; the page laid out
            let page = scroll.contentView.bounds.height - scroll.verticalPageScroll
            let paged = was - (try #require(layout.knob(scroll)).place)
            #expect(abs(paged - page) < 12, "a page is \(page) pt; a click in the track moved \(paged)")
            #expect(!LiveScroll.inFlight)
        }

        /// The scroller keeps the part last clicked. VoiceOver's value set
        /// after a click in the track goes to the value, not a page.
        @Test("VoiceOver's value set after a click in the track goes to the value")
        func aStalePartDoesNotPage() async throws {
            let (window, scroll, scroller) = try await Self.session()
            defer { window.close() }
            defer {
                TranscriptScroller.tracking = { NSApp.currentEvent.map { [.leftMouseDown, .leftMouseDragged, .periodic].contains($0.type) } ?? false }
                TranscriptScroller.part = { $0.hitPart }
            }
            // The click is over: AppKit tracks nothing, its part is stale.
            TranscriptScroller.tracking = { false }
            TranscriptScroller.part = { _ in .decrementPage }
            await Self.park(scroller, at: 0.4, window)
            #expect(abs(scroller.doubleValue - 0.4) < 0.01, "set to 0.4, the knob is at \(scroller.doubleValue)")
            #expect(abs(Self.systemPlace(scroll) - 0.4) < 0.1, "the clip is at \(Self.systemPlace(scroll))")
        }

        @Test("A transcript closed mid-jump stops its ticker and ends its live scroll")
        func closingMidJumpLetsGo() async throws {
            let (window, scroll, scroller) = try await Self.session()
            let driver = try #require(TranscriptScroller.driving(scroll))
            var started = 0
            var ended = 0
            let center = NotificationCenter.default
            let a = center.addObserver(forName: NSScrollView.willStartLiveScrollNotification, object: scroll, queue: nil) { _ in started += 1 }
            let b = center.addObserver(forName: NSScrollView.didEndLiveScrollNotification, object: scroll, queue: nil) { _ in ended += 1 }
            defer { center.removeObserver(a); center.removeObserver(b) }
            Self.track(scroller, to: 0.3)
            try #require(driver.ticking && driver.aimed != nil, "the jump settled at once")
            window.contentView = nil
            window.close()
            #expect(!driver.ticking, "the ticker outlived the transcript")
            #expect(started == 1 && ended == 1, "\(started) live scrolls began, \(ended) ended")
            #expect(scroller.target !== driver, "the scroller still sends its actions here")
        }

        @Test(
            "Dragging the knob scrolls in proportion, and the knob stays under the pointer",
            arguments: [NSScroller.Style.legacy, .overlay])
        func draggingTheKnobScrolls(style: NSScroller.Style) async throws {
            let (window, scroll, _) = try await Self.session()
            defer { window.close() }
            let scroller = try await Self.shown(style, scroll, window)
            var started = 0
            var ended = 0
            let center = NotificationCenter.default
            let a = center.addObserver(forName: NSScrollView.willStartLiveScrollNotification, object: scroll, queue: nil) { _ in started += 1 }
            let b = center.addObserver(forName: NSScrollView.didEndLiveScrollNotification, object: scroll, queue: nil) { _ in ended += 1 }
            defer { center.removeObserver(a); center.removeObserver(b) }
            var places: [Double] = []
            for share in [0.5, 0.25] {
                let start = scroller.doubleValue
                for step in 1...4 {
                    Self.track(scroller, to: start + (share - start) * Double(step) / 4)
                }
                await T.settle(window, 600)  // capture-exempt: not a capture; the rows at the new place laid out and the aim let go
                #expect(abs(scroller.doubleValue - share) < 0.01, "dragged to \(share), the knob is at \(scroller.doubleValue)")
                places.append(Self.systemPlace(scroll))
            }
            // The clip went where the knob was taken, near enough by the
            // content's own measure (which is the lazy stack's guess).
            #expect(abs(places[0] - 0.5) < 0.1, "at half the knob's travel, the clip is at \(places[0]) of the content")
            #expect(places[1] < places[0] - 0.1, "dragged up, the clip went from \(places[0]) to \(places[1])")
            #expect(started == 2 && ended == 2, "\(started) live scrolls began, \(ended) ended")
            #expect(!LiveScroll.inFlight)
        }

        @Test("Dragged past either end, the knob and the reader stop at that end, and the live scroll ends", arguments: [-1.0, 2.0])
        func draggingPastAnEnd(past: Double) async throws {
            let (window, scroll, _) = try await Self.session()
            defer { window.close() }
            let scroller = try await Self.shown(.legacy, scroll, window)
            await Self.park(scroller, at: 0.5, window)
            // AppKit's tracking keeps the value within the track.
            let end = past < 0 ? 0.0 : 1.0
            Self.track(scroller, to: 0.5 + (end - 0.5) / 2)
            Self.track(scroller, to: end)
            await T.settle(window, 800)  // capture-exempt: not a capture; the rows at the end laid out and the aim let go
            #expect(abs(scroller.doubleValue - end) < 0.002, "dragged past \(end), the knob is at \(scroller.doubleValue)")
            #expect(abs(Self.systemPlace(scroll) - end) < 0.002, "dragged past \(end), the clip is at \(Self.systemPlace(scroll))")
            #expect(!LiveScroll.inFlight, "the drag's live scroll never ended")
        }

        @Test(
            "A click in the track pages toward it; with jump-to-spot, the knob goes to the click",
            arguments: [NSScroller.Style.legacy, .overlay])
        func clickingTheTrack(style: NSScroller.Style) async throws {
            let (window, scroll, _) = try await Self.session()
            defer { window.close() }
            let scroller = try await Self.shown(style, scroll, window)
            await Self.park(scroller, at: 0.7, window)
            let driver = try #require(TranscriptScroller.driving(scroll))
            let layout = try #require(driver.layout)
            let was = try #require(layout.knob(scroll)).place
            // A click in the track above the knob: AppKit's action with the
            // part it hit, which a test can't set, so the page it calls for.
            driver.page(-1, scroll)
            await T.settle(window, 100)  // capture-exempt: not a capture; the page laid out
            // A page up: the window's height less the context kept, by our
            // heights (the clip's own place moves again as the stack resizes
            // above, the rows holding still), give or take a row's estimate
            // put right as it came on screen.
            let page = scroll.contentView.bounds.height - scroll.verticalPageScroll
            let moved = was - (try #require(layout.knob(scroll)).place)
            #expect(abs(moved - page) < 12, "a page is \(page) pt; the view moved \(moved)")
            #expect(!LiveScroll.inFlight)

            // "Jump to the spot that's clicked": AppKit puts the knob there
            // and tracks it.
            Self.track(scroller, to: 0.5)
            await T.settle(window, 600)  // capture-exempt: not a capture; the rows there laid out and the aim let go
            #expect(abs(scroller.doubleValue - 0.5) < 0.01, "clicked at the middle, the knob is at \(scroller.doubleValue)")
            #expect(abs(Self.systemPlace(scroll) - 0.5) < 0.1)
            #expect(!LiveScroll.inFlight)
        }

        /// A transcript that only just scrolls, and one that fits: VoiceOver
        /// and the track still move it, and leave no live scroll behind.
        @Test("On a short transcript, VoiceOver and the track leave no live scroll open", arguments: [3, 14])
        func aShortTranscriptEndsItsScroll(rows: Int) async throws {
            let (window, scroll, _) = try await Self.session(rows: rows)
            defer { window.close() }
            let scroller = try await Self.shown(.legacy, scroll, window)
            scroller.setAccessibilityValue(NSNumber(value: 0.0))
            await T.settle(window, 600)  // capture-exempt: not a capture; the aim let go
            #expect(!LiveScroll.inFlight, "\(rows) rows: VoiceOver's value left a live scroll open")
            Self.track(scroller, to: 1)
            await T.settle(window, 600)  // capture-exempt: not a capture; the aim let go
            #expect(!LiveScroll.inFlight, "\(rows) rows: a jump in the track left a live scroll open")
            try #require(TranscriptScroller.driving(scroll)).page(-1, scroll)
            await T.settle(window, 100)  // capture-exempt: not a capture; the page laid out
            #expect(!LiveScroll.inFlight, "\(rows) rows: a page left a live scroll open")
        }

        @Test("Overlay or legacy, as Show Scroll Bars says, it is the transcript's scroller, placed the same")
        func bothScrollerStyles() async throws {
            let (window, scroll, scroller) = try await Self.session()
            defer { window.close() }
            await Self.park(scroller, at: 0.4, window)
            let value = scroller.doubleValue
            #expect(abs(value - 0.4) < 0.01, "parked at \(value)")
            for style in [NSScroller.Style.legacy, .overlay, .legacy] {
                scroll.scrollerStyle = style
                await T.settle(window, 100)  // capture-exempt: not a capture; the scroll view tiled again
                let now = try #require(scroll.verticalScroller)
                #expect(now.target === TranscriptScroller.driving(scroll), "\(style.rawValue): the scroller isn't driven")
                #expect(now.scrollerStyle == style)
                let width = NSScroller.scrollerWidth(for: now.controlSize, scrollerStyle: style)
                #expect(abs(now.frame.width - width) < 1, "\(style.rawValue): \(now.frame.width) pt wide")
                #expect(abs(now.doubleValue - value) < 0.002, "\(style.rawValue): the knob moved from \(value) to \(now.doubleValue)")
            }
        }

        /// AppKit draws an `NSScroller` subclass in its old legacy style, a
        /// hairline track and a thin knob; ours must be drawn as the
        /// system's: the same pixels as AppKit's own scroller in the same
        /// place, across the track away from the knob.
        @Test("In legacy style the scroller is drawn as AppKit's own, light and dark", arguments: [false, true])
        func legacyIsDrawnAsTheSystems(dark: Bool) async throws {
            let (window, scroll, scroller) = try await Self.session()
            defer { window.close() }
            window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            // Observed, as anything in the app may: an observer swaps the
            // object's class (KVO), which once brought the old look back.
            let observing = scroller.observe(\.frame) { _, _ in }
            defer { observing.invalidate() }
            scroll.scrollerStyle = .legacy
            await T.settle(window, 100)  // capture-exempt: not a capture; the scroller tiled
            // Where AppKit puts the knob and the track it runs in, which
            // differ between the old style and the system's.
            // AppKit's own draws its knob and track as filled, rounded
            // layers; the old style draws into them instead.
            func parts(_ s: NSScroller) -> String {
                let imp = s.value(forKey: "scrollerImp") as? NSObject
                func layer(_ key: String) -> String {
                    guard imp?.responds(to: NSSelectorFromString(key)) == true, let l = imp?.value(forKey: key) as? CALayer else { return "\(key): none" }
                    let fill = l.backgroundColor.flatMap { $0.components?.map { String(format: "%.3f", $0) }.joined(separator: " ") } ?? "none"
                    return "\(key): \(l.frame) fill \(fill) corner \(l.cornerRadius) drawn \(l.contents != nil)"
                }
                return layer("knobLayer") + "; " + layer("trackLayer")
            }
            let ours = parts(scroller)
            let system = NSScroller(frame: scroller.frame)
            scroll.verticalScroller = system
            scroll.scrollerStyle = .overlay
            scroll.scrollerStyle = .legacy
            await T.settle(window, 100)  // capture-exempt: not a capture; AppKit's own scroller tiled in ours' place
            let theirs = parts(system)
            #expect(theirs.contains("fill 0") || theirs.contains("fill 1"), "AppKit's own has no filled knob to compare: \(theirs)")
            #expect(ours == theirs, "ours: \(ours); AppKit's: \(theirs)")
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
            let alpha = try #require(Self.knobAlpha(scroller), "AppKit's scroller part has no knobAlpha to read")
            if let event = Fling.wheel(0, phase: (4, 0)) { scroll.scrollWheel(with: event) }
            #expect(alpha > 0.5, "mid-scroll, the overlay knob's opacity is \(alpha)")
        }

        @Test("VoiceOver reads the knob's place and sets it, in a live scroll that ends")
        func voiceOverReadsAndMoves() async throws {
            let (window, scroll, scroller) = try await Self.session()
            defer { window.close() }
            var started = 0
            var ended = 0
            let center = NotificationCenter.default
            let a = center.addObserver(forName: NSScrollView.willStartLiveScrollNotification, object: scroll, queue: nil) { _ in started += 1 }
            let b = center.addObserver(forName: NSScrollView.didEndLiveScrollNotification, object: scroll, queue: nil) { _ in ended += 1 }
            defer { center.removeObserver(a); center.removeObserver(b) }
            #expect(scroller.accessibilityRole() == .scrollBar)
            let read = try #require(scroller.accessibilityValue() as? NSNumber)
            #expect(abs(read.doubleValue - scroller.doubleValue) < 0.001)
            await Self.park(scroller, at: 0.5, window)
            #expect(abs(scroller.doubleValue - 0.5) < 0.01, "set to half, the knob is at \(scroller.doubleValue)")
            #expect(abs(Self.systemPlace(scroll) - 0.5) < 0.1)
            let now = try #require(scroller.accessibilityValue() as? NSNumber)
            #expect(abs(now.doubleValue - 0.5) < 0.01)
            #expect(!LiveScroll.inFlight, "VoiceOver's value left a live scroll open")

            // AppKit's own scroller takes no increment or decrement (it
            // answers false, as the system's does in any scroll view):
            // VoiceOver pages the scroll area itself.
            #expect(!scroller.accessibilityPerformIncrement())
            #expect(started == 1 && ended == 1, "\(started) live scrolls began, \(ended) ended")
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
