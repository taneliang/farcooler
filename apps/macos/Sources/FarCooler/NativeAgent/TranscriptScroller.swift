import AgentKit
import AppKit
import SwiftUI

/// Where the transcript's scroller knob goes (ov-701).
///
/// The lazy stack sizes the rows it hasn't built at the mean of the ones it
/// has, and when it has built enough to know better it sizes its content
/// again in one frame: flung up `RealSession`, from 47,285 pt to about
/// 38,600. The rows on screen hold still through that; the system scroller's
/// knob, which is the clip's place in the content, jumped 18% of its track
/// (ov-699).
///
/// So the knob is placed from heights of our own: each row's as it was
/// laid out, where it has been, and an estimate from its text where it
/// hasn't. A row's estimate is replaced by its height as the row comes on
/// screen, so the knob's place in the whole moves by that one row's error,
/// a fraction of a percent, never by the stack's correction of all of them.
///
/// The reader's place in our heights is found from a row on screen: the
/// top of the clip is so far into that row, and the row is the sum of the
/// rows above it into ours. The scroller is the system's own `NSScroller`,
/// given that place instead of the scroll view's (`TranscriptScroller`), so
/// it looks, fades and follows "Show scroll bars" as every other does.
@MainActor
final class TranscriptLayout {
    /// The coordinate space rows report their frames in: the content's own,
    /// which is the scroll view's document.
    static let space = "native-transcript-content"

    /// The stack's spacing between items.
    let spacing: CGFloat
    /// The rows' text sizes, for the estimate.
    var store: AgentRowStore?
    /// The space above the first item while it isn't built: the stack's
    /// padding, and the older-rows spinner when there is one.
    var headGuess: CGFloat = Spacing.section

    private(set) var items: [ConversationItem] = []
    private var index: [String: Int] = [:]
    /// Heights as laid out, by item id, and the width they were laid out at.
    private var measured: [String: CGFloat] = [:]
    private var guessed: [String: CGFloat] = [:]
    private var width: CGFloat = 0
    /// The items built now, where the stack put them.
    private var frames: [String: CGRect] = [:]
    /// `tops[i]`: where item `i` starts, below the first item's top, in our
    /// heights; `tops[count]` is past the last item's spacing.
    private var tops: [CGFloat] = []
    private var head: CGFloat?
    /// Below the last item: the queued lines, a handoff row, the padding.
    private var tail: CGFloat = Spacing.section

    weak var scroller: TranscriptScroller?

    init(spacing: CGFloat) {
        self.spacing = spacing
    }

    /// The items in order, as the stack draws them.
    func set(items: [ConversationItem]) {
        guard items != self.items else { return }
        self.items = items
        index = Dictionary(items.enumerated().map { ($1.id, $0) }, uniquingKeysWith: { a, _ in a })
        tops = []
        if head != nil, frames[items.first?.id ?? ""] == nil { head = nil }
    }

    /// An item laid out at `frame`, in the content's space.
    func place(_ id: String, _ frame: CGRect) {
        frames[id] = frame
        if abs(frame.width - width) > 0.5 {
            width = frame.width
            guessed = [:]
        }
        if measured[id].map({ abs($0 - frame.height) > 0.25 }) ?? true {
            measured[id] = frame.height
            tops = []
        }
        if id == items.first?.id { head = frame.minY }
        scroller?.refresh()
    }

    /// An item the stack let go of: its height is kept.
    func drop(_ id: String) {
        frames[id] = nil
    }

    private func height(_ i: Int) -> CGFloat {
        let item = items[i]
        if let h = measured[item.id] { return h }
        if let h = guessed[item.id] { return h }
        let h = Self.estimate(item, store: store, width: width)
        guessed[item.id] = h
        return h
    }

    private func ensureTops() {
        guard tops.count != items.count + 1 else { return }
        tops = [0]
        tops.reserveCapacity(items.count + 1)
        var y: CGFloat = 0
        for i in items.indices {
            y += height(i) + spacing
            tops.append(y)
        }
    }

