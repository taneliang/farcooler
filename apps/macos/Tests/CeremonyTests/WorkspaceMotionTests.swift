import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A view the hosting view's hit test can name, counting how often it's
/// made and taken down.
final class Named: NSView {
    var name = ""
}

@MainActor
final class Tally {
    var made: [String: Int] = [:]
    var gone: [String: Int] = [:]
}

struct NamedView: NSViewRepresentable {
    let name: String
    let tally: Tally
    func makeNSView(context: Context) -> Named {
        tally.made[name, default: 0] += 1
        let view = Named()
        view.name = name
        return view
    }
    func updateNSView(_ view: Named, context: Context) {}
    static func dismantleNSView(_ view: Named, coordinator: ()) {
        MainActor.assumeIsolated { Tally.current?.gone[view.name, default: 0] += 1 }
    }
}

extension Tally {
    /// The tally the views of the test running now report to: dismantling
    /// is static, so it can't reach an instance any other way.
    @MainActor static var current: Tally?
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
        let tally = Tally()
        let host: NSHostingView<Hosted>
        let window: NSWindow

        struct Hosted: View {
            @ObservedObject var level: Level
            let tally: Tally
            let motion: Animation
            var body: some View {
                WorkspaceView(
                    opened: level.opened, hasConversation: true, cell: WorkspaceColumns.defaultCell,
                    focused: false, peek: false, listWidth: .constant(300),
                    conversation: { Color.clear }, rail: { NamedView(name: "rail", tally: tally) },
                    board: { NamedView(name: "board", tally: tally) },
                    breadcrumb: { _ in Color.clear.frame(height: 30) },
                    detail: { item in NamedView(name: item, tally: tally) }, motion: motion)
                .frame(width: 1032, height: 400)
            }
        }

        init(motion: Animation) {
            Tally.current = tally
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

        func hit(_ x: CGFloat) -> String {
            (host.hitTest(NSPoint(x: x, y: 200)) as? Named)?.name ?? "none"
        }

        func close() {
            window.close()
            Tally.current = nil
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
        #expect(harness.tally.made["bil-3"] == 1, "bil-3 was rebuilt by its own close")
    }
}
