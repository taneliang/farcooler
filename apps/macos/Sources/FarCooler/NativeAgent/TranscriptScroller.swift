import AgentKit
import AppKit
import ObjectiveC
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
///
/// Estimates are made at the transcript's width (the clip's, less its
/// padding), never a row's: a thinking row or a task list is as narrow as
/// its words. Heights laid out at another width are scaled by their
/// estimates' ratio until the row is laid out again.
@MainActor
final class TranscriptLayout {
    /// The coordinate space rows report their frames in: the content's own,
    /// which is the scroll view's document.
    static let space = "native-transcript-content"
    /// The stack's padding on each side, inside the clip.
    static let padding = Spacing.section

    /// The stack's spacing between items.
    let spacing: CGFloat
    /// Where the items come from, read as the scroller asks.
    let store: AgentRowStore

    weak var scroller: TranscriptScroller?
    private var scroll: NSScrollView? { scroller?.superview as? NSScrollView }

    private(set) var items: [ConversationItem] = []
    private var index: [String: Int] = [:]
    /// What an item's estimate is made from, worked out once.
    private var shapes: [String: Shape] = [:]
    /// Heights as laid out, by item id, and the width they were laid out at.
    private var measured: [String: (height: CGFloat, width: CGFloat)] = [:]
    /// The transcript's width the sums below were made at.
    private var width: CGFloat = 0
    /// Each item's height and the spacing after it, at `width`, and their
    /// running sums (`Sums`), so a row measured updates its own entry.
    private var sums = Sums()
    private var built = false
    /// The items built now, where the stack put them.
    private var frames: [String: CGRect] = [:]
    private var head: CGFloat?
    /// Below the last item: the queued lines, a handoff row, the padding.
    private var tail: CGFloat = Spacing.section
    /// The last knob worked out, and what it was worked out from: the scroll
    /// view sets the scroller's value and size separately on each scroll.
    private var cached: (key: [CGFloat], knob: Knob?)?

    init(store: AgentRowStore, spacing: CGFloat) {
        self.store = store
        self.spacing = spacing
    }

    // MARK: Keeping up

    /// The store's items, if they changed: an array the store hasn't set
    /// again is the same buffer, so most calls compare two pointers.
    private func sync() {
        let now = store.items
        let same = now.count == items.count
            && now.withUnsafeBufferPointer { a in items.withUnsafeBufferPointer { b in a.baseAddress == b.baseAddress } }
        guard !same, now != items else { return }
        items = now
        index = Dictionary(now.enumerated().map { ($1.id, $0) }, uniquingKeysWith: { a, _ in a })
        built = false
        cached = nil
        if head != nil, frames[now.first?.id ?? ""] == nil { head = nil }
    }

    /// The transcript's width now: the clip's, less the stack's padding.
    private func contentWidth() -> CGFloat? {
        guard let scroll else { return nil }
        let insets = scroll.contentInsets
        let width = scroll.contentView.bounds.width - insets.left - insets.right - Self.padding * 2
        return width > 0 ? width : nil
    }

    /// The sums at the transcript's width now, built again when it changed.
    private func ensureSums() {
        sync()
        if let now = contentWidth(), abs(now - width) > 0.5 {
            width = now
            built = false
        }
        guard !built else { return }
        sums = Sums(items.indices.map { height($0) + spacing })
        built = true
        cached = nil
    }

    /// An item laid out at `frame`, in the content's space.
    func place(_ id: String, _ frame: CGRect) {
        frames[id] = frame
        cached = nil
        let width = contentWidth() ?? self.width
        if measured[id].map({ abs($0.height - frame.height) > 0.25 || abs($0.width - width) > 0.5 }) ?? true {
            measured[id] = (frame.height, width)
            // Its own entry, if the sums are at this width; otherwise they
            // are built again on the next ask.
            if built, abs(width - self.width) <= 0.5, let i = index[id] {
                sums.set(i, frame.height + spacing)
            }
        }
        if id == items.first?.id { head = frame.minY }
        scroller?.refresh()
    }

    /// An item the stack let go of: its height is kept.
    func drop(_ id: String) {
        frames[id] = nil
        cached = nil
    }