    /// Where the knob goes: its place on its track, 0 to 1, its share of
    /// the track, and whether a row on screen placed it, rather than one
    /// the stack built somewhere else (the clip just moved far).
    struct Knob {
        var value: Double
        var proportion: CGFloat
        var onScreen = true
        /// The clip's top in our heights, from the content's top.
        var place: CGFloat = 0
    }

    /// The knob's place and size for the scroll view as it is now, from our
    /// heights: nil before any item is laid out.
    func knob(_ scroll: NSScrollView) -> Knob? {
        guard let doc = scroll.documentView, !items.isEmpty, !frames.isEmpty else { return nil }
        ensureTops()
        let clip = scroll.contentView.bounds
        let insets = scroll.contentInsets
        let y = clip.minY
        // The built item nearest the clip's top.
        var anchor: (i: Int, frame: CGRect, distance: CGFloat)?
        for (id, frame) in frames {
            guard let i = index[id] else { continue }
            let distance = y < frame.minY ? frame.minY - y : y > frame.maxY ? y - frame.maxY : 0
            if anchor.map({ distance < $0.distance }) ?? true { anchor = (i, frame, distance) }
        }
        guard let anchor else { return nil }
        let head = self.head ?? headGuess
        if let last = items.last, let frame = frames[last.id] { tail = max(0, doc.frame.height - frame.maxY) }
        let place = head + tops[anchor.i] + (y - anchor.frame.minY)
        let content = head + tops[items.count] - spacing + tail
        let total = content + insets.top + insets.bottom
        let range = total - clip.height
        guard range > 1 else { return nil }
        // At an end of the real content, the knob is at that end of its track.
        let realRange = doc.frame.height + insets.top + insets.bottom - clip.height
        let proportion = min(1, clip.height / total)
        if y + insets.top >= realRange - 0.5 { return Knob(value: 1, proportion: proportion, place: range) }
        if y + insets.top <= 0.5 { return Knob(value: 0, proportion: proportion, place: 0) }
        return Knob(
            value: Double(min(1, max(0, (place + insets.top) / range))), proportion: proportion, onScreen: anchor.distance < clip.height,
            place: place + insets.top)
    }

    /// The built item at the clip's top, and how far into it the clip is.
    func top(_ scroll: NSScrollView) -> (id: String, into: CGFloat)? {
        let y = scroll.contentView.bounds.minY
        return frames.filter { $0.value.minY <= y + 0.5 && $0.value.maxY + spacing > y }.min { $0.value.minY > $1.value.minY }
            .map { ($0.key, y - $0.value.minY) }
    }

    /// The clip's top for the knob at `value`: through the item there if it's
    /// built, else from the nearest built item by our heights.
    func origin(for value: Double, in scroll: NSScrollView) -> CGFloat? {
        guard let doc = scroll.documentView, !items.isEmpty, !frames.isEmpty else { return nil }
        ensureTops()
        let clip = scroll.contentView.bounds
        let insets = scroll.contentInsets
        let head = self.head ?? headGuess
        let content = head + tops[items.count] - spacing + tail
        let range = content + insets.top + insets.bottom - clip.height
        let place = CGFloat(value) * range - insets.top
        // The last item starting at or above `place`.
        var lo = 0, hi = items.count - 1
        while lo < hi {
            let mid = (lo + hi + 1) / 2
            if head + tops[mid] <= place { lo = mid } else { hi = mid - 1 }
        }
        let through: (i: Int, frame: CGRect)? =
            frames[items[lo].id].map { (lo, $0) }
            ?? frames.compactMap { id, frame in index[id].map { ($0, frame) } }.min { abs($0.0 - lo) < abs($1.0 - lo) }
        guard let through else { return nil }
        let y = through.frame.minY + (place - head - tops[through.i])
        let realRange = doc.frame.height + insets.top + insets.bottom - clip.height
        return min(max(y, -insets.top), realRange - insets.top)
    }

