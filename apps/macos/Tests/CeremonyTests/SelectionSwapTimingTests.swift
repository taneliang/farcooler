import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// How fast a selection change reaches the screen (ov-293): the real
/// `WorkspaceView`, with the app's own motion, in a window, read back pixel
/// by pixel as it changes.
///
/// The owner found navigation laggy: a cross-fade that seemed "both delayed
/// and a little slow". Measured here before the fix, on
/// `WorkspaceMotion.spring`: opening a task had moved 4% at 22 ms, was 95%
/// drawn at about 200 ms and settled at 260 ms; closing it the same; and a
/// click on a second task left its terminal blank for 150 ms
/// (`WorkspaceMotion.settle`). Each swap is now fully drawn at the first
/// reading, 4–6 ms in, and a click mounts the terminal at once; only a
/// quick walk through the list still waits for its last step.
/// `OV293_FRAMES=1` prints every reading, to look at the curve itself.
@MainActor
@Suite(.serialized)
struct SelectionSwapTimingTests {
    @MainActor
    final class Level: ObservableObject {
        @Published var opened: String?
    }

    /// The colors each part is drawn in, far enough apart that a fraction of
    /// the way from one to the other reads cleanly at either scale.
    static let orchestrator = NSColor(srgbRed: 0, green: 0, blue: 1, alpha: 1)
    static let waiting = NSColor(srgbRed: 0.5, green: 0.5, blue: 0.5, alpha: 1)
    static func color(_ item: String) -> NSColor {
        item == "a" ? NSColor(srgbRed: 1, green: 0, blue: 0, alpha: 1) : NSColor(srgbRed: 0, green: 1, blue: 0, alpha: 1)
    }

    struct Hosted: View {
        @ObservedObject var level: Level
        var body: some View {
            // The app's own motion: nothing passed but what's drawn.
            WorkspaceView(
                opened: level.opened, hasConversation: true, cell: WorkspaceColumns.defaultCell,
                focused: false, navigatorWidth: .constant(200),
                conversation: { Color(nsColor: SelectionSwapTimingTests.orchestrator) },
                navigator: { Color.white },
                breadcrumb: { _ in Color.clear.frame(height: 0) },
                detail: { item, settled in
                    VStack(spacing: 0) {
                        // Its header and record: drawn whether or not it's
                        // settled.
                        Color(nsColor: SelectionSwapTimingTests.color(item))
                        // Its terminal: only once settled.
                        Color(nsColor: settled ? SelectionSwapTimingTests.color(item) : SelectionSwapTimingTests.waiting)
                    }
                })
            .frame(width: 600, height: 300)
        }
    }

    /// One reading: when, in ms after the change, and how far each half of
    /// the main area has gone from what it showed to what it will show.
    struct Sample {
        let ms: Double
        let top: Double
        let bottom: Double
    }

    /// Where in its change each reading is: 0 at `from`, 1 at `to`.
    static func fraction(_ c: NSColor, from: NSColor, to: NSColor) -> Double {
        let d = [to.redComponent - from.redComponent, to.greenComponent - from.greenComponent, to.blueComponent - from.blueComponent]
        let v = [c.redComponent - from.redComponent, c.greenComponent - from.greenComponent, c.blueComponent - from.blueComponent]
        let dd = d.reduce(0) { $0 + $1 * $1 }
        return Double(zip(d, v).reduce(0) { $0 + $1.0 * $1.1 } / dd)
    }

    @MainActor
    final class Harness {
        let level = Level()
        let window: NSWindow
        let host: NSHostingView<Hosted>

        init() {
            host = NSHostingView(rootView: Hosted(level: level))
            // Off every screen, but ordered in, so the motion runs as it does
            // in a window someone's looking at.
            window = NSWindow(
                contentRect: NSRect(x: -6000, y: -6000, width: 600, height: 300), styleMask: [.borderless],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = host
            window.orderFrontRegardless()
        }

        func rest() async {
            for _ in 0..<20 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(25))
            }
        }

        /// The main area's color at its top and bottom quarter, as drawn now.
        func colors() -> (NSColor, NSColor) {
            let rep = host.lookBitmap(scale: 1)!
            // The main area is 201–600 pt; the bitmap's y runs down.
            return (rep.color(atPoint: 400, 75), rep.color(atPoint: 400, 225))
        }

