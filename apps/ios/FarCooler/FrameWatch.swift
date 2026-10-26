#if DEBUG
import SwiftUI
import UIKit

/// `-frame-watch` (ov-788): a scripted fling, timed frame by frame, for
/// measuring a screen's scroll before and after a change.
///
/// A test can't fling well enough to measure: XCUITest snapshots the app's
/// accessibility tree before each gesture, on the main thread, and waits for
/// the app to go idle between gestures, so a measured hitch would be the
/// test's own. So the test only posts `com.farcooler.harness.fling`, and the
/// app scrolls itself: the screen's main scroll view, moved by a display link
/// at a fling's speed, decaying as UIKit's normal deceleration does, eight
/// flings away from where it starts and eight back. Moved by time, not by frame, so a frame
/// the main thread holds up moves the list as far as a finger's would have.
///
/// Each frame's interval is measured on the same link. A frame later than
/// its slot is a hitch, by how much it was late; the report is how many,
/// the worst, those over one frame, and the hitch time ratio (late ms per
/// second of fling), as Instruments' Animation Hitches gives them. It's set
/// as `frame-watch`'s value, and logged as a `FRAME-WATCH` line.
@MainActor
final class FrameWatch: ObservableObject {
    static var isRequested: Bool { CommandLine.arguments.contains("-frame-watch") }
    static let shared = FrameWatch()

    @Published private(set) var report = ""

    private var link: CADisplayLink?
    private weak var scroll: UIScrollView?
    private var last: CFTimeInterval = 0
    private var started: CFTimeInterval = 0
    private var flingStart: CFTimeInterval = 0
    private var fling = 0
    private var hitches: [Double] = []
    private var frames = 0
    /// `SelectableTextView.made` when the fling began.
    private var madeBefore = 0
    private var slot: Double = 1.0 / 60
    /// Away from where it starts first: up from a transcript's tail, down
    /// from a list's top.
    private var firstWay: CGFloat = -1

    /// Each fling's speed as it leaves the finger, in points a second.
    private static let speed: CGFloat = 5_000
    /// UIKit's `.normal` deceleration rate, per millisecond.
    private static let rate: CGFloat = 0.998
    private static let flings = 16
    private static let flingTime: CFTimeInterval = 1.2

    private init() {
        let center = CFNotificationCenterGetDarwinNotifyCenter()
        CFNotificationCenterAddObserver(
            center, nil,
            { _, _, _, _, _ in Task { @MainActor in FrameWatch.shared.start() } },
            "com.farcooler.harness.fling" as CFString, nil, .deliverImmediately)
    }

    private func start() {
        guard link == nil, let scroll = Self.mainScroll() else {
            report = "error=no-scroll-view"
            return
        }
        self.scroll = scroll
        firstWay = scroll.contentOffset.y <= -scroll.adjustedContentInset.top + 10 ? 1 : -1
        report = ""
        madeBefore = SelectableTextView.made
        hitches = []
        frames = 0
        fling = 0
        last = 0
        let link = CADisplayLink(target: Ticker(self), selector: #selector(Ticker.tick(_:)))
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    fileprivate func tick(_ link: CADisplayLink) {
        let now = link.timestamp
        slot = max(link.targetTimestamp - link.timestamp, 1.0 / 240)
        if last == 0 {
            started = now
            flingStart = now
        } else {
            frames += 1
            let interval = now - last
            // Late by more than half a slot: a frame was missed.
            if interval > slot * 1.5 { hitches.append((interval - slot) * 1000) }
        }
        let dt = last == 0 ? 0 : now - last
        last = now
        guard let scroll else { return finish() }
        let t = now - flingStart
        if t >= Self.flingTime {
            fling += 1
            flingStart = now
            if fling >= Self.flings { return finish() }
        }
        // Away from the start for the first half, then back.
        let direction: CGFloat = fling < Self.flings / 2 ? firstWay : -firstWay
        let velocity = Self.speed * pow(Self.rate, CGFloat(t * 1000))
        let top = -scroll.adjustedContentInset.top
        let bottom = max(top, scroll.contentSize.height - scroll.bounds.height + scroll.adjustedContentInset.bottom)
        var y = scroll.contentOffset.y + direction * velocity * CGFloat(dt)
        y = min(max(y, top), bottom)
        scroll.contentOffset.y = y
    }

    private func finish() {
        link?.invalidate()
        link = nil
        let seconds = max(last - started, 0.001)
        let late = hitches.reduce(0, +)
        let overOne = hitches.filter { $0 > slot * 1000 }.count
        let worst = hitches.max() ?? 0
        report = String(
            format:
                "frames=%d seconds=%.2f hitches=%d over-one-frame=%d worst-ms=%.1f ratio-ms-per-s=%.1f slot-ms=%.1f text-views-made=%d",
            frames, seconds, hitches.count, overOne, worst, late / seconds, slot * 1000, SelectableTextView.made - madeBefore)
        NSLog("FRAME-WATCH %@ all=%@", report, hitches.sorted(by: >).prefix(20).map { String(format: "%.0f", $0) }.joined(separator: ","))
    }

    /// The largest scroll view on screen that can scroll a long way
    /// vertically: the conversation's transcript, the task's list. Not a
    /// text view, and not a code block's box.
    private static func mainScroll() -> UIScrollView? {
        var best: UIScrollView?
        func walk(_ view: UIView) {
            if let scroll = view as? UIScrollView, !(scroll is UITextView), !scroll.isHidden, scroll.window != nil,
                scroll.contentSize.height > scroll.bounds.height + 200
            {
                let area = scroll.bounds.width * scroll.bounds.height
                if area > (best.map { $0.bounds.width * $0.bounds.height } ?? 0) { best = scroll }
            }
            view.subviews.forEach(walk)
        }
        for scene in UIApplication.shared.connectedScenes {
            for window in (scene as? UIWindowScene)?.windows ?? [] where !window.isHidden { walk(window) }
        }
        return best
    }
}

/// The display link's target: it holds its target strongly.
private final class Ticker: NSObject {
    weak var watch: FrameWatch?
    init(_ watch: FrameWatch) { self.watch = watch }
    @MainActor @objc func tick(_ link: CADisplayLink) { watch?.tick(link) }
}

/// `FrameWatch`'s report, where a test reads it.
struct FrameWatchReport: View {
    @ObservedObject private var watch = FrameWatch.shared

    var body: some View {
        Color.clear
            .frame(width: 1, height: 1)  // style-exempt: DEBUG probe a UI test reads, not a separator
            .allowsHitTesting(false)
            .accessibilityElement()
            .accessibilityIdentifier("frame-watch")
            .accessibilityValue(watch.report)
    }
}
#endif
