import AgentKit
import AppKit
import QuartzCore
import SwiftUI

@testable import Far_Cooler

/// A session as long and as uneven as a real one (ov-691), the Mac's copy of
/// the iPhone's `RealSession` (ov-675): four hundred rows of prompts (some
/// with photos, which come late), runs of tool calls, long answers with lists
/// and code, folded subagent reports and short lines between. Rows differ in
/// height by ten times, so a lazy stack's guesses for the rows it hasn't
/// built are wrong, as they are in the app.
enum RealSession {
    static let count = 400

    static func rows(_ count: Int = count) -> [[String: Any]] {
        (1...count).map(row)
    }

    static func row(_ n: Int) -> [String: Any] {
        let kind: [String: Any]
        switch n % 10 {
        case 0:
            var turn: [String: Any] = [
                "prompt": "Turn \(n): look at the scroll view again. "
                    + String(repeating: "When I scroll up it jumps, and sometimes the whole list stutters until I stop. ", count: 1 + n % 3),
                "origin": "Typed", "background_running": 0, "started_ms": 1, "ended_ms": 40_000, "duration_ms": 40_000, "outcome": "Finished",
            ]
            if n % 20 == 0 { turn["images"] = [["mime": "image/png"], ["mime": "image/png"]] }
            // The newest turn is still working, as the reader's is while a
            // reply streams in below it.
            if n == count {
                turn["outcome"] = NSNull()
                turn["ended_ms"] = NSNull()
                turn["duration_ms"] = NSNull()
                turn["activity"] = "Busy"
            }
            kind = ["Turn": turn]
        case 1, 2, 3, 4:
            kind = ["Tool": [
                "name": n % 2 == 0 ? "Read" : "Bash", "summary": "Step \(n) of the look", "status": "Done", "started_ms": 1, "ended_ms": 400,
                "diff": [Any](), "input": "command: rg -n scrollTo apps/macos\ndescription: Step \(n)", "result": "NativeAgentView.swift:115",
            ] as [String: Any]]
        case 5:
            let list = (1...(3 + n % 4)).map { "- Point \($0): the offset moves when a row above is measured, by its height less the guess." }
            let text = "Answer \(n). Here is **what I found**, in `NativeAgentView`.\n\n" + list.joined(separator: "\n")
                + "\n\n```swift\nScrollView { LazyVStack { ForEach(store.items) { row in NativeRowView(row) } } }\nlet y = clip.bounds.origin.y\n```\n\n"
                + "| Row | Height |\n| --- | --- |\n| Report | 344 |\n| Message | 48 |\n\nThat is all for this step."
            kind = ["Prose": ["text": text, "conclusion": false, "at_ms": 1]]
        case 6:
            let body = (0..<8).map { "Line \($0) of the report: the lane ran its tests, read the files and wrote what it saw." }.joined(separator: "\n\n")
            kind = ["Turn": [
                "prompt": body, "origin": "Notification", "background_running": 0, "started_ms": 1, "ended_ms": 2, "outcome": "Finished",
                "harness": ["kind": "Report", "title": "", "body": body, "source": "Review lane \(n)"],
            ] as [String: Any]]
        case 8:
            let text = (0..<(2 + n % 5)).map {
                "Paragraph \($0) of answer \(n): the list is lazy, so rows above the screen are guessed until they are built, and then measured."
            }
            kind = ["Prose": ["text": text.joined(separator: "\n\n"), "conclusion": true, "at_ms": 1]]
        default:
            kind = ["Prose": ["text": "Line \(n).", "conclusion": false, "at_ms": 1]]
        }
        return ["id": "real:\(n)", "ord": n, "rev": 1, "turn": NSNull(), "provisional": false, "kind": kind]
    }

    /// The streaming answer at the end, `words` long, in paragraphs of fifty.
    static func streaming(words: Int, rev: Int) -> [String: Any] {
        var text = "Streaming answer."
        for word in 0..<words { text += word % 50 == 0 ? "\n\nParagraph \(word / 50): word\(word)" : " word\(word)" }
        return ["id": "real:stream", "ord": count + 1, "rev": rev, "turn": NSNull(), "provisional": false,
                "kind": ["Prose": ["text": text, "conclusion": false, "at_ms": 1]]]
    }