    private func height(_ i: Int) -> CGFloat {
        let item = items[i]
        guard let m = measured[item.id] else { return estimate(item, at: width) }
        if abs(m.width - width) <= 0.5 { return m.height }
        // Laid out at another width: scaled as its estimate scales.
        let then = estimate(item, at: m.width)
        return then > 0 ? m.height * estimate(item, at: width) / then : m.height
    }

    private func estimate(_ item: ConversationItem, at width: CGFloat) -> CGFloat {
        let shape = shapes[item.id] ?? {
            let shape = Shape(item, store: store)
            shapes[item.id] = shape
            return shape
        }()
        return shape.height(at: width)
    }

    // MARK: The knob

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
    /// heights: nil before any item is laid out, or when the transcript
    /// fits.
    func knob(_ scroll: NSScrollView) -> Knob? {
        guard let doc = scroll.documentView else { return nil }
        ensureSums()
        let clip = scroll.contentView.bounds
        let insets = scroll.contentInsets
        let key = [clip.minY, clip.height, clip.width, doc.frame.height, insets.top, insets.bottom]
        if let cached, cached.key == key { return cached.knob }
        let knob = work(scroll, doc: doc)
        cached = (key, knob)
        return knob
    }

    private func work(_ scroll: NSScrollView, doc: NSView) -> Knob? {
        guard !items.isEmpty, let anchor = anchor(near: scroll.contentView.bounds.minY) else { return nil }
        let clip = scroll.contentView.bounds
        let insets = scroll.contentInsets
        let y = clip.minY
        let head = self.head ?? headGuess
        if let last = items.last, let frame = frames[last.id] { tail = max(0, doc.frame.height - frame.maxY) }
        let place = head + sums.prefix(anchor.i) + (y - anchor.frame.minY)
        let total = content(head) + insets.top + insets.bottom
        let range = total - clip.height
        guard range > 1 else { return nil }
        // At an end of the real content, the knob is at that end of its track.
        let realRange = doc.frame.height + insets.top + insets.bottom - clip.height
        let proportion = min(1, clip.height / total)
        if y + insets.top >= realRange - 0.5 { return Knob(value: 1, proportion: proportion, place: range) }
        if y + insets.top <= 0.5 { return Knob(value: 0, proportion: proportion, place: 0) }
        let value = Double(min(1, max(0, (place + insets.top) / range)))
        return Knob(value: value, proportion: proportion, onScreen: anchor.distance < clip.height, place: place + insets.top)
    }

    /// The space above the first item while it isn't built: the stack's
    /// padding, and the older-rows spinner when there is one.
    private var headGuess: CGFloat { Self.padding + (store.moreBefore ? 16 + spacing : 0) }

    /// The whole transcript's height in our heights.
    private func content(_ head: CGFloat) -> CGFloat {
        head + sums.prefix(items.count) - spacing + tail
    }

    /// The built item nearest `y`, and how far from it `y` is.
    private func anchor(near y: CGFloat) -> (i: Int, frame: CGRect, distance: CGFloat)? {
        var anchor: (i: Int, frame: CGRect, distance: CGFloat)?
        for (id, frame) in frames {
            guard let i = index[id] else { continue }
            let distance = y < frame.minY ? frame.minY - y : y > frame.maxY ? y - frame.maxY : 0
            if anchor.map({ distance < $0.distance }) ?? true { anchor = (i, frame, distance) }
        }
        return anchor
    }

    /// The first built item on screen from the clip's top, and how far
    /// into it the clip is (negative when the clip's top is above it).
    func top(_ scroll: NSScrollView) -> (id: String, into: CGFloat)? {
        let y = scroll.contentView.bounds.minY
        let shown = frames.filter { $0.value.maxY > y }
        return shown.min { $0.value.minY < $1.value.minY }.map { ($0.key, y - $0.value.minY) }
    }

