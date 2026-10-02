import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A view the hosting view's hit test can name, counting how often it's
/// made and taken down.
final class MotionMarker: NSView {
    var name = ""
}

@MainActor
final class MotionTally {
    var made: [String: Int] = [:]
    var gone: [String: Int] = [:]
    var fetches: [String: Int] = [:]
}

struct MotionMarkerView: NSViewRepresentable {
    let name: String
    let tally: MotionTally
    func makeNSView(context: Context) -> MotionMarker {
        tally.made[name, default: 0] += 1
        let view = MotionMarker()
        view.name = name
        return view
    }
    func updateNSView(_ view: MotionMarker, context: Context) {}
    static func dismantleNSView(_ view: MotionMarker, coordinator: ()) {
        MainActor.assumeIsolated { MotionTally.current?.gone[view.name, default: 0] += 1 }
    }
}

extension MotionTally {
    /// The tally the views of the test running now report to: dismantling
    /// is static, so it can't reach an instance any other way.
    @MainActor static var current: MotionTally?
}

/// Opening a task beside the board, glancing through others and closing it
/// (ov-85): the board is never taken down or rebuilt, each task's view is
/// made once while it's open, and what takes a click follows the window's
/// state at once, never the motion.
@MainActor
@Suite(.serialized)
struct WorkspaceMotionTests {
    @MainActor
    final class Level: ObservableObject {
        @Published var opened: String?
    }

    /// A workspace 1032 pt wide: the rail at 0–28, the board's list at
    /// 29–329 beside a task, and the task from 330. With nothing open, the
    /// board runs to the trailing edge.
    @MainActor
    final class Harness {
        let level = Level()
        let tally = MotionTally()
        let host: NSHostingView<Hosted>
        let window: NSWindow

        struct Hosted: View {
            @ObservedObject var level: Level
            let tally: MotionTally
            let motion: Animation
            var body: some View {
                WorkspaceView(
                    opened: level.opened, hasConversation: true, cell: WorkspaceColumns.defaultCell,
                    focused: false, peek: false, listWidth: .constant(300),
                    conversation: { Color.clear }, rail: { MotionMarkerView(name: "rail", tally: tally) },
                    board: { MotionMarkerView(name: "board", tally: tally) },
                    breadcrumb: { _ in Color.clear.frame(height: 30) },
                    detail: { item, settled in
                        ZStack {
                            MotionMarkerView(name: item, tally: tally)
                            // What waits on settling: a terminal, and a read
                            // of the runner.
                            if settled {
                                MotionMarkerView(name: "work-\(item)", tally: tally)
                                    .task { tally.fetches[item, default: 0] += 1 }
                            }
                        }
                    }, motion: motion)
                .frame(width: 1032, height: 400)
            }
        }

        init(motion: Animation) {
            MotionTally.current = tally
            host = NSHostingView(rootView: Hosted(level: level, tally: tally, motion: motion))
            window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 1032, height: 400), styleMask: [.borderless],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = host
        }

        func settle(_ frames: Int = 5) async {
            for _ in 0..<frames {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }

        /// What a click `x` pt in lands on: a task's name, whether on its
        /// header or its settled work ("work-…"), or a part's.
        func hit(_ x: CGFloat) -> String {
            let name = (host.hitTest(NSPoint(x: x, y: 200)) as? MotionMarker)?.name ?? "none"
            return name.hasPrefix("work-") ? String(name.dropFirst("work-".count)) : name
        }

        func close() {
            window.close()
            MotionTally.current = nil
        }
    }

    /// Slow enough that 40 ms in, nothing has visibly moved.
    private static let slow = Animation.spring(response: 3, dampingFraction: 1)
    private static let quick = Animation.linear(duration: 0.05)

    @Test("Opening, switching and closing keep the board, and make each task once")
    func theBoardIsNeverRebuilt() async {
        let harness = Harness(motion: Self.quick)
        await harness.settle()
        #expect(harness.hit(900) == "board")
        harness.level.opened = "bil-3"
        await harness.settle(10)
        #expect(harness.hit(100) == "board")
        #expect(harness.hit(900) == "bil-3")
        // Glancing: the next one in place, the board untouched.
        harness.level.opened = "bil-7"
        await harness.settle(10)
        #expect(harness.hit(900) == "bil-7")
        #expect(harness.hit(100) == "board")
        harness.level.opened = nil
        await harness.settle(10)
        #expect(harness.hit(900) == "board")
        harness.close()
        #expect(harness.tally.made["board"] == 1, "the board was rebuilt")
        #expect(harness.tally.gone["board"] == nil, "the board was taken down")
        #expect(harness.tally.made["rail"] == 1)
        #expect(harness.tally.made["bil-3"] == 1 && harness.tally.made["bil-7"] == 1)
        // Let go of once each: switched away from, and closed and settled.
        #expect(harness.tally.gone["bil-3"] == 1 && harness.tally.gone["bil-7"] == 1)
    }