        /// Sets `opened` to `next` and reads the screen back every few ms for
        /// `span`.
        func change(
            to next: String?, from before: (NSColor, NSColor), to after: (NSColor, NSColor), span: Double = 600
        ) async -> [Sample] {
            var samples: [Sample] = []
            let clock = ContinuousClock()
            let start = clock.now
            level.opened = next
            while true {
                try? await Task.sleep(for: .milliseconds(4))
                let elapsed = start.duration(to: clock.now)
                let ms = Double(elapsed.components.seconds) * 1000 + Double(elapsed.components.attoseconds) / 1e15
                let (top, bottom) = colors()
                samples.append(
                    Sample(
                        ms: ms, top: SelectionSwapTimingTests.fraction(top, from: before.0, to: after.0),
                        bottom: SelectionSwapTimingTests.fraction(bottom, from: before.1, to: after.1)))
                if ms > span { break }
            }
            return samples
        }

        func close() { window.close() }
    }

    /// The first reading at or past `level` of the way, for `part`.
    static func reached(_ samples: [Sample], _ level: Double, _ part: KeyPath<Sample, Double>) -> Double? {
        samples.first { $0[keyPath: part] >= level }?.ms
    }

    static func describe(_ name: String, _ samples: [Sample]) -> String {
        func ms(_ v: Double?) -> String { v.map { String(format: "%.0f ms", $0) } ?? "never" }
        return "\(name): first change \(ms(reached(samples, 0.02, \.top))), drawn \(ms(reached(samples, 0.95, \.top))), "
            + "terminal \(ms(reached(samples, 0.95, \.bottom))); first reading at \(ms(samples.first?.ms))"
    }

    /// The cap (ov-293): a selection change starts on the next frame and is
    /// drawn within 150 ms, its terminal included on a click. Judged only on
    /// readings past the cap and its slack, each of which must show it
    /// drawn: a loaded runner whose first reading comes late (472 ms once,
    /// under the full suite) then has fewer to judge, never a false red.
    static let cap = 150.0
    static let slack = 60.0

    @Test("A selection change starts at once and is drawn within the cap")
    func swapsAreInstant() async {
        let harness = Harness()
        await harness.rest()
        let blue = (Self.orchestrator, Self.orchestrator)
        let red = (Self.color("a"), Self.color("a"))
        let green = (Self.color("b"), Self.color("b"))
        let open = await harness.change(to: "a", from: blue, to: red)
        await harness.rest()
        let swap = await harness.change(to: "b", from: red, to: green)
        await harness.rest()
        let close = await harness.change(to: nil, from: green, to: blue)
        await harness.rest()
        harness.close()
        let report = [("open", open), ("switch", swap), ("close", close)].map { Self.describe($0.0, $0.1) }
        print("ov-293 timings\n" + report.joined(separator: "\n"))
        if ProcessInfo.processInfo.environment["OV293_FRAMES"] != nil {
            for (name, samples) in [("open", open), ("switch", swap), ("close", close)] {
                print(name + " " + samples.prefix(40).map { String(format: "%.0f:%.2f/%.2f", $0.ms, $0.top, $0.bottom) }.joined(separator: " "))
            }
        }
        for (name, samples) in [("open", open), ("switch", swap), ("close", close)] {
            // No delay: moved by the first reading two frames in, past the
            // pass that takes the change in.
            let early = samples.first { $0.ms >= 34 }!
            #expect(early.top > 0.02, "\(name) hadn't started \(Int(early.ms)) ms in")
            // No longer than the cap: drawn by then.
            let undrawn = samples.first { $0.ms > Self.cap + Self.slack && $0.top < 0.95 }
            #expect(undrawn == nil, "\(name) wasn't drawn \(Int(undrawn?.ms ?? 0)) ms in")
            let blank = samples.first { $0.ms > Self.cap + Self.slack && $0.bottom < 0.95 }
            #expect(blank == nil, "\(name)'s terminal wasn't drawn \(Int(blank?.ms ?? 0)) ms in")
            // A click mounts the terminal with the rest, not a settle later.
            let drawn = Self.reached(samples, 0.95, \.top) ?? .infinity
            let terminal = Self.reached(samples, 0.95, \.bottom) ?? .infinity
            #expect(terminal <= drawn + 40, "\(name)'s terminal waited \(Int(terminal - drawn)) ms")
        }
    }
}