    /// The clip's top for the knob at `value`: through the item there if it's
    /// built, else from the nearest built item by our heights.
    func origin(for value: Double, in scroll: NSScrollView) -> CGFloat? {
        guard let doc = scroll.documentView else { return nil }
        ensureSums()
        guard !items.isEmpty, !frames.isEmpty else { return nil }
        let clip = scroll.contentView.bounds
        let insets = scroll.contentInsets
        let head = self.head ?? headGuess
        let range = content(head) + insets.top + insets.bottom - clip.height
        let place = CGFloat(value) * range - insets.top
        // The item `place` falls in: the one after all those that end above
        // it.
        let at = max(0, min(items.count - 1, sums.count(upTo: place - head)))
        let through: (i: Int, frame: CGRect)? = frames[items[at].id].map { (at, $0) }
            ?? anchor(near: clip.minY).map { ($0.i, $0.frame) }
        guard let through else { return nil }
        let y = through.frame.minY + (place - head - sums.prefix(through.i))
        let realRange = doc.frame.height + insets.top + insets.bottom - clip.height
        return min(max(y, -insets.top), realRange - insets.top)
    }

    // MARK: Estimates

    /// What a row's height is estimated from: its kind and its text's
    /// paragraphs, by length. Worked out once a row; its height at a width
    /// is arithmetic.
    struct Shape {
        enum Kind {
            case prose, bubble(images: Bool), notice, fixed(CGFloat)
        }

        var kind: Kind
        var paragraphs: [Int] = []

        @MainActor init(_ item: ConversationItem, store: AgentRowStore) {
            // A run of tool calls is one line.
            guard case .row(let id) = item else {
                kind = .fixed(15)
                return
            }
            guard let row = store.box(id)?.row else {
                kind = .fixed(0)
                return
            }
            func counts(_ text: String) -> [Int] {
                text.split(separator: "\n", omittingEmptySubsequences: false).map(\.count)
            }
            switch row.kind {
            case .prose(let prose):
                kind = .prose
                paragraphs = counts(prose.text)
            case .turn(let turn):
                if let harness = turn.harness {
                    // A status line, a note, or a report folded after a few lines.
                    kind = .fixed(harness.kind == "Status" ? 16 : harness.kind == "Note" ? 44 : 114)
                } else if AgentConversation.isScheduled(turn) {
                    kind = .fixed(60)
                } else if NativeCopy.isNotice(turn) {
                    kind = .notice
                    paragraphs = counts(AgentConversation.noticeText(turn))
                } else if turn.prompt.isEmpty {
                    kind = .fixed(16 + (turn.images.isEmpty ? 0 : 76))
                } else {
                    kind = .bubble(images: !turn.images.isEmpty)
                    paragraphs = counts(turn.prompt)
                }
            case .tasks(let tasks):
                kind = .fixed(CGFloat(tasks.items.count) * 20 + 13)
            case .thinking:
                kind = .fixed(13)
            case .hint, .unknown:
                kind = .fixed(0)
            default:
                kind = .fixed(20)
            }
        }

        /// Lines of `perLine` characters the paragraphs take.
        private func lines(perLine: CGFloat) -> CGFloat {
            let perLine = max(20, perLine)
            return paragraphs.reduce(0) { $0 + max(1, (CGFloat($1) / perLine).rounded(.up)) }
        }

        /// Fitted to `RealSession`'s rows at 688 pt, and kept to the
        /// reading measures the rows themselves keep: prose at most 680 pt
        /// wide (`AgentReplyText`), the person's bubble at most 560 pt with
        /// its padding (`YourBubble`).
        func height(at width: CGFloat) -> CGFloat {
            switch kind {
            case .fixed(let height): return height
            case .prose: return lines(perLine: min(width, 680) / 6.3) * 17
            case .notice:
                let shown = lines(perLine: width / 6.4)
                return shown > 3 ? 114 : shown * 15 + 22
            case .bubble(let images):
                // The bubble, the outcome line under it, and its photos.
                let text = (min(width, 560) - Spacing.inset * 2) / 6.7
                return 17 + lines(perLine: text) * 16 + 16 + (images ? 76 : 0)
            }
        }
    }

    /// Running sums over the items' heights (a Fenwick tree): one entry set,
    /// a prefix summed, or a place found, in log time.
    struct Sums {
        private var tree: [CGFloat] = [0]
        private var values: [CGFloat] = []

        init() {}

        init(_ values: [CGFloat]) {
            self.values = values
            tree = [0] + values
            for i in tree.indices.dropFirst() {
                let parent = i + (i & -i)
                if parent < tree.count { tree[parent] += tree[i] }
            }
        }