    /// A row's height without building it: its text's lines at the width,
    /// as body text sets them, and its chrome, fitted to `RealSession`'s
    /// rows. Only its error moves the knob, once, as the row comes on
    /// screen.
    static func estimate(_ item: ConversationItem, store: AgentRowStore?, width: CGFloat) -> CGFloat {
        let width = width > 0 ? width : 640
        func lines(_ text: String, perLine: CGFloat) -> CGFloat {
            text.split(separator: "\n", omittingEmptySubsequences: false).reduce(0) { $0 + max(1, (CGFloat($1.count) / max(20, perLine)).rounded(.up)) }
        }
        guard case .row(let id) = item else { return 16 }
        guard let row = store?.box(id)?.row else { return 0 }
        switch row.kind {
        case .prose(let prose): return lines(prose.text, perLine: width / 7.4) * 17
        case .turn(let turn):
            if let harness = turn.harness {
                // A status line, a note, or a report folded after a few lines.
                switch harness.kind {
                case "Status": return 16
                case "Note": return 44
                default: return 114
                }
            }
            if AgentConversation.isScheduled(turn) { return 60 }
            // Folded at three caption lines.
            if NativeCopy.isNotice(turn) {
                let shown = lines(AgentConversation.noticeText(turn), perLine: width / 6.4)
                return shown > 3 ? 114 : shown * 15 + 22
            }
            // The bubble, the outcome line under it, and its photos.
            let text = turn.prompt.isEmpty ? 0 : 17 + lines(turn.prompt, perLine: width / 8.6) * 16
            return text + 16 + (turn.images.isEmpty ? 0 : 76)
        case .hint, .unknown: return 0
        default: return 20
        }
    }
}

/// The transcript's vertical scroller: the system's, placed from
/// `TranscriptLayout` rather than from the scroll view's content size
/// (ov-701). Dragging its knob and clicking its track scroll through the
/// same heights, so the knob stays under the pointer.
final class TranscriptScroller: NSScroller {
    weak var layout: TranscriptLayout?
    /// Where the pointer, or VoiceOver, put the knob. The clip is scrolled
    /// there through our heights, but rows the stack hadn't built are laid
    /// out at its own guess, a frame later: so until the clip's place in
    /// our heights agrees, the knob stays here and the clip is put right
    /// again (`settle`).
    private var aimed: Double?
    /// The pointer is held down on the scroller.
    private var dragging = false
    private var settling = false
    private var settles = 0

    /// Drawn by the system, so it is as good as the system's on overlay
    /// scroll bars.
    override class var isCompatibleWithOverlayScrollers: Bool { self == TranscriptScroller.self }

    // The scroll view sets these from its content on every scroll; the
    // knob takes ours instead, or the scroll view's before any row is laid
    // out. AppKit's own setters call each other, which go straight through.
    override var doubleValue: Double {
        get { super.doubleValue }
        set { if setting { super.doubleValue = newValue } else { refresh(fallback: (newValue, nil)) } }
    }

    override var floatValue: Float {
        get { super.floatValue }
        set { if setting { super.floatValue = newValue } else { refresh(fallback: (Double(newValue), nil)) } }
    }

    override var knobProportion: CGFloat {
        get { super.knobProportion }
        set { if setting { super.knobProportion = newValue } else { refresh(fallback: (nil, newValue)) } }
    }

    /// Inside our own setting of the knob.
    private var setting = false

    /// The scroll view this scrolls: its superview.
    private var scrollView: NSScrollView? { superview as? NSScrollView }

    /// Place the knob again, from the layout's heights.
    func refresh(fallback: (value: Double?, proportion: CGFloat?) = (nil, nil)) {
        guard !setting else { return }
        setting = true
        defer { setting = false }
        let knob = scrollView.flatMap { layout?.knob($0) }
        if let aimed, let knob {
            if abs(knob.value - aimed) > 0.0005 {
                if !settling {
                    settling = true
                    DispatchQueue.main.async { [weak self] in self?.settle() }
                }
            } else if !dragging, knob.onScreen {
                self.aimed = nil
                endLive()
            }
        }
        let value = aimed ?? knob?.value ?? fallback.value
        let proportion = knob?.proportion ?? fallback.proportion
        if let value, super.doubleValue != value { super.doubleValue = value }
        if let proportion, super.knobProportion != proportion { super.knobProportion = proportion }
    }