    /// A runner answering each prompt image with a photo-sized PNG, late, as
    /// over a network.
    actor Photos: PromptImageSource {
        let png: Data
        init() { png = Photos.photo() }
        func promptImage(terminal: String, row: String, index: Int) async throws -> Data {
            try await Task.sleep(for: .milliseconds(400))
            return png
        }

        /// 2000 by 1500 of noise, so it doesn't compress to nothing.
        static func photo() -> Data {
            let rep = NSBitmapImageRep(
                bitmapDataPlanes: nil, pixelsWide: 2000, pixelsHigh: 1500, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
                isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
            var seed: UInt32 = 7
            let bytes = rep.bitmapData!
            for i in 0..<(rep.bytesPerRow * 1500) {
                seed = seed &* 1_664_525 &+ 1_013_904_223
                bytes[i] = UInt8(truncatingIfNeeded: seed >> 24)
            }
            return rep.representation(using: .png, properties: [:])!
        }
    }

    /// A runner that takes Stop and Send Now, so the view offers them as
    /// it does for a working claude.
    struct Keys: InterruptSink {
        func interrupt(terminal: String) async throws {}
        func sendNow(terminal: String) async throws {}
    }
}

/// A trackpad fling, frame by frame (ov-691): each display-link tick moves the
/// conversation's scroll view by what a coasting finger would, lays it out,
/// and notes when the tick came, how long the layout took and how far it was
/// asked to move. A tick that came a frame or more late is a hitch.
@MainActor
final class Fling: NSObject {
    let scroll: NSScrollView
    let window: NSWindow
    /// Points a second; negative scrolls up, toward older rows.
    private(set) var velocity: CGFloat
    /// The per-millisecond decay of a coast, as the system's normal rate.
    let decay: CGFloat = 0.998
    /// Called each tick after the move and the layout, with the move asked for.
    var onTick: ((CGFloat) -> Void)?
    /// Called each tick before the move: rows streaming in, and so on.
    var beforeTick: ((Int) -> Void)?

    private(set) var stamps: [CFTimeInterval] = []
    private(set) var work: [Double] = []
    /// Each tick's main-thread time, from the tick to the run loop's next
    /// wait, the commit and the drawing included, in milliseconds.
    private(set) var busy: [Double] = []
    private(set) var period: CFTimeInterval = 1.0 / 120
    private var link: CADisplayLink?
    /// Each stretch the main thread ran without waiting, longer than 4 ms.
    private(set) var awake: [(start: CFTimeInterval, end: CFTimeInterval)] = []
    private var observer: CFRunLoopObserver?
    private var tickedAt: CFTimeInterval?
    private var wokeAt: CFTimeInterval?
    private var last: CFTimeInterval?
    private(set) var done = false

    init(_ scroll: NSScrollView, in window: NSWindow, velocity: CGFloat) {
        self.scroll = scroll
        self.window = window
        self.velocity = velocity
    }