        mutating func set(_ i: Int, _ value: CGFloat) {
            guard values.indices.contains(i) else { return }
            let change = value - values[i]
            values[i] = value
            var at = i + 1
            while at < tree.count {
                tree[at] += change
                at += at & -at
            }
        }

        /// The first `n` values' sum.
        func prefix(_ n: Int) -> CGFloat {
            var sum: CGFloat = 0
            var at = min(n, values.count)
            while at > 0 {
                sum += tree[at]
                at -= at & -at
            }
            return sum
        }

        /// How many values from the first add up to `limit` or less.
        func count(upTo limit: CGFloat) -> Int {
            var at = 0
            var left = limit
            var step = 1
            while step * 2 < tree.count { step *= 2 }
            while step > 0 {
                if at + step < tree.count, tree[at + step] <= left {
                    at += step
                    left -= tree[at]
                }
                step /= 2
            }
            return at
        }
    }
}

/// The transcript's vertical scroller: the system's, placed from
/// `TranscriptLayout` rather than from the scroll view's content size
/// (ov-701). Dragging its knob, clicking its track and VoiceOver scroll
/// through the same heights, so the knob stays under the pointer.
final class TranscriptScroller: NSScroller {
    weak var layout: TranscriptLayout?
    /// Where the pointer, or VoiceOver, put the knob. The clip is scrolled
    /// there through our heights, but rows the stack hadn't built are laid
    /// out at its own guess, a frame later: so until the clip's place in
    /// our heights agrees, the knob stays here and the clip is put right
    /// again each frame (`settle`), for at most `settleFor` after the
    /// pointer lets go.
    private var aimed: Double?
    private var settleUntil: CFTimeInterval = 0
    private var settler: Timer?
    static let settleFor: CFTimeInterval = 0.4
    /// The pointer is held down on the scroller.
    private var dragging = false

    /// AppKit draws a subclass of `NSScroller` in the old style (a hairline
    /// track and a thin knob, where the system's has a filled track and a
    /// wider knob), deciding by the class the scroller says it is. Ours
    /// draws nothing of its own, so it says it is AppKit's; Swift's own
    /// casts read the class itself, not this.
    private static let drawnAsTheSystems: Void = {
        let block: @convention(block) (AnyObject) -> AnyClass = { _ in NSScroller.self }
        class_addMethod(TranscriptScroller.self, NSSelectorFromString("class"), imp_implementationWithBlock(block), "#@:")
    }()

    override init(frame: NSRect) {
        _ = Self.drawnAsTheSystems
        super.init(frame: frame)
    }

    required init?(coder: NSCoder) {
        _ = Self.drawnAsTheSystems
        super.init(coder: coder)
    }

    /// Drawn by the system, so it is as good as the system's on overlay
    /// scroll bars.
    override class var isCompatibleWithOverlayScrollers: Bool { true }

    // The scroll view sets these from its content on every scroll; the
    // knob takes ours instead, or the scroll view's when ours can't be
    // worked out. AppKit's own setters call each other, which go straight
    // through.
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

    /// A move of the knob larger than this with nothing aimed is shown a
    /// run-loop turn later, once it's still there: the scroll view can set
    /// the clip for a new width or a resized stack before the rows report
    /// where they went, and the knob would flick away and back for a frame.
    /// A two-finger scroll moves it far less a frame.
    static let jumpHeld = 0.01
    private var held = false
    /// While the window's width changes, the stack builds rows again at the
    /// new width and lets others go, a frame or two apart, and a row on
    /// screen can report a place from the old width: so the knob holds
    /// still until the width has held for `widthSettle`, then goes where
    /// the reader is, as the system's does when a resize ends.
    static let widthSettle: CFTimeInterval = 0.25
    private var width: CGFloat = 0
    private var widthChanged: CFTimeInterval = 0
    private var widthCheck = false

