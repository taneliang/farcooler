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

/// Opening a task in the main area, glancing through others and closing it
/// (ov-85, ov-89): neither the board nor the orchestrator is ever taken down
/// or rebuilt, each task's view is made once while it's open, and what takes
/// a click follows the window's state at once, never the motion.
@MainActor
@Suite(.serialized)
struct WorkspaceMotionTests {
    @MainActor
    final class Level: ObservableObject {
        @Published var opened: String?
        @Published var boardOver = false
    }

    /// A workspace 1032 pt wide: the board at 732–1032 in every state
    /// (ov-89). With nothing open, the orchestrator fills 0–731; with a task
    /// open, the rail is at 0–28 and the task at 29–731.
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
            let width: CGFloat
            var body: some View {
                WorkspaceView(
                    opened: level.opened, hasConversation: true, cell: WorkspaceColumns.defaultCell,
                    focused: false, peek: false, boardOver: level.boardOver, boardWidth: .constant(300),
                    conversation: { MotionMarkerView(name: "conversation", tally: tally) },
                    rail: { MotionMarkerView(name: "rail", tally: tally) },
                    board: { MotionMarkerView(name: "board", tally: tally) },
                    strip: { MotionMarkerView(name: "strip", tally: tally) },
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
                    }, onDismissBoard: { level.boardOver = false }, motion: motion)
                .frame(width: width, height: 400)
            }
        }

        init(motion: Animation, width: CGFloat = 1032) {
            MotionTally.current = tally
            host = NSHostingView(rootView: Hosted(level: level, tally: tally, motion: motion, width: width))
            window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: width, height: 400), styleMask: [.borderless],
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
        #expect(harness.hit(400) == "conversation")
        #expect(harness.hit(10) == "conversation")
        #expect(harness.hit(900) == "board")
        harness.level.opened = "bil-3"
        await harness.settle(10)
        #expect(harness.hit(10) == "rail")
        #expect(harness.hit(400) == "bil-3")
        #expect(harness.hit(900) == "board")
        // Glancing: the next one in place, the board untouched.
        harness.level.opened = "bil-7"
        await harness.settle(10)
        #expect(harness.hit(400) == "bil-7")
        #expect(harness.hit(900) == "board")
        harness.level.opened = nil
        await harness.settle(10)
        #expect(harness.hit(400) == "conversation")
        #expect(harness.hit(900) == "board")
        harness.close()
        #expect(harness.tally.made["board"] == 1, "the board was rebuilt")
        #expect(harness.tally.gone["board"] == nil, "the board was taken down")
        // The orchestrator's view is moved, never made again.
        #expect(harness.tally.made["conversation"] == 1, "the orchestrator was rebuilt")
        #expect(harness.tally.gone["conversation"] == nil, "the orchestrator was taken down")
        #expect(harness.tally.made["rail"] == 1)
        #expect(harness.tally.made["bil-3"] == 1 && harness.tally.made["bil-7"] == 1)
        // Let go of once each: switched away from, and closed and settled.
        #expect(harness.tally.gone["bil-3"] == 1 && harness.tally.gone["bil-7"] == 1)
    }

    /// Mid-flight, on a spring slowed to three seconds: an open is an open
    /// at once, the task taking the click where the orchestrator is still
    /// sliding away; a close is a close at once, the orchestrator taking it
    /// back; and opening again picks the motion up rather than waiting.
    @Test("Nothing waits on the motion, and rapid toggles end where they're last sent")
    func nothingWaitsOnTheMotion() async {
        let harness = Harness(motion: Self.slow)
        await harness.settle()
        harness.level.opened = "bil-3"
        await harness.settle(2)
        // Barely moving, and already open: the orchestrator, still over the
        // task, takes no click, and the board stays where it was.
        #expect(harness.hit(400) == "bil-3", "the orchestrator sliding away took the click")
        #expect(harness.hit(900) == "board")
        try? await Task.sleep(for: .seconds(3.5))
        await harness.settle()
        #expect(harness.hit(400) == "bil-3")
        #expect(harness.hit(10) == "rail")
        // Closed: the task is still on screen, the orchestrator sliding back
        // over it, and it takes no click.
        harness.level.opened = nil
        await harness.settle(2)
        #expect(harness.hit(400) != "bil-3", "a closing task took the click")
        #expect(harness.hit(900) == "board")
        // Five more inside a tenth of a second.
        for next in ["bil-7", nil, "bil-9", nil, "bil-11"] as [String?] {
            harness.level.opened = next
            await harness.settle(1)
        }
        try? await Task.sleep(for: .seconds(3.5))
        await harness.settle()
        #expect(harness.hit(400) == "bil-11")
        #expect(harness.hit(900) == "board")
        harness.close()
        #expect(harness.tally.made["board"] == 1)
        #expect(harness.tally.gone["board"] == nil)
        #expect(harness.tally.made["conversation"] == 1)
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
        let hit = harness.hit(400)
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

    /// A detail 700 pt wide, under the 779 that keeps the sidebar: the
    /// board is a 28 pt strip at 671–699 in every state, the orchestrator
    /// fills 0–670, and the strip pops the board open over the main area at
    /// 371–670, where a click outside puts it away. The board is the same
    /// view throughout (ov-89).
    @Test("At a narrow width the board is a strip that pops open over the main area")
    func aNarrowBoardIsAStrip() async {
        let harness = Harness(motion: Self.quick, width: 700)
        await harness.settle()
        #expect(harness.hit(690) == "strip")
        #expect(harness.hit(500) == "conversation")
        harness.level.opened = "bil-3"
        await harness.settle(10)
        #expect(harness.hit(690) == "strip")
        #expect(harness.hit(500) == "bil-3")
        harness.level.boardOver = true
        await harness.settle(10)
        #expect(harness.hit(500) == "board")
        #expect(harness.hit(690) == "strip")
        // Outside it, the dimming takes the click and puts it away.
        let outside = harness.host.hitTest(NSPoint(x: 200, y: 200))
        #expect(!(outside is MotionMarker), "a click outside the board reached \(String(describing: outside))")
        harness.level.boardOver = false
        await harness.settle(10)
        #expect(harness.hit(500) == "bil-3")
        harness.level.opened = nil
        await harness.settle(10)
        #expect(harness.hit(500) == "conversation")
        harness.close()
        #expect(harness.tally.made["board"] == 1, "the board was rebuilt")
        #expect(harness.tally.made["conversation"] == 1, "the orchestrator was rebuilt")
    }

    /// With nothing open, the orchestrator runs right up to the board, with
    /// no band of canvas before it; beside a task, it's the rail's width
    /// narrower. Either way the width its terminal reports to tmux, its own
    /// less `viewportSlack`, is the same (ov-89 review). (Fails with the
    /// panel at one width and a canvas band after it, as it was, or with the
    /// slack not set.)
    @Test("The orchestrator reaches the board and reports one width")
    func theOrchestratorReachesTheBoard() async {
        final class Seen { var drawn: [CGFloat] = []; var reported: [CGFloat] = [] }
        let seen = Seen()
        struct Probe: View {
            let seen: Seen
            @Environment(\.viewportSlack) private var slack
            var body: some View {
                GeometryReader { proxy in
                    Color.clear.onAppear { record(proxy.size.width) }
                        .onChange(of: proxy.size.width) { _, width in record(width) }
                        .onChange(of: slack) { _, _ in record(proxy.size.width) }
                }
            }
            // To the point: layout lands a hair off whole points.
            func record(_ width: CGFloat) {
                seen.drawn.append(width.rounded())
                seen.reported.append((width - slack).rounded())
            }
        }
        let level = Level()
        struct Hosted: View {
            @ObservedObject var level: Level
            let seen: Seen
            var body: some View {
                WorkspaceView(
                    opened: level.opened, hasConversation: true, cell: WorkspaceColumns.defaultCell,
                    focused: false, peek: level.boardOver, boardWidth: .constant(300),
                    conversation: { Probe(seen: seen) }, rail: { Color.clear }, board: { Color.clear },
                    strip: { Color.clear }, breadcrumb: { _ in Color.clear }, detail: { _, _ in Color.clear },
                    motion: .linear(duration: 0.02))
                .frame(width: 1032, height: 400)
            }
        }
        let host = NSHostingView(rootView: Hosted(level: level, seen: seen))
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1032, height: 400), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        func settle() async {
            for _ in 0..<6 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }
        await settle()
        let filling = seen.drawn.last
        level.opened = "bil-3"
        await settle()
        let railed = seen.drawn.last
        // Popped open (`peek`, here driven by `boardOver`'s flag).
        level.boardOver = true
        await settle()
        level.opened = nil
        level.boardOver = false
        await settle()
        window.close()
        // The main area is 1032 − 301 = 731 wide, all of it the orchestrator's.
        let full: CGFloat = 731
        let past: CGFloat = 702
        #expect(filling == full, "\(seen.drawn)")
        #expect(railed == past, "\(seen.drawn)")
        #expect(Set(seen.reported) == [past], "reported \(seen.reported)")
    }
}