    /// The clip put where the knob is, now the rows there are built. A few
    /// tries after the pointer lets go, then the knob goes where the clip is.
    private func settle() {
        settling = false
        guard let aimed, let scroll = scrollView else { return }
        scrollTo(aimed, scroll)
        settles += 1
        if !dragging, settles > 4 {
            self.aimed = nil
            refresh()
            endLive()
        }
    }

    /// The scroll view in a live scroll of ours. The stack keeps the rows on
    /// screen through its resizes only in one: a clip moved outside one, as
    /// AppKit's own scroller moves it, is left where it was, and the rows
    /// slide under it.
    private var live = false

    private func beginLive(_ scroll: NSScrollView) {
        guard !live else { return }
        live = true
        NotificationCenter.default.post(name: NSScrollView.willStartLiveScrollNotification, object: scroll)
    }

    private func endLive() {
        guard live, !dragging, aimed == nil else { return }
        live = false
        if let scroll = scrollView { NotificationCenter.default.post(name: NSScrollView.didEndLiveScrollNotification, object: scroll) }
    }

    /// Hold the knob at `value` and scroll there.
    private func aim(_ value: Double, _ scroll: NSScrollView) {
        beginLive(scroll)
        aimed = value
        settles = 0
        scrollTo(value, scroll)
        refresh()
    }

    // MARK: The pointer

    /// "Jump to the spot that's clicked", from Appearance settings.
    static let systemJumpsToClick: @MainActor () -> Bool = { UserDefaults.standard.bool(forKey: "AppleScrollerPagingBehavior") }
    /// Read on each click; a test sets it.
    static var jumpsToClick = systemJumpsToClick

    override func mouseDown(with event: NSEvent) {
        guard let scroll = scrollView, let window, layout != nil else { return super.mouseDown(with: event) }
        let point = convert(event.locationInWindow, from: nil)
        // In the window's coordinates, as AppKit's own is asked.
        switch testPart(event.locationInWindow) {
        case .knob:
            drag(from: point, in: scroll, window: window)
        case .decrementPage, .incrementPage:
            // "Jump to the spot that's clicked" in Appearance, or Option for
            // the other one, as every scroll bar does.
            let jumps = Self.jumpsToClick() != event.modifierFlags.contains(.option)
            if jumps {
                let knob = rect(for: .knob)
                aim(value(knobTop: along(point) - length(knob) / 2), scroll)
                drag(from: point, in: scroll, window: window)
            } else {
                page(from: point, in: scroll, window: window)
            }
        default:
            super.mouseDown(with: event)
        }
    }

    /// Down the track, from its top.
    private func along(_ point: NSPoint) -> CGFloat {
        let slot = rect(for: .knobSlot)
        return isFlipped ? point.y - slot.minY : slot.maxY - point.y
    }

    private func length(_ rect: NSRect) -> CGFloat { rect.height }

    private func value(knobTop: CGFloat) -> Double {
        let travel = length(rect(for: .knobSlot)) - length(rect(for: .knob))
        return travel > 0 ? Double(min(1, max(0, knobTop / travel))) : 0
    }

    private func knobTop() -> CGFloat {
        let slot = rect(for: .knobSlot)
        let knob = rect(for: .knob)
        return isFlipped ? knob.minY - slot.minY : slot.maxY - knob.maxY
    }

    /// The pointer's next event of `kinds`, in the tracking mode, as AppKit's
    /// own controls take them. A test hands them over instead: outside
    /// `NSApp.run`, `nextEvent` ends the test process's main run loop.
    static let systemNextEvent: @MainActor (NSEvent.EventTypeMask) -> NSEvent? = {
        NSApp.nextEvent(matching: $0, until: .distantFuture, inMode: .eventTracking, dequeue: true)
    }
    static var nextEvent = systemNextEvent