    /// Place the knob again, from the layout's heights.
    func refresh(fallback: (value: Double?, proportion: CGFloat?) = (nil, nil), confirmed: Bool = false) {
        guard !setting else { return }
        setting = true
        defer { setting = false }
        let knob = scrollView.flatMap { layout?.knob($0) }
        var value = aimed ?? knob?.value ?? fallback.value
        var proportion = knob?.proportion ?? fallback.proportion
        if let clip = scrollView?.contentView.bounds.width, abs(clip - width) > 0.5 {
            if width > 0 { widthChanged = CACurrentMediaTime() }
            width = clip
        }
        if aimed == nil, CACurrentMediaTime() - widthChanged < Self.widthSettle, super.doubleValue > 0 {
            value = super.doubleValue
            proportion = super.knobProportion
            if !widthCheck {
                widthCheck = true
                checkWidth()
            }
        } else if aimed == nil, !confirmed, let knob, abs(knob.value - super.doubleValue) > Self.jumpHeld, super.doubleValue > 0 || knob.value < 1 {
            value = super.doubleValue
            if !held {
                held = true
                DispatchQueue.main.async { [weak self] in
                    guard let self else { return }
                    held = false
                    refresh(confirmed: true)
                }
            }
        }
        if let value, super.doubleValue != value { super.doubleValue = value }
        if let proportion, super.knobProportion != proportion { super.knobProportion = proportion }
    }

    /// Once the width has held for `widthSettle`, the knob goes where the
    /// reader is.
    private func checkWidth() {
        let left = Self.widthSettle - (CACurrentMediaTime() - widthChanged)
        DispatchQueue.main.asyncAfter(deadline: .now() + max(0.02, left)) { [weak self] in
            guard let self else { return }
            if CACurrentMediaTime() - widthChanged < Self.widthSettle { return checkWidth() }
            widthCheck = false
            refresh(confirmed: true)
        }
    }

    // MARK: Aiming

    /// Hold the knob at `value` and scroll there, in a live scroll, putting
    /// the clip right each frame until it agrees.
    private func aim(_ value: Double, _ scroll: NSScrollView) {
        beginLive(scroll)
        aimed = value
        settleUntil = CACurrentMediaTime() + Self.settleFor
        scrollTo(value, scroll)
        refresh()
        if settler == nil {
            let timer = Timer(timeInterval: 1.0 / 60, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated { self?.settle() }
            }
            RunLoop.main.add(timer, forMode: .common)
            settler = timer
        }
        settle()
    }

    /// A frame's check of the aim: the clip put right while the knob and
    /// the clip disagree; let go once they agree on rows on screen, when
    /// there's no knob to agree with (a transcript that fits), or once
    /// `settleFor` has passed since the pointer let go.
    private func settle() {
        guard let aimed, let scroll = scrollView else { return release() }
        if dragging { settleUntil = CACurrentMediaTime() + Self.settleFor }
        guard let knob = layout?.knob(scroll) else { return release() }
        if abs(knob.value - aimed) > 0.0005 {
            scrollTo(aimed, scroll)
        } else if !dragging, knob.onScreen {
            return release()
        }
        if !dragging, CACurrentMediaTime() > settleUntil { release() }
    }

    /// The knob goes back to where the clip is, and the live scroll ends.
    private func release() {
        aimed = nil
        settler?.invalidate()
        settler = nil
        refresh()
        endLive()
    }

    /// The scroll view in a live scroll of ours. The stack keeps the rows on
    /// screen through its resizes only in one: a clip moved outside one, as
    /// AppKit's own scroller moves it, is left where it was, and the rows
    /// slide under it. None of ours inside AppKit's own (a coast still
    /// going), whose end is AppKit's to say.
    private var live = false

    private func beginLive(_ scroll: NSScrollView) {
        guard !live, !LiveScroll.scrolls(scroll) else { return }
        live = true
        NotificationCenter.default.post(name: NSScrollView.willStartLiveScrollNotification, object: scroll)
    }

    private func endLive() {
        guard live, !dragging, aimed == nil else { return }
        live = false
        if let scroll = scrollView {
            NotificationCenter.default.post(name: NSScrollView.didEndLiveScrollNotification, object: scroll)
        }
    }

    // MARK: The pointer

    /// "Jump to the spot that's clicked", from Appearance settings.
    static let systemJumpsToClick: @MainActor () -> Bool = { UserDefaults.standard.bool(forKey: "AppleScrollerPagingBehavior") }
    /// Read on each click; a test sets it.
    static var jumpsToClick = systemJumpsToClick