    /// Mid-flight, on a spring slowed to three seconds: a close is a close
    /// at once, the board takes the click where the task still is on
    /// screen, and opening again picks the motion up rather than waiting.
    @Test("Nothing waits on the motion, and rapid toggles end where they're last sent")
    func nothingWaitsOnTheMotion() async {
        let harness = Harness(motion: Self.slow)
        await harness.settle()
        harness.level.opened = "bil-3"
        await harness.settle(2)
        // Barely moving, and already open: the board list beside it is
        // clickable where it will be.
        #expect(harness.hit(100) == "board")
        // Still off to the trailing side, sliding in: not already where the
        // motion ends, fading in there (frames, ov-85).
        #expect(harness.hit(900) != "bil-3", "it appeared where the motion ends")
        try? await Task.sleep(for: .seconds(3.5))
        await harness.settle()
        #expect(harness.hit(900) == "bil-3")
        // Closed: still on screen, sliding, and it takes no click.
        harness.level.opened = nil
        await harness.settle(2)
        #expect(harness.hit(900) != "bil-3", "a closing task took the click")
        // Five more inside a tenth of a second.
        for next in ["bil-7", nil, "bil-9", nil, "bil-11"] as [String?] {
            harness.level.opened = next
            await harness.settle(1)
        }
        try? await Task.sleep(for: .seconds(3.5))
        await harness.settle()
        #expect(harness.hit(900) == "bil-11")
        #expect(harness.hit(100) == "board")
        harness.close()
        #expect(harness.tally.made["board"] == 1)
        #expect(harness.tally.gone["board"] == nil)
    }
    /// Closed and, mid-flight, opened again: the same view slides back,
    /// never taken down and made again. (Fails when what's drawn follows
    /// what's open instead of outliving it: `show` setting `drawn = item`
    /// for nil too.)
    @Test("Reopened mid-close, the same view comes back")
    func reopenedMidCloseIsTheSameView() async {
        let harness = Harness(motion: Self.slow)
        await harness.settle()
        harness.level.opened = "bil-3"
        try? await Task.sleep(for: .seconds(3.5))
        await harness.settle()
        harness.level.opened = nil
        // Well into the 3 s close, long past any removal's own fade.
        try? await Task.sleep(for: .milliseconds(600))
        await harness.settle(1)
        harness.level.opened = "bil-3"
        try? await Task.sleep(for: .seconds(3.5))
        await harness.settle()
        let hit = harness.hit(900)
        harness.close()
        #expect(hit == "bil-3", "\(hit)")
        #expect(harness.tally.made["bil-3"] == 1, "bil-3 was made \(harness.tally.made["bil-3"] ?? 0) times")
        #expect(harness.tally.gone["bil-3"] == nil, "bil-3 was taken down mid-close")
    }

    /// Ten quick steps through the list, as a held arrow takes them: every
    /// one is drawn at once, but only the last settles, so one terminal is
    /// mounted and one record read for the whole walk. (Fails with
    /// `settle` setting `settled` at once on a switch.)
    @Test("Ten quick steps mount one terminal and read one record")
    func tenQuickStepsSettleOnce() async {
        let harness = Harness(motion: Self.quick)
        await harness.settle()
        harness.level.opened = "t0"
        await harness.settle(10)
        let start = harness.tally.fetches.values.reduce(0, +)
        for index in 1...10 {
            harness.level.opened = "t\(index)"
            await harness.settle(1)
        }
        // Drawn at once, header and all: the last one is up already.
        let drawnAtOnce = harness.tally.made["t10"] == 1
        try? await Task.sleep(for: .milliseconds(400))
        await harness.settle()
        let mounted = (1...10).map { harness.tally.made["work-t\($0)"] ?? 0 }
        let fetched = harness.tally.fetches.values.reduce(0, +) - start
        harness.close()
        #expect(drawnAtOnce)
        #expect(mounted == Array(repeating: 0, count: 9) + [1], "mounted \(mounted)")
        #expect(fetched == 1, "read \(fetched) records")
    }
}