    func start() {
        let link = window.contentView!.displayLink(target: self, selector: #selector(tick(_:)))
        link.add(to: .main, forMode: .common)
        self.link = link
        // After every other observer, Core Animation's commit included.
        let activities = CFRunLoopActivity.beforeWaiting.rawValue | CFRunLoopActivity.afterWaiting.rawValue
        let observer = CFRunLoopObserverCreateWithHandler(nil, activities, true, CFIndex.max) { [weak self] _, activity in
            MainActor.assumeIsolated {
                guard let self else { return }
                let now = CACurrentMediaTime()
                if activity == .afterWaiting {
                    self.wokeAt = now
                    return
                }
                if let woke = self.wokeAt, now - woke > 0.004 { self.awake.append((woke, now)) }
                self.wokeAt = nil
                guard let at = self.tickedAt else { return }
                self.busy.append((now - at) * 1000)
                self.tickedAt = nil
            }
        }
        CFRunLoopAddObserver(CFRunLoopGetMain(), observer, .commonModes)
        self.observer = observer
    }

    func stop() {
        link?.invalidate()
        link = nil
        if let observer { CFRunLoopRemoveObserver(CFRunLoopGetMain(), observer, .commonModes) }
        observer = nil
        done = true
    }

    @objc private func tick(_ link: CADisplayLink) {
        period = link.duration > 0 ? link.duration : period
        let now = link.timestamp
        stamps.append(CACurrentMediaTime())
        tickedAt = stamps.last
        defer { last = now }
        guard let last else { return }
        let dt = now - last
        beforeTick?(stamps.count)
        let delta = velocity * CGFloat(dt)
        velocity *= pow(decay, CGFloat(dt * 1000))
        let clip = scroll.contentView
        var origin = clip.bounds.origin
        let top = -scroll.contentInsets.top
        origin.y = max(top, origin.y + delta)
        let began = CACurrentMediaTime()
        clip.scroll(to: origin)
        scroll.reflectScrolledClipView(clip)
        window.contentView?.layoutSubtreeIfNeeded()
        work.append((CACurrentMediaTime() - began) * 1000)
        onTick?(delta)
        if abs(velocity) < 30 || origin.y <= top { stop() }
    }

    /// Wait for the coast to end.
    func finish(timeout: Duration = .seconds(20)) async {
        let end = ContinuousClock.now + timeout
        while !done, ContinuousClock.now < end { try? await Task.sleep(for: .milliseconds(20)) }
        stop()
    }

    /// The time ticks came late, a period or more past when they were due,
    /// in milliseconds per second of fling: XCTest's hitch time ratio.
    var hitchRatio: Double {
        guard stamps.count > 2 else { return 0 }
        let gaps = zip(stamps.dropFirst(), stamps).map { $0 - $1 }
        let late = gaps.map { gap -> Double in
            let missed = (gap / period).rounded() - 1
            return missed >= 1 ? missed * period * 1000 : 0
        }.reduce(0, +)
        return late / (stamps.last! - stamps.first!)
    }

    /// `hitchRatio`, counting only the ticks that came late while the main
    /// thread ran a frame or longer without a break: this process's own
    /// hitches, not another's on a busy machine.
    var mainThreadHitchRatio: Double {
        guard stamps.count > 2 else { return 0 }
        let long = awake.filter { $0.end - $0.start >= period }
        let late = zip(stamps.dropFirst(), stamps).map { now, was -> Double in
            let missed = ((now - was) / period).rounded() - 1
            guard missed >= 1, long.contains(where: { $0.end > was && $0.start < now }) else { return 0 }
            return missed * period * 1000
        }.reduce(0, +)
        return late / (stamps.last! - stamps.first!)
    }

    /// Ticks that came a period or more late.
    var dropped: Int {
        zip(stamps.dropFirst(), stamps).filter { ($0 - $1) / period >= 1.5 }.count
    }

    var summary: String {
        func p(_ values: [Double], _ q: Double) -> Double {
            let sorted = values.sorted()
            return sorted.isEmpty ? 0 : sorted[min(sorted.count - 1, Int(Double(sorted.count) * q))]
        }
        let seconds = stamps.count > 1 ? stamps.last! - stamps.first! : 0
        let over = busy.filter { $0 > period * 1000 }.count
        return String(
            format: "%.2f s, %d ticks at %.0f Hz, %d late, hitch %.1f ms/s (main thread's %.1f); layout median %.2f p95 %.2f max %.2f ms; "
                + "main thread median %.2f p95 %.2f max %.2f ms, %d over a frame",
            seconds, stamps.count, 1 / period, dropped, hitchRatio, mainThreadHitchRatio, p(work, 0.5), p(work, 0.95), work.max() ?? 0,
            p(busy, 0.5), p(busy, 0.95), busy.max() ?? 0, over)
            + "; stalls " + awake.filter { $0.end - $0.start >= period }.map { String(format: "%.0f", ($0.end - $0.start) * 1000) }.joined(separator: ",")
    }

    /// The scroll view SwiftUI made for a `ScrollView` in `view`: the
    /// deepest vertical one, the conversation's.
    static func scrollView(in view: NSView) -> NSScrollView? {
        var found: NSScrollView?
        func walk(_ v: NSView) {
            if let s = v as? NSScrollView, s.hasVerticalScroller || s.documentView != nil,
               (s.documentView?.frame.height ?? 0) > (found?.documentView?.frame.height ?? 0) {
                found = s
            }
            v.subviews.forEach(walk)
        }
        walk(view)
        return found
    }
}