    override func mouseDown(with event: NSEvent) {
        guard let scroll = scrollView, layout != nil else { return super.mouseDown(with: event) }
        let point = convert(event.locationInWindow, from: nil)
        // In the window's coordinates, as AppKit's own is asked.
        switch testPart(event.locationInWindow) {
        case .knob:
            drag(from: point, in: scroll)
        case .decrementPage, .incrementPage:
            // "Jump to the spot that's clicked" in Appearance, or Option for
            // the other one, as every scroll bar does.
            if Self.jumpsToClick() != event.modifierFlags.contains(.option) {
                aim(value(knobTop: along(point) - rect(for: .knob).height / 2), scroll)
                drag(from: point, in: scroll)
            } else {
                page(from: point, in: scroll)
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

    private func value(knobTop: CGFloat) -> Double {
        let travel = rect(for: .knobSlot).height - rect(for: .knob).height
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

    private func drag(from point: NSPoint, in scroll: NSScrollView) {
        let grab = along(point) - knobTop()
        beginLive(scroll)
        dragging = true
        track(.leftMouseDragged) { event in
            aim(value(knobTop: along(convert(event.locationInWindow, from: nil)) - grab), scroll)
            NotificationCenter.default.post(name: NSScrollView.didLiveScrollNotification, object: scroll)
        }
        dragging = false
        // Let go at once: a press with no drag aimed at nothing.
        if aimed == nil { endLive() } else { settle() }
    }

    /// A page at a time toward the pointer while it's held in the track, as
    /// the system's does.
    private func page(from point: NSPoint, in scroll: NSScrollView) {
        var pointer = along(point)
        func step() {
            let knob = knobTop()
            guard pointer < knob || pointer > knob + rect(for: .knob).height else { return }
            self.step(pointer < knob ? -1 : 1, scroll)
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

    /// A page up (-1) or down (1): the window's height less the context the
    /// scroll view keeps.
    private func step(_ direction: CGFloat, _ scroll: NSScrollView) {
        let clip = scroll.contentView.bounds
        let insets = scroll.contentInsets
        let page = max(1, clip.height - scroll.verticalPageScroll)
        let real = (scroll.documentView?.frame.height ?? 0) + insets.top + insets.bottom - clip.height
        let y = min(max(clip.minY + direction * page, -insets.top), real - insets.top)
        scroll.contentView.scroll(to: NSPoint(x: clip.minX, y: y))
        scroll.reflectScrolledClipView(scroll.contentView)
        NotificationCenter.default.post(name: NSScrollView.didLiveScrollNotification, object: scroll)
    }

    /// Scroll to the knob at `value`, through our heights, or in proportion
    /// to the content when ours can't say.
    private func scrollTo(_ value: Double, _ scroll: NSScrollView) {
        let clip = scroll.contentView.bounds
        let insets = scroll.contentInsets
        let real = (scroll.documentView?.frame.height ?? 0) + insets.top + insets.bottom - clip.height
        let y = layout?.origin(for: value, in: scroll) ?? (CGFloat(value) * max(0, real) - insets.top)
        guard abs(clip.minY - y) > 0.5 else { return }
        scroll.contentView.scroll(to: NSPoint(x: clip.minX, y: y))
        scroll.reflectScrolledClipView(scroll.contentView)
    }

    // MARK: VoiceOver

    override func setAccessibilityValue(_ value: Any?) {
        guard let scroll = scrollView, let number = value as? NSNumber else { return super.setAccessibilityValue(value) }
        aim(min(1, max(0, number.doubleValue)), scroll)
    }

    /// A page down, or up, in a live scroll of its own.
    override func accessibilityPerformIncrement() -> Bool {
        guard let scroll = scrollView else { return super.accessibilityPerformIncrement() }
        pageForVoiceOver(1, scroll)
        return true
    }

    override func accessibilityPerformDecrement() -> Bool {
        guard let scroll = scrollView else { return super.accessibilityPerformDecrement() }
        pageForVoiceOver(-1, scroll)
        return true
    }

    private func pageForVoiceOver(_ direction: CGFloat, _ scroll: NSScrollView) {
        beginLive(scroll)
        step(direction, scroll)
        endLive()
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
            // A scroller set in place of the scroll view's own isn't shown
            // by an overlay scroll (its knob stays clear) until the style
            // is set again, which pairs it with the scroll view anew.
            let style = scroll.scrollerStyle
            scroll.scrollerStyle = style == .overlay ? .legacy : .overlay
            scroll.scrollerStyle = style
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