    /// The pointer's events until it's let go.
    private func track(_ kinds: NSEvent.EventTypeMask, _ each: (NSEvent) -> Void) {
        while let event = Self.nextEvent(kinds.union(.leftMouseUp)), event.type != .leftMouseUp {
            each(event)
        }
    }

    private func drag(from point: NSPoint, in scroll: NSScrollView, window: NSWindow) {
        let grab = along(point) - knobTop()
        beginLive(scroll)
        dragging = true
        track(.leftMouseDragged) { event in
            aim(value(knobTop: along(convert(event.locationInWindow, from: nil)) - grab), scroll)
            NotificationCenter.default.post(name: NSScrollView.didLiveScrollNotification, object: scroll)
        }
        dragging = false
        refresh()
        endLive()
    }

    /// A page at a time toward the pointer while it's held in the track, as
    /// the system's does.
    private func page(from point: NSPoint, in scroll: NSScrollView, window: NSWindow) {
        var pointer = along(point)
        func step() {
            let knob = knobTop()
            let size = length(rect(for: .knob))
            guard pointer < knob || pointer > knob + size else { return }
            let page = max(1, scroll.contentView.bounds.height - scroll.verticalPageScroll)
            let clip = scroll.contentView.bounds
            let insets = scroll.contentInsets
            let real = (scroll.documentView?.frame.height ?? 0) + insets.top + insets.bottom - clip.height
            let y = min(max(clip.minY + (pointer < knob ? -page : page), -insets.top), real - insets.top)
            scroll.contentView.scroll(to: NSPoint(x: clip.minX, y: y))
            scroll.reflectScrolledClipView(scroll.contentView)
        }
        beginLive(scroll)
        dragging = true
        defer {
            dragging = false
            endLive()
        }
        step()
        NSEvent.startPeriodicEvents(afterDelay: NSEvent.keyRepeatDelay, withPeriod: 0.05)
        defer { NSEvent.stopPeriodicEvents() }
        track([.leftMouseDragged, .periodic]) { event in
            if event.type == .leftMouseDragged {
                pointer = along(convert(event.locationInWindow, from: nil))
            } else {
                step()
            }
        }
    }

    /// Scroll to the knob at `value`, through our heights.
    private func scrollTo(_ value: Double, _ scroll: NSScrollView) {
        guard let y = layout?.origin(for: value, in: scroll) else { return }
        let clip = scroll.contentView.bounds
        guard abs(clip.minY - y) > 0.5 else { return }
        scroll.contentView.scroll(to: NSPoint(x: clip.minX, y: y))
        scroll.reflectScrolledClipView(scroll.contentView)
    }

    // MARK: VoiceOver

    override func setAccessibilityValue(_ value: Any?) {
        guard let scroll = scrollView, let number = value as? NSNumber else { return super.setAccessibilityValue(value) }
        aim(min(1, max(0, number.doubleValue)), scroll)
    }
}

/// Hangs a `TranscriptScroller` on the scroll view this sits in.
struct TranscriptScrollerInstaller: NSViewRepresentable {
    let layout: TranscriptLayout

    func makeNSView(context: Context) -> Probe {
        let probe = Probe()
        probe.layout = layout
        return probe
    }

    func updateNSView(_ probe: Probe, context: Context) {
        probe.layout = layout
        probe.install()
    }

    final class Probe: NSView {
        var layout: TranscriptLayout?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            install()
        }

        func install() {
            guard let layout, let scroll = enclosingScrollView else { return }
            if let mine = scroll.verticalScroller as? TranscriptScroller, mine.layout === layout { return }
            let scroller = TranscriptScroller()
            scroller.layout = layout
            layout.scroller = scroller
            scroll.verticalScroller = scroller
            scroller.refresh()
        }
    }
}

extension View {
    /// The item's frame, for the scroller's heights (ov-701).
    func placed(_ id: String, in layout: TranscriptLayout) -> some View {
        onGeometryChange(for: CGRect.self) { $0.frame(in: .named(TranscriptLayout.space)) } action: { layout.place(id, $0) }
            .onDisappear { layout.drop(id) }
    }
}
