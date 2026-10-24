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

    /// What drives the scroll view's scroller from these heights: owned
    /// here, so it lives as long as the transcript does.
    var scroller: TranscriptScroller?
    private var scroll: NSScrollView? { scroller?.scroll }

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
    /// The transcript's width the frames were reported at. When it changes
    /// they are all let go, and only rows laid out again at the new width
    /// report: a row the stack keeps but doesn't lay out again would stay
    /// where the old width put it, a wrong place to read the reader from.
    private var framesWidth: CGFloat = 0
    private var head: CGFloat?
    /// Below the last item: the queued lines, a handoff row, the padding.
    private var tail: CGFloat = Spacing.section
    /// The last knob worked out, and what it was worked out from: the scroll
    /// view sets the scroller's value and size separately on each scroll.
    private var cached: (key: Geometry, knob: Knob?)?

    /// What a knob is worked out from, besides our heights.
    struct Geometry: Equatable {
        var y, height, width, document, top, bottom: CGFloat

        init(_ scroll: NSScrollView) {
            let clip = scroll.contentView.bounds
            let insets = scroll.contentInsets
            y = clip.minY
            height = clip.height
            width = clip.width
            document = scroll.documentView?.frame.height ?? 0
            top = insets.top
            bottom = insets.bottom
        }
    }

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
        let width = contentWidth() ?? self.width
        if abs(width - framesWidth) > 0.5 {
            frames = [:]
            framesWidth = width
        }
        frames[id] = frame
        cached = nil
        if measured[id].map({ abs($0.height - frame.height) > 0.25 || abs($0.width - width) > 0.5 }) ?? true {
            measured[id] = (frame.height, width)
            // Its own entry, if the sums are at this width; otherwise they
            // are built again on the next ask.
            if built, abs(width - self.width) <= 0.5, let i = index[id] {
                sums.set(i, frame.height + spacing)
            }
        }
        if id == items.first?.id { head = frame.minY }
        scroller?.needsApply = true
    }

    /// An item the stack let go of: its height is kept.
    func drop(_ id: String) {
        frames[id] = nil
        cached = nil
        scroller?.needsApply = true
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
        /// The track's length in our heights: the knob's travel, in points
        /// of the content.
        var range: CGFloat = 1
    }

    /// The knob's place and size for the scroll view as it is now, from our
    /// heights: nil before any item is laid out, or when the transcript
    /// fits.
    func knob(_ scroll: NSScrollView) -> Knob? {
        guard let doc = scroll.documentView else { return nil }
        ensureSums()
        let key = Geometry(scroll)
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
        if y + insets.top >= realRange - 0.5 { return Knob(value: 1, proportion: proportion, place: range, range: range) }
        if y + insets.top <= 0.5 { return Knob(value: 0, proportion: proportion, place: 0, range: range) }
        let value = Double(min(1, max(0, (place + insets.top) / range)))
        return Knob(value: value, proportion: proportion, onScreen: anchor.distance < clip.height, place: place + insets.top, range: range)
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

    /// Where the reader is, as a row and how far down it, a share of its
    /// height: what a resize keeps.
    struct Reading: Equatable {
        var id: String
        var share: CGFloat
    }

    func reading(_ scroll: NSScrollView) -> Reading? {
        guard let top = top(scroll), let frame = frames[top.id], frame.height > 0 else { return nil }
        return Reading(id: top.id, share: top.into / frame.height)
    }

    /// Whether `id`'s row is built.
    func has(_ id: String) -> Bool { frames[id] != nil }

    /// The knob's place for `reading`, by our heights: for a row the
    /// stack let go of.
    func value(for reading: Reading, in scroll: NSScrollView) -> Double? {
        guard let doc = scroll.documentView, let i = index[reading.id] else { return nil }
        ensureSums()
        let clip = scroll.contentView.bounds
        let insets = scroll.contentInsets
        let head = self.head ?? headGuess
        let range = content(head) + insets.top + insets.bottom - clip.height
        guard range > 1, doc.frame.height > 0 else { return nil }
        let row = sums.prefix(i + 1) - sums.prefix(i) - spacing
        let place = head + sums.prefix(i) + reading.share * row
        return Double(min(1, max(0, (place + insets.top) / range)))
    }

    /// The clip's top for `reading`, if its row is built.
    func origin(for reading: Reading, in scroll: NSScrollView) -> CGFloat? {
        frames[reading.id].map { $0.minY + reading.share * $0.height }
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

/// The transcript's vertical scroller, driven (ov-701). The scroll view's
/// own `NSScroller` draws, fades and follows "Show scroll bars" as every
/// other does; this sets its knob from `TranscriptLayout`'s heights rather
/// than the content's size, and takes its actions.
///
/// - **The knob.** The scroll view sets the scroller's value and size from
///   its content on every scroll (about once a frame while scrolling), after
///   any notification of the scroll. So the knob is set again just before
///   each frame is committed: a run-loop observer ordered after AppKit's
///   display cycle and before Core Animation's commit.
/// - **The actions.** The scroller's target is this rather than the scroll
///   view: a knob dragged, a click in the track, or VoiceOver's value set
///   or increment arrives here, with AppKit's own tracking, paging repeat
///   and "Jump to the spot that's clicked".
@MainActor
final class TranscriptScroller: NSObject {
    weak var layout: TranscriptLayout?
    private(set) weak var scroll: NSScrollView?
    /// The scroll view's scroller this drives.
    private(set) weak var scroller: NSScroller?

    /// The scroll views driven, for a test to find this from one.
    private static let driving = NSMapTable<NSScrollView, TranscriptScroller>.weakToWeakObjects()

    static func driving(_ scroll: NSScrollView) -> TranscriptScroller? { driving.object(forKey: scroll) }

    /// Where the pointer, or VoiceOver, put the knob. The clip is scrolled
    /// there through our heights, but rows the stack hadn't built are laid
    /// out at its own guess, a frame later: so until the clip's place in
    /// our heights agrees, the knob stays here and the clip is put right
    /// again each frame (`settle`), for at most `settleFrames` displayed
    /// frames after the pointer lets go. A gesture of the reader's own
    /// drops it at once.
    private(set) var aimed: Double?
    private var settleLeft = 0
    static let settleFrames = 24
    /// While the width changes, the stack builds rows again at the new
    /// width and lets others go, a frame or two apart, and a row on screen
    /// can report a place from the old width: so the knob holds still until
    /// the width has held for `widthFrames` displayed frames, then goes
    /// where the reader is, as the system's does when a resize ends.
    static let widthFrames = 15
    private var width: CGFloat = 0
    private var widthLeft = 0
    /// Where the reader was when the width began to change, unless at an
    /// end: put back only if the stack let go of their row meanwhile.
    private var resizedFrom: TranscriptLayout.Reading?
    /// Whether a ticker runs: an aim or a width settling.
    var ticking: Bool { ticker != nil }
    /// Whether the reader would be put back when the width holds.
    var restorePending: Bool { resizedFrom != nil }
    /// A move of the knob larger than this with nothing aimed is shown a
    /// frame later, once it's still there: the scroll view can set the clip
    /// for a resized stack before the rows report where they went. Not a
    /// move the clip's own scroll accounts for, with the content's size
    /// unchanged: a frame 120 ms late scrolls 1,500 pt at once, and a knob
    /// held a frame then moved two frames' worth on the next (ov-715). Nor,
    /// the content resized, one a row on screen placed, the same way as the
    /// clip and no farther than the clip did; and a hold is let go when the
    /// knob is asked again before the frame is committed, the rows reported
    /// (ov-734).
    static let jumpHeld = 0.01
    private var heldFrom: Double?
    /// Where the clip was, the content's height, and the knob's place in
    /// our heights, when the knob last showed where the reader is.
    private var shownAt: (y: CGFloat, document: CGFloat, place: CGFloat)?
    /// The value and size the knob was last given, which the scroll view
    /// overwrites; how many times it did, for a test.
    private var shown: (value: Double, proportion: CGFloat)?
    private(set) var overwritten = 0

    /// The scroller's own target and action, given back on detaching.
    private weak var theirTarget: AnyObject?
    private var theirAction: Selector?
    private var observer: CFRunLoopObserver?
    private var ticker: CADisplayLink?
    private var watching: [NSObjectProtocol] = []
    private var monitor: Any?

    init(layout: TranscriptLayout) {
        self.layout = layout
        super.init()
    }

    // MARK: Attaching

    /// Drive `scroll`'s vertical scroller; again if the scroll view made a
    /// new one.
    func attach(_ scroll: NSScrollView) {
        guard let scroller = scroll.verticalScroller else { return }
        if self.scroll === scroll, self.scroller === scroller, scroller.target === self { return }
        if self.scroll !== scroll { detach() }
        self.scroll = scroll
        self.scroller = scroller
        Self.driving.setObject(self, forKey: scroll)
        if scroller.target !== self {
            theirTarget = scroller.target
            theirAction = scroller.action
        }
        scroller.target = self
        scroller.action = #selector(scrolled(_:))
        if observer == nil {
            let observer = CFRunLoopObserverCreateWithHandler(
                nil, CFRunLoopActivity.beforeWaiting.rawValue | CFRunLoopActivity.exit.rawValue, true, 1_999_999
            ) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.pass() }
            }
            CFRunLoopAddObserver(CFRunLoopGetMain(), observer, .commonModes)
            self.observer = observer
        }
        if watching.isEmpty {
            let center = NotificationCenter.default
            // A live scroll AppKit begins is the reader's own gesture.
            watching.append(center.addObserver(forName: NSScrollView.willStartLiveScrollNotification, object: scroll, queue: nil) { [weak self] _ in
                MainActor.assumeIsolated { self?.readerScrolled(appKitLive: true) }
            })
            // The content resized (a row grew, the stack sized its content
            // again): the clip view sets the scroller from it on the same
            // notification, which may come after the pass above in a frame,
            // inside Core Animation's commit. Observed after the clip view's
            // own, so ours is set last.
            if let doc = scroll.documentView {
                watching.append(center.addObserver(forName: NSView.frameDidChangeNotification, object: doc, queue: nil) { [weak self] _ in
                    MainActor.assumeIsolated { self?.apply() }
                })
            }
        }
        apply()
    }

    /// Let go of the scroll view: the ticker stopped, a live scroll of ours
    /// ended, the scroller handed back to the scroll view.
    func detach() {
        ticker?.invalidate()
        ticker = nil
        if let observer { CFRunLoopRemoveObserver(CFRunLoopGetMain(), observer, .commonModes) }
        observer = nil
        watching.forEach(NotificationCenter.default.removeObserver)
        watching = []
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        aimed = nil
        dragging = false
        endLive()
        if let scroller, scroller.target === self {
            scroller.target = theirTarget
            scroller.action = theirAction
        }
        scroll = nil
        scroller = nil
    }

    deinit {
        MainActor.assumeIsolated { detach() }
    }

    // MARK: The knob

    /// Our heights changed, or the scroller was given its value: set the
    /// knob on the next pass.
    var needsApply = true
    /// The geometry the knob was last set for.
    private var appliedFor: TranscriptLayout.Geometry?

    /// The run-loop pass before a frame is committed: nothing unless the
    /// clip, the document or our heights changed since the last, or the
    /// scroll view set the scroller's knob again.
    private func pass() {
        guard let scroll, let scroller else { return }
        let now = TranscriptLayout.Geometry(scroll)
        let reset = shown.map { scroller.doubleValue != $0.value || scroller.knobProportion != $0.proportion } ?? true
        guard needsApply || reset || now != appliedFor else { return }
        apply()
    }

    /// Set the knob from our heights, before the frame is committed.
    func apply(confirmed: Bool = false) {
        guard let scroll, let scroller, let layout else { return }
        if scroll.verticalScroller !== scroller || scroller.target !== self { return attach(scroll) }
        let clip = scroll.contentView.bounds.width
        if abs(clip - width) > 0.5 {
            if width > 0 {
                if widthLeft == 0, aimed == nil {
                    let knob = layout.knob(scroll)
                    let atEnd = knob.map { $0.value <= 0.0005 || $0.value >= 0.9995 } ?? true
                    resizedFrom = atEnd ? nil : layout.reading(scroll)
                }
                widthLeft = Self.widthFrames
                tick()
            }
            width = clip
        }
        guard let knob = layout.knob(scroll) else {
            shown = nil
            return
        }
        var value = aimed ?? knob.value
        var proportion = knob.proportion
        if let shown, aimed == nil {
            if widthLeft > 0 {
                (value, proportion) = shown
            } else if !confirmed, abs(knob.value - shown.value) > Self.jumpHeld, !scrolled(to: knob, scroll) {
                // Shown on the next displayed frame, if it's still there.
                if heldFrom == nil {
                    heldFrom = shown.value
                    tick()
                }
                value = shown.value
            } else {
                // Asked again before the frame is committed, with the rows
                // reported: held no longer (ov-734).
                heldFrom = nil
            }
        }
        needsApply = false
        let applied = TranscriptLayout.Geometry(scroll)
        appliedFor = applied
        if let shown, scroller.doubleValue != shown.value || scroller.knobProportion != shown.proportion { overwritten += 1 }
        if scroller.doubleValue != value { scroller.doubleValue = value }
        if scroller.knobProportion != proportion { scroller.knobProportion = proportion }
        shown = (value, proportion)
        // Held, what the knob's move is measured from stays where it was
        // last shown.
        if heldFrom == nil { shownAt = aimed == nil && value == knob.value ? (applied.y, applied.document, knob.place) : nil }
    }

    /// Whether the knob's move since it last showed the reader's place is
    /// the clip's scroll: the content the same height, and the knob's place
    /// in our heights moved as far as the clip did, to within `jumpHeld`
    /// of its track.
    ///
    /// The content resized, the clip's move is no exact measure: the stack
    /// moved it by what it took off above as well as by the scroll. The
    /// knob is the reader's place then if a row on screen placed it, and it
    /// moved the same way as the clip and no farther. One placed from a row
    /// the clip has left was read before the rows reported where the stack
    /// put them; one that moved with the clip still, a row laid out again
    /// at a new width (1.4 to 2.3% as a window's edge began to move), or
    /// against it, is no scroll. Those are what the hold is for. Held for any resize, a late frame's
    /// scroll on the frame the stack sized its content again (1,500 pt, the
    /// clip 10,755 pt) was shown a frame late, with the next frame's: a
    /// jump of 3.4% of the track beyond the scroll (ov-734).
    private func scrolled(to knob: TranscriptLayout.Knob, _ scroll: NSScrollView) -> Bool {
        guard let shownAt else { return false }
        let now = TranscriptLayout.Geometry(scroll)
        guard abs(now.document - shownAt.document) <= 0.5 else {
            let moved = knob.place - shownAt.place
            let clip = now.y - shownAt.y
            let tolerance = CGFloat(Self.jumpHeld) * max(1, knob.range)
            // Against the clip, a row laid out again, not the reader's scroll.
            let along = moved * clip >= 0 || abs(moved) <= tolerance
            return knob.onScreen && along && abs(moved) - abs(clip) <= tolerance
        }
        let unexplained = abs((knob.place - shownAt.place) - (now.y - shownAt.y))
        return Double(unexplained / max(1, knob.range)) <= Self.jumpHeld
    }

    // MARK: Ticks

    /// A tick a displayed frame while an aim settles or the width does.
    private func tick() {
        guard ticker == nil, let scroll else { return }
        let link = scroll.displayLink(target: self, selector: #selector(ticked))
        link.add(to: .main, forMode: .common)
        ticker = link
    }

    @objc private func ticked() {
        if heldFrom != nil {
            heldFrom = nil
            apply(confirmed: true)
        }
        if aimed != nil { settle() }
        if widthLeft > 0 {
            widthLeft -= 1
            if widthLeft == 0 { widthHeld() }
        }
        if aimed == nil, widthLeft == 0, heldFrom == nil {
            ticker?.invalidate()
            ticker = nil
        }
    }

    /// The width has held: the knob goes where the reader is. If the stack
    /// let go of the reader's row meanwhile (it can, with the main thread
    /// busy), they are put back on it, as far down it as they were. Not in
    /// a gesture of the reader's own, and not at an end, where the stack's
    /// own anchor keeps them.
    private func widthHeld() {
        let from = resizedFrom
        resizedFrom = nil
        guard aimed == nil, let from, let scroll, let layout, !LiveScroll.scrolls(scroll), !layout.has(from.id),
              let value = layout.value(for: from, in: scroll)
        else { return apply(confirmed: true) }
        aim(value, scroll)
    }

    // MARK: Aiming

    /// The pointer is held down on the scroller.
    private var dragging = false

    /// Hold the knob at `value` and scroll there, in a live scroll, putting
    /// the clip right each frame until it agrees.
    func aim(_ value: Double, _ scroll: NSScrollView) {
        resizedFrom = nil
        beginLive(scroll)
        aimed = value
        settleLeft = Self.settleFrames
        scrollTo(value, scroll)
        apply()
        tick()
        watchForTheReader()
    }

    /// A frame's check of the aim: the clip put right while the knob and
    /// the clip disagree; let go once they agree on rows on screen, when
    /// there's no knob to agree with (a transcript that fits), or
    /// `settleFrames` after the pointer let go.
    private func settle() {
        guard let aimed, let scroll else { return release() }
        dragging = Self.held() && dragging
        if dragging { settleLeft = Self.settleFrames }
        guard let knob = layout?.knob(scroll) else { return release() }
        if abs(knob.value - aimed) > 0.0005 {
            scrollTo(aimed, scroll)
        } else if !dragging, knob.onScreen {
            return release()
        }
        if !dragging {
            settleLeft -= 1
            if settleLeft <= 0 { release() }
        }
    }

    /// The knob goes back to where the clip is, and the live scroll ends.
    private func release() {
        aimed = nil
        dragging = false
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        apply(confirmed: true)
        endLive()
    }

    /// The reader scrolls, or presses, while an aim settles: the aim is
    /// dropped at once. Where AppKit begins a live scroll of its own (a
    /// trackpad's gesture), ours ends without a word, its end being
    /// AppKit's to say; where it begins none (a mouse wheel's tick, a press
    /// elsewhere), ours ends as any other does.
    private func readerScrolled(appKitLive: Bool) {
        guard !posting else { return }
        resizedFrom = nil
        guard aimed != nil else { return }
        aimed = nil
        dragging = false
        if appKitLive { live = false }
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        apply(confirmed: true)
        endLive()
    }

    /// A scroll wheel's or a press's event while an aim settles. A press on
    /// the scroller itself is AppKit's tracking, which aims again.
    private func watchForTheReader() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.scrollWheel, .leftMouseDown]) { [weak self] event in
            MainActor.assumeIsolated { self?.reader(event, in: event.window) }
            return event
        }
    }

    /// An event of the reader's while an aim settles: a wheel's, or a press
    /// anywhere in the window but on the scroller (AppKit's tracking, which
    /// aims again). `window` is the window AppKit is sending it to.
    func reader(_ event: NSEvent, in window: NSWindow?) {
        guard let scroll, window === scroll.window else { return }
        if event.type == .leftMouseDown, let scroller, scroller.testPart(event.locationInWindow) != .noPart { return }
        // A trackpad's gesture, or its coast: AppKit's live scroll.
        let phased = event.type == .scrollWheel && (!event.phase.isEmpty || !event.momentumPhase.isEmpty)
        readerScrolled(appKitLive: phased)
    }

    /// The scroll view in a live scroll of ours. The stack keeps the rows on
    /// screen through its resizes only in one: a clip moved outside one is
    /// left where it was, and the rows slide under it. None of ours inside
    /// AppKit's own (a coast still going), whose end is AppKit's to say.
    private var live = false
    private var posting = false

    private func beginLive(_ scroll: NSScrollView) {
        guard !live, !LiveScroll.scrolls(scroll) else { return }
        live = true
        posting = true
        NotificationCenter.default.post(name: NSScrollView.willStartLiveScrollNotification, object: scroll)
        posting = false
    }

    private func endLive() {
        guard live, !dragging, aimed == nil else { return }
        live = false
        if let scroll { NotificationCenter.default.post(name: NSScrollView.didEndLiveScrollNotification, object: scroll) }
    }

    // MARK: Actions

    /// The scroller's action: AppKit's tracking of the knob, a click in the
    /// track (repeated while held), or VoiceOver's value set (no part).
    @objc func scrolled(_ sender: NSScroller) {
        guard let scroll else { return }
        // The part the pointer is on, only while AppKit tracks it: the
        // scroller keeps the last one, so VoiceOver's value set after a
        // click in the track would otherwise page.
        let part = Self.tracking() ? Self.part(sender) : .noPart
        switch part {
        case .decrementPage, .decrementLine: page(-1, scroll)
        case .incrementPage, .incrementLine: page(1, scroll)
        case .knob, .knobSlot:
            dragging = Self.held()
            aim(sender.doubleValue, scroll)
            NotificationCenter.default.post(name: NSScrollView.didLiveScrollNotification, object: scroll)
        default:
            aim(min(1, max(0, sender.doubleValue)), scroll)
        }
    }

    /// Whether AppKit is tracking the pointer on the scroller: the event in
    /// hand is a press, a drag, or a paging repeat. A test sets these.
    static var tracking: @MainActor () -> Bool = {
        guard let event = NSApp.currentEvent else { return false }
        return [.leftMouseDown, .leftMouseDragged, .periodic].contains(event.type)
    }
    static var part: @MainActor (NSScroller) -> NSScroller.Part = { $0.hitPart }
    static var held: @MainActor () -> Bool = { NSEvent.pressedMouseButtons & 1 != 0 }

    /// A page up (-1) or down (1), in a live scroll of its own: the
    /// window's height less the context the scroll view keeps.
    func page(_ direction: CGFloat, _ scroll: NSScrollView) {
        resizedFrom = nil
        aimed = nil
        beginLive(scroll)
        let clip = scroll.contentView.bounds
        let insets = scroll.contentInsets
        let page = max(1, clip.height - scroll.verticalPageScroll)
        let real = (scroll.documentView?.frame.height ?? 0) + insets.top + insets.bottom - clip.height
        let y = min(max(clip.minY + direction * page, -insets.top), real - insets.top)
        scroll.contentView.scroll(to: NSPoint(x: clip.minX, y: y))
        scroll.reflectScrolledClipView(scroll.contentView)
        NotificationCenter.default.post(name: NSScrollView.didLiveScrollNotification, object: scroll)
        apply(confirmed: true)
        endLive()
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
}

/// Drives the vertical scroller of the scroll view this sits in.
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
            if window == nil { layout?.scroller?.detach() } else { install() }
        }

        func install() {
            guard let layout, window != nil, let scroll = enclosingScrollView else { return }
            let scroller = layout.scroller ?? TranscriptScroller(layout: layout)
            layout.scroller = scroller
            scroller.attach(scroll)
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
